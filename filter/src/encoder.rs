//! ffmpeg child process + Annex B packet splitting.

use std::collections::VecDeque;
use std::io::{Read, Write};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crate::config::Config;
use crate::mediatype::mmio_fourcc;
use crate::state::FormatInfo;

#[derive(Clone, Copy, PartialEq)]
pub enum CodecKind {
    H264,
    H265,
    Other,
}

#[derive(Clone, Copy, PartialEq)]
pub enum AlphaMode {
    WebmVp9,
    WebmAv1,
    MovProres,
    MkvFfv1,
}

pub fn alpha_mode_of(alpha_format: &str) -> Option<AlphaMode> {
    match alpha_format.to_ascii_lowercase().as_str() {
        "webm_vp9" | "vp9" => Some(AlphaMode::WebmVp9),
        "webm_av1" | "av1" => Some(AlphaMode::WebmAv1),
        "mov_prores" | "prores" => Some(AlphaMode::MovProres),
        "mkv_ffv1" | "ffv1" => Some(AlphaMode::MkvFfv1),
        _ => None,
    }
}

pub fn alpha_mode(cfg: &Config) -> Option<AlphaMode> {
    alpha_mode_of(&cfg.alpha_format)
}

pub fn pick_fourcc_mux(codec: &str, alpha_format: &str) -> (u32, &'static str) {
    // Transparent output always uses an H.264 elementary stream on the
    // DirectShow/AVI side (the temporary AVI is deleted after rendering);
    // the real transparent file is written in parallel by the same ffmpeg.
    if alpha_mode_of(alpha_format).is_some() {
        return (mmio_fourcc(b'H', b'2', b'6', b'4'), "h264");
    }
    if codec.contains("264") || codec.contains("h264") || codec.contains("avc") {
        (mmio_fourcc(b'H', b'2', b'6', b'4'), "h264")
    } else if codec.contains("265") || codec.contains("hevc") || codec.contains("h265") {
        (mmio_fourcc(b'H', b'E', b'V', b'C'), "hevc")
    } else if codec.contains("av1") {
        (mmio_fourcc(b'a', b'v', b'0', b'1'), "av1")
    } else if codec.contains("ffv1") {
        (mmio_fourcc(b'F', b'F', b'V', b'1'), "ffv1")
    } else if codec.contains("utvideo") {
        (mmio_fourcc(b'U', b'L', b'R', b'G'), "utvideo")
    } else {
        (mmio_fourcc(b'H', b'2', b'6', b'4'), "h264")
    }
}

/// Quick check that the configured encoder can actually be opened by this
/// ffmpeg build + driver. NVENC fails fast on a mismatched driver; ffmpeg
/// itself would otherwise only fail after the first frame arrives, which
/// would hang our write side.
pub fn codec_supported(cfg: &Config) -> bool {
    probe_codec(cfg)
}

fn probe_codec(cfg: &Config) -> bool {
    let mut args: Vec<String> = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "warning".into(),
    ];
    if cfg.codec.contains("qsv") && alpha_mode(cfg).is_none() {
        args.push("-init_hw_device".into());
        args.push("qsv=hw".into());
        args.push("-filter_hw_device".into());
        args.push("hw".into());
    }
    args.extend([
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        "color=black:s=160x120:r=30:d=1".into(),
        "-frames:v".into(),
        "1".into(),
        "-map".into(),
        "0:v".into(),
    ]);
    if let Some(mode) = alpha_mode(cfg) {
        alpha_output_args(mode, cfg, &mut args);
    } else {
        let probe_fmt = FormatInfo {
            width: 160,
            height: 120,
            bpp: 24,
            bottom_up: false,
            pix_fmt: "bgr24".to_string(),
            frame_dur: 333_333,
        };
        let mut codec_args: Vec<String> = Vec::new();
        build_codec_args(cfg, &probe_fmt, &mut codec_args);
        args.extend(codec_args);
    }
    args.push("-f".into());
    args.push("null".into());
    args.push("-".into());
    run_probe(&cfg.ffmpeg_path, &args, &cfg.codec)
}

/// Spawn ffmpeg for a probe and wait up to 20 s. Some broken GPU drivers
/// can hang ffmpeg forever; never block MMD on that.
fn run_probe(ffmpeg_path: &str, args: &[String], label: &str) -> bool {
    let mut cmd = Command::new(ffmpeg_path);
    cmd.args(args)
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            crate::state::debug_log(&format!(
                "codec probe spawn failed ({}): {}",
                label, e
            ));
            return false;
        }
    };
    let stderr = match child.stderr.take() {
        Some(s) => s,
        None => return false,
    };
    let err_text: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let et2 = err_text.clone();
    std::thread::spawn(move || {
        let mut r = stderr;
        let mut buf = [0u8; 4096];
        loop {
            match r.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    et2
                        .lock()
                        .unwrap()
                        .push_str(&String::from_utf8_lossy(&buf[..n]));
                }
            }
        }
    });

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let _ = child.wait();
                let text = err_text.lock().unwrap().clone();
                // Some encoders silently fall back to a non-alpha pixel
                // format when they do not support yuva* (e.g. libaom-av1).
                // Treat that as unsupported so we fall back to an opaque
                // CPU container instead of writing a "transparent" file
                // without alpha.
                let alpha_dropped = text.contains("Incompatible pixel format")
                    || text.contains("auto-selecting format");
                if !status.success() || alpha_dropped {
                    crate::state::debug_log(&format!(
                        "codec probe {} failed (alpha_dropped={}): {}",
                        label,
                        alpha_dropped,
                        text.trim()
                    ));
                    return false;
                }
                return true;
            }
            Ok(None) => {}
            Err(_) => return false,
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            crate::state::debug_log(&format!(
                "codec probe {} timed out after 20s",
                label
            ));
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// Resolve "auto*" codec names (written by the config UI's "自动（推荐）"
/// option) to the first encoder this machine actually supports: NVENC ->
/// QSV -> AMF -> CPU.
pub fn resolve_codec(cfg: &Config) -> String {
    let base = cfg.codec.trim().to_ascii_lowercase();
    if !base.starts_with("auto") {
        return cfg.codec.clone();
    }
    let candidates: &[&str] = if base.contains("265") || base.contains("hevc") {
        &["hevc_nvenc", "hevc_qsv", "hevc_amf", "libx265"]
    } else if base.contains("av1") {
        &["av1_nvenc", "av1_qsv", "av1_amf", "libaom-av1"]
    } else {
        &["h264_nvenc", "h264_qsv", "h264_amf", "libx264"]
    };
    for c in candidates {
        let mut probe = cfg.clone();
        probe.codec = c.to_string();
        if probe_codec(&probe) {
            crate::state::debug_log(&format!("auto codec resolved to {}", c));
            return c.to_string();
        }
    }
    crate::state::debug_log("auto codec: no hardware encoder available, using CPU");
    candidates.last().unwrap().to_string()
}

/// Encoder-side options for the transparent (alpha-capable) final file.
/// These codecs are CPU-only in practice (libvpx/libaom/prores/ffv1), so no
/// hardware device setup is needed.
fn alpha_output_args(mode: AlphaMode, cfg: &Config, a: &mut Vec<String>) {
    let (codec, pix_fmt) = match mode {
        AlphaMode::WebmVp9 => ("libvpx-vp9", "yuva420p"),
        AlphaMode::WebmAv1 => ("libaom-av1", "yuva420p"),
        AlphaMode::MovProres => ("prores_ks", "yuva444p10le"),
        AlphaMode::MkvFfv1 => ("ffv1", "yuva444p"),
    };
    a.push("-c:v".into());
    a.push(codec.into());
    a.push("-pix_fmt".into());
    a.push(pix_fmt.into());
    match mode {
        AlphaMode::WebmVp9 => {
            if cfg.bitrate > 0 {
                a.push("-b:v".into());
                a.push(cfg.bitrate.to_string());
            } else {
                a.push("-crf".into());
                a.push(if cfg.crf > 0 { cfg.crf } else { 18 }.to_string());
            }
            a.push("-deadline".into());
            a.push("good".into());
            a.push("-cpu-used".into());
            a.push("2".into());
            a.push("-row-mt".into());
            a.push("1".into());
        }
        AlphaMode::WebmAv1 => {
            if cfg.bitrate > 0 {
                a.push("-b:v".into());
                a.push(cfg.bitrate.to_string());
            } else {
                a.push("-crf".into());
                a.push(if cfg.crf > 0 { cfg.crf } else { 30 }.to_string());
            }
            a.push("-cpu-used".into());
            a.push("6".into());
            a.push("-row-mt".into());
            a.push("1".into());
        }
        AlphaMode::MovProres => {
            a.push("-profile:v".into());
            a.push("4444".into());
            a.push("-vendor".into());
            a.push("apl0".into());
        }
        AlphaMode::MkvFfv1 => {
            a.push("-level".into());
            a.push("3".into());
        }
    }
    for arg in cfg.extra.split_whitespace() {
        a.push(arg.to_string());
    }
}

/// Build the encoder-side options (filters, codec, preset, quality). Shared
/// by the real pipeline and the startup probe so the probe sees exactly the
/// same arguments the encoder will get.
fn build_codec_args(cfg: &Config, fmt: &FormatInfo, a: &mut Vec<String>) {
    let nvenc = cfg.codec.contains("nvenc");
    let qsv = cfg.codec.contains("qsv");
    let amf = cfg.codec.contains("amf");
    let gpu = nvenc || qsv || amf;
    let lossless = cfg.codec.contains("ffv1")
        || cfg.codec.contains("utvideo")
        || cfg.codec.contains("huffyuv")
        || cfg.codec.contains("ffvhuff");

    if fmt.bottom_up {
        a.push("-vf".into());
        a.push(if qsv {
            "vflip,format=nv12,hwupload=extra_hw_frames=64".into()
        } else {
            "vflip".into()
        });
    } else if qsv {
        a.push("-vf".into());
        a.push("format=nv12,hwupload=extra_hw_frames=64".into());
    }
    a.push("-c:v".into());
    a.push(cfg.codec.clone());
    if !cfg.preset.is_empty() && !(nvenc && cfg.preset == "veryfast") {
        if amf {
            a.push("-quality".into());
        } else {
            a.push("-preset".into());
        }
        a.push(cfg.preset.clone());
    }
    let have_bitrate = cfg.bitrate > 0;
    if gpu {
        if have_bitrate {
            // VBR: average bitrate target; ffmpeg picks the encoder's
            // variable-bitrate default when only -b:v is given.
            a.push("-b:v".into());
            a.push(cfg.bitrate.to_string());
        } else if nvenc {
            a.push("-qp".into());
            a.push("23".into());
        } else if qsv {
            a.push("-global_quality".into());
            a.push("23".into());
        } else {
            a.push("-qp_i".into());
            a.push("23".into());
            a.push("-qp_p".into());
            a.push("23".into());
        }
    } else if have_bitrate {
        a.push("-b:v".into());
        a.push(cfg.bitrate.to_string());
    } else if cfg.crf > 0 {
        a.push("-crf".into());
        a.push(cfg.crf.to_string());
    }
    if !lossless && !qsv {
        // QSV's filter chain already outputs NV12; setting -pix_fmt here
        // would conflict with the hardware frames.
        a.push("-pix_fmt".into());
        a.push("yuv420p".into());
    }
    if !lossless
        && !nvenc
        && (cfg.codec.contains("264") || cfg.codec.contains("265"))
    {
        a.push("-bf".into());
        a.push("0".into());
    }
    for arg in cfg.extra.split_whitespace() {
        a.push(arg.to_string());
    }
}

/// CPU fallback config for when a GPU encoder is unavailable.
pub fn cpu_fallback(cfg: &Config) -> Config {
    let mut c = cfg.clone();
    c.codec = if cfg.codec.contains("265") {
        "libx265".to_string()
    } else if cfg.codec.contains("av1") {
        "libaom-av1".to_string()
    } else {
        "libx264".to_string()
    };
    c.preset = "veryfast".to_string();
    c
}

/// ffmpeg container format name for our ini value.
fn container_format(cfg_container: &str) -> &'static str {
    match cfg_container.to_ascii_lowercase().as_str() {
        "mp4" | "m4v" => "mp4",
        "mkv" | "matroska" => "matroska",
        "mov" => "mov",
        "webm" => "webm",
        "ts" | "mpegts" => "mpegts",
        _ => "mp4",
    }
}

/// Escape a path for ffmpeg's tee muxer URL syntax.
fn tee_escape(path: &str) -> String {
    let mut s = path.replace('\\', "/");
    s = s
        .replace('[', "\\[")
        .replace(']', "\\]")
        .replace('|', "\\|")
        .replace(':', "\\:");
    s
}

pub fn build_args(cfg: &Config, fmt: &FormatInfo) -> Vec<String> {
    let (_, mux) = pick_fourcc_mux(&cfg.codec, &cfg.alpha_format);
    let alpha = alpha_mode(cfg);
    let qsv = cfg.codec.contains("qsv") && alpha.is_none();
    let fps = 10_000_000.0 / fmt.frame_dur as f64;

    let mut a: Vec<String> = vec![
        "-y".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-nostdin".into(),
    ];
    if qsv {
        // Make sure QSV really uses the Intel media engine: initialize the
        // device explicitly and upload frames to it before encoding.
        a.push("-init_hw_device".into());
        a.push("qsv=hw".into());
        a.push("-filter_hw_device".into());
        a.push("hw".into());
    }
    a.extend([
        "-f".into(),
        "rawvideo".into(),
        "-pix_fmt".into(),
        fmt.pix_fmt.clone(),
        "-s".into(),
        format!("{}x{}", fmt.width, fmt.height),
        "-r".into(),
        format!("{}", fps),
        "-i".into(),
        "pipe:0".into(),
    ]);
    if let Some(mode) = alpha {
        // Transparent final file (WebM/MOV/MKV with alpha) ...
        if fmt.bottom_up {
            a.push("-vf".into());
            a.push("vflip".into());
        }
        a.push("-map".into());
        a.push("0:v".into());
        alpha_output_args(mode, cfg, &mut a);
        a.push(if cfg.container_path.is_empty() {
            "transparent.out".into()
        } else {
            cfg.container_path.clone()
        });
        // ... plus an H.264 elementary stream on stdout for the DirectShow
        // AVI Mux (the temporary AVI is deleted after rendering).
        if fmt.bottom_up {
            a.push("-vf".into());
            a.push("vflip".into());
        }
        a.push("-map".into());
        a.push("0:v".into());
        a.push("-c:v".into());
        a.push("libx264".into());
        a.push("-preset".into());
        a.push("ultrafast".into());
        a.push("-pix_fmt".into());
        a.push("yuv420p".into());
        a.push("-f".into());
        a.push("h264".into());
        a.push("pipe:1".into());
    } else {
        build_codec_args(cfg, fmt, &mut a);
        a.push("-map".into());
        a.push("0:v".into());
        if !cfg.container.is_empty() && !cfg.container_path.is_empty() {
            // Dual output: the encoded stream goes to the extra container AND
            // to our stdout as an elementary stream (for the AVI Mux).
            let fmt = container_format(&cfg.container);
            let url = format!(
                "[f={}:onfail=ignore]{}|[f={}]pipe\\:1",
                fmt,
                tee_escape(&cfg.container_path),
                mux
            );
            a.push("-f".into());
            a.push("tee".into());
            a.push(url);
        } else {
            a.push("-f".into());
            a.push(mux.to_string());
            a.push("pipe:1".into());
        }
    }
    a
}

pub struct Encoder {
    pub child: Child,
    pub stdin: Option<ChildStdin>,
    pub packets: Arc<Mutex<VecDeque<Vec<u8>>>>,
    pub reader: Option<JoinHandle<()>>,
    pub reader_done: Arc<AtomicBool>,
    pub err_buf: Arc<Mutex<String>>,
}

/// stderr messages that mean the encoder could not be opened (e.g. the
/// ffmpeg build is too new for the installed NVIDIA driver).
const FAIL_KEYWORDS: [&str; 7] = [
    "Driver does not support",
    "Could not open encoder",
    "Error while opening encoder",
    "Error opening output file",
    "Unknown encoder",
    "No NVENC capable",
    "Conversion failed!",
];

impl Encoder {
    pub fn start(cfg: &Config, fmt: &FormatInfo) -> std::io::Result<Self> {
        crate::state::always_log(&format!(
            "Encoder::start args: {:?}",
            build_args(cfg, fmt)
        ));
        let mut cmd = Command::new(&cfg.ffmpeg_path);
        cmd.args(build_args(cfg, fmt))
            .creation_flags(0x08000000) // CREATE_NO_WINDOW
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn()?;

        // Read stderr in the background: ffmpeg often does NOT exit when the
        // encoder fails to open (it keeps waiting on stdin), so we must watch
        // its error output to detect e.g. an NVENC/driver mismatch.
        let stderr = child.stderr.take().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::Other, "no ffmpeg stderr")
        })?;
        let err_buf: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
        let eb2 = err_buf.clone();
        std::thread::spawn(move || {
            let mut r = stderr;
            let mut buf = [0u8; 4096];
            loop {
                match r.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        eb2.lock().unwrap().push_str(&String::from_utf8_lossy(&buf[..n]));
                    }
                }
            }
        });

        // Give ffmpeg a moment to initialize; fail fast on early exit or an
        // encoder-open error in stderr instead of hanging on the pipe.
        for _ in 0..100 {
            match child.try_wait()? {
                Some(status) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let text = err_buf.lock().unwrap().clone();
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!(
                            "ffmpeg exited during startup (code {}){}",
                            status,
                            if text.is_empty() {
                                String::new()
                            } else {
                                format!(": {}", text.trim())
                            }
                        ),
                    ));
                }
                None => {}
            }
            let text = err_buf.lock().unwrap().clone();
            if let Some(kw) = FAIL_KEYWORDS.iter().find(|k| text.contains(**k)) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("ffmpeg encoder open failed (\"{}\"): {}", kw, text.trim()),
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }

        let stdin = child.stdin.take();
        let stdout = child.stdout.take().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::Other, "no ffmpeg stdout")
        })?;

        let packets: Arc<Mutex<VecDeque<Vec<u8>>>> = Arc::new(Mutex::new(VecDeque::new()));
        let (_, mux) = pick_fourcc_mux(&cfg.codec, &cfg.alpha_format);
        let kind = if mux == "h264" {
            CodecKind::H264
        } else if mux == "hevc" {
            CodecKind::H265
        } else {
            CodecKind::Other
        };
        let annexb = Arc::new(Mutex::new(AnnexB::new(kind)));
        let reader_done = Arc::new(AtomicBool::new(false));

        let p2 = packets.clone();
        let a2 = annexb.clone();
        let d2 = reader_done.clone();
        let reader = std::thread::spawn(move || {
            let mut reader = stdout;
            let mut buf = [0u8; 65536];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let mut pk = p2.lock().unwrap();
                        a2.lock().unwrap().feed(&buf[..n], &mut pk);
                    }
                    Err(_) => break,
                }
            }
            let mut pk = p2.lock().unwrap();
            a2.lock().unwrap().finish(&mut pk);
            d2.store(true, Ordering::SeqCst);
        });

        Ok(Encoder {
            child,
            stdin,
            packets,
            reader: Some(reader),
            reader_done,
            err_buf,
        })
    }

    pub fn write_frame(&mut self, data: &[u8]) -> std::io::Result<()> {
        if self.failed() {
            let text = self.err_buf.lock().unwrap().clone();
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!(
                    "ffmpeg encoder failed while running: {}",
                    text.trim()
                ),
            ));
        }
        match self.stdin.as_mut() {
            Some(s) => s.write_all(data),
            None => Ok(()),
        }
    }

    /// True if ffmpeg has reported an encoder error or has already exited.
    pub fn failed(&mut self) -> bool {
        if let Ok(Some(_)) = self.child.try_wait() {
            return true;
        }
        let text = self.err_buf.lock().unwrap();
        FAIL_KEYWORDS.iter().any(|k| text.contains(k))
    }

    /// Close stdin so ffmpeg flushes and emits the final packets.
    pub fn flush(&mut self) {
        self.stdin.take();
    }

    pub fn wait_reader(&mut self, timeout_ms: u64) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        while !self.reader_done.load(Ordering::SeqCst)
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if let Some(h) = self.reader.take() {
            let _ = h.join();
        }
    }

    pub fn stop(&mut self) {
        self.flush();
        let text = self.err_buf.lock().unwrap().clone();
        if !text.trim().is_empty() {
            crate::state::debug_log(&format!("Encoder::stop ffmpeg stderr: {}", text.trim()));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.wait_reader(2000);
    }

    pub fn drain(&self) -> Vec<Vec<u8>> {
        let mut pk = self.packets.lock().unwrap();
        pk.drain(..).collect()
    }
}

// ---------------------------------------------------------------------------
// Annex B (H.264/HEVC) access-unit splitting
// ---------------------------------------------------------------------------

pub struct AnnexB {
    buf: Vec<u8>,
    group: Vec<u8>,
    group_has_vcl: bool,
    kind: CodecKind,
}

impl AnnexB {
    pub fn new(kind: CodecKind) -> Self {
        AnnexB {
            buf: Vec::new(),
            group: Vec::new(),
            group_has_vcl: false,
            kind,
        }
    }

    pub fn feed(&mut self, data: &[u8], out: &mut VecDeque<Vec<u8>>) {
        self.buf.extend_from_slice(data);
        self.parse(out);
    }

    pub fn finish(&mut self, out: &mut VecDeque<Vec<u8>>) {
        self.parse(out);
        // The last NAL has no trailing start code; handle the tail at EOF.
        if let Some((pos, len)) = Self::find_start(&self.buf, 0) {
            let nal_start = pos + len;
            if nal_start < self.buf.len() {
                let mut tail = self.buf[nal_start..].to_vec();
                while tail.last() == Some(&0) {
                    tail.pop();
                }
                if !tail.is_empty() {
                    self.push_nal(&tail, out);
                }
            }
        }
        self.buf.clear();
        if !self.group.is_empty() {
            out.push_back(std::mem::take(&mut self.group));
            self.group_has_vcl = false;
        }
    }

    fn nal_is_vcl(&self, h: u8) -> bool {
        if h & 0x80 != 0 {
            return false;
        }
        match self.kind {
            CodecKind::H264 => {
                let t = h & 0x1F;
                (1..=5).contains(&t)
            }
            CodecKind::H265 => {
                let t = (h >> 1) & 0x3F;
                t <= 31
            }
            CodecKind::Other => true,
        }
    }

    fn find_start(buf: &[u8], from: usize) -> Option<(usize, usize)> {
        let mut i = from;
        while i + 2 < buf.len() {
            if buf[i] == 0 && buf[i + 1] == 0 {
                if buf[i + 2] == 1 {
                    if i > 0 && buf[i - 1] == 0 {
                        return Some((i - 1, 4));
                    }
                    return Some((i, 3));
                }
                if i + 3 < buf.len() && buf[i + 2] == 0 && buf[i + 3] == 1 {
                    return Some((i, 4));
                }
            }
            i += 1;
        }
        None
    }

    fn flush_group(&mut self, out: &mut VecDeque<Vec<u8>>) {
        if !self.group.is_empty() {
            out.push_back(std::mem::take(&mut self.group));
            self.group_has_vcl = false;
        }
    }

    fn push_nal(&mut self, nal: &[u8], out: &mut VecDeque<Vec<u8>>) {
        let vcl = self.nal_is_vcl(nal[0]);
        // Each NAL in an access unit must be separated by a start code.
        self.group.extend_from_slice(&[0, 0, 0, 1]);
        self.group.extend_from_slice(nal);
        if vcl {
            self.group_has_vcl = true;
            self.flush_group(out);
        } else {
            if self.group_has_vcl && !self.group.is_empty() {
                self.flush_group(out);
            }
        }
    }

    fn parse(&mut self, out: &mut VecDeque<Vec<u8>>) {
        let Some((first, mut code_len)) = Self::find_start(&self.buf, 0) else {
            return;
        };
        let mut nal_start = first + code_len;
        let mut keep_from = first; // keep the leading start code in the buffer
        let mut nal = Vec::new();

        loop {
            let Some((next, cl)) = Self::find_start(&self.buf, nal_start) else {
                break;
            };
            code_len = cl;
            if next > nal_start {
                let mut nal_end = next;
                while nal_end > nal_start && self.buf[nal_end - 1] == 0 {
                    nal_end -= 1;
                }
                if nal_end > nal_start {
                    nal.clear();
                    nal.extend_from_slice(&self.buf[nal_start..nal_end]);
                    self.push_nal(&nal, out);
                }
            }
            nal_start = next + code_len;
            keep_from = next;
        }

        // Keep everything from the last complete start code onwards, so a
        // partially received NAL or start code is preserved for the next feed.
        self.buf.drain(..keep_from);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sc4() -> Vec<u8> {
        vec![0, 0, 0, 1]
    }

    #[test]
    fn splits_access_units() {
        let mut ab = AnnexB::new(CodecKind::H264);
        let mut out = VecDeque::new();
        let mut stream = Vec::new();
        // SPS + PPS + IDR (VCL) => one packet
        stream.extend(sc4());
        stream.extend([0x67, 1, 2]); // SPS
        stream.extend(sc4());
        stream.extend([0x68, 3, 4]); // PPS
        stream.extend(sc4());
        stream.extend([0x65, 5, 6]); // IDR slice
        // Next frame: SEI + P slice
        stream.extend(sc4());
        stream.extend([0x06, 7]); // SEI
        stream.extend(sc4());
        stream.extend([0x41, 8, 9]); // P slice

        // Feed in two chunks to exercise buffering.
        let split = 11;
        ab.feed(&stream[..split], &mut out);
        assert_eq!(out.len(), 0, "no complete access unit yet");
        ab.feed(&stream[split..], &mut out);
        ab.finish(&mut out);
        assert_eq!(out.len(), 2, "two access units after finish");

        let first = &out[0];
        assert!(first.windows(2).any(|w| w == [0x67, 1]), "SPS in first packet");
        assert!(first.windows(2).any(|w| w == [0x68, 3]), "PPS in first packet");
        assert!(first.windows(2).any(|w| w == [0x65, 5]), "IDR in first packet");
        assert!(out[1].windows(2).any(|w| w == [0x06, 7]), "SEI in second packet");
        assert!(out[1].windows(2).any(|w| w == [0x41, 8]), "P slice in second packet");
    }

    #[test]
    fn handles_four_byte_and_three_byte_codes() {
        let mut ab = AnnexB::new(CodecKind::H264);
        let mut out = VecDeque::new();
        let mut stream = Vec::new();
        stream.extend(sc4());
        stream.extend([0x65, 1]);
        stream.extend([0, 0, 1]);
        stream.extend([0x41, 2]);
        ab.feed(&stream, &mut out);
        ab.finish(&mut out);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn ignores_empty_tail() {
        let mut ab = AnnexB::new(CodecKind::H264);
        let mut out = VecDeque::new();
        ab.feed(&[0, 0, 0], &mut out);
        ab.finish(&mut out);
        assert!(out.is_empty());
    }
}
