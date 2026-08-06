//! FFmpegEncoder.ini configuration.

use std::fs;
use std::path::{Path, PathBuf};
use windows::core::PCWSTR;
use windows::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleW};

#[derive(Clone)]
pub struct Config {
    pub ffmpeg_path: String,
    pub codec: String,
    pub preset: String,
    pub crf: i32,
    pub bitrate: i32,
    // crf = quality-first, vbr = dynamic bitrate, cbr = constant bitrate.
    pub rate_mode: String,
    pub extra: String,
    // Output media-type tuning (mainly for AVI Mux compatibility).
    pub out_lsample: i32,
    pub out_cbextra: i32,
    pub out_bisizeimage: i32,
    pub out_bitrate: i32,
    pub out_fourcc: String,
    pub out_rcsource_zero: bool,
    pub debug: bool,
    // Extra container output (dual-write): mp4 / mkv / mov / webm.
    // Empty = off. container_path empty = auto (same name as the AVI file).
    pub container: String,
    pub container_path: String,
    // Transparent output mode. Empty = off. When set, the final container
    // carries the alpha channel from the input frames (WebM VP9/AV1,
    // MOV ProRes 4444, MKV FFV1). The intermediate AVI stream stays H.264.
    pub alpha_format: String,
    // Remove the MMD-generated .avi after a successful render when the extra
    // container file exists. AVI is only kept as a fallback on failure/cancel.
    pub delete_avi: bool,
    // Copy the audio track from the MMD .avi into the extra container file.
    pub merge_audio: bool,
}

fn ini_candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    // 1. Next to the filter DLL (the install folder; the config UI writes
    //    the same file there, so uninstall removes everything together).
    if let Some(dir) = dll_dir() {
        v.push(dir.join("FFmpegEncoder.ini"));
    }
    // 2. Next to the host executable (typically the MMD folder).
    if let Ok(mut p) = std::env::current_exe() {
        p.set_file_name("FFmpegEncoder.ini");
        v.push(p);
    }
    v
}

/// Directory of this filter DLL, so the installer's default ini can be found
/// without touching the MMD folder.
fn dll_dir() -> Option<PathBuf> {
    for name in ["FFmpegVideoEncoder.dll", "ffmpeg_encoder.dll"] {
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        if let Ok(h) = unsafe { GetModuleHandleW(PCWSTR(wide.as_ptr())) } {
            let mut buf = [0u16; 1024];
            let n = unsafe { GetModuleFileNameW(Some(h), &mut buf) };
            if n > 0 {
                let path = PathBuf::from(String::from_utf16_lossy(&buf[..n as usize]));
                if let Some(dir) = path.parent() {
                    return Some(dir.to_path_buf());
                }
            }
        }
    }
    None
}

/// Make sure the configured ffmpeg path still exists. If the app was
/// uninstalled/moved, fall back to the filter's own folder or to PATH.
pub fn resolve_ffmpeg_path(cfg: &Config) -> String {
    if cfg.ffmpeg_path == "ffmpeg" || Path::new(&cfg.ffmpeg_path).is_file() {
        return cfg.ffmpeg_path.clone();
    }
    if let Some(dir) = dll_dir() {
        for cand in [dir.join("bin").join("ffmpeg.exe"), dir.join("ffmpeg.exe")] {
            if cand.is_file() {
                return cand.to_string_lossy().to_string();
            }
        }
    }
    "ffmpeg".to_string()
}

pub fn load() -> Config {
    let mut cfg = Config {
        ffmpeg_path: "ffmpeg".to_string(),
        codec: "libx264".to_string(),
        preset: "veryfast".to_string(),
        crf: 18,
        bitrate: 0,
        rate_mode: "crf".to_string(),
        extra: String::new(),
        out_lsample: 0,
        out_cbextra: 0,
        out_bisizeimage: 0,
        out_bitrate: 0,
        out_fourcc: String::new(),
        out_rcsource_zero: false,
        debug: false,
        container: String::new(),
        container_path: String::new(),
        alpha_format: String::new(),
        delete_avi: true,
        merge_audio: true,
    };

    let mut path = None;
    for cand in ini_candidates() {
        if cand.exists() {
            path = Some(cand);
            break;
        }
    }
    let Some(path) = path else {
        return cfg;
    };

    let Ok(text) = fs::read_to_string(&path) else {
        return cfg;
    };

    let mut section = String::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].trim().to_ascii_lowercase();
            continue;
        }
        let Some(eq) = line.find('=') else {
            continue;
        };
        let key = line[..eq].trim().to_ascii_lowercase();
        let value = line[eq + 1..].trim().to_string();
        match section.as_str() {
            "ffmpeg" if key == "path" => cfg.ffmpeg_path = value,
            "video" if key == "codec" => cfg.codec = value,
            "video" if key == "preset" => cfg.preset = value,
            "video" if key == "crf" => cfg.crf = value.parse().unwrap_or(18),
            "video" if key == "bitrate" => cfg.bitrate = value.parse().unwrap_or(0),
            "video" if key == "rate_mode" => cfg.rate_mode = value,
            "video" if key == "extra" => cfg.extra = value,
            "video" if key == "out_lsample" => cfg.out_lsample = value.parse().unwrap_or(0),
            "video" if key == "out_cbextra" => cfg.out_cbextra = value.parse().unwrap_or(0),
            "video" if key == "out_bisizeimage" => cfg.out_bisizeimage = value.parse().unwrap_or(0),
            "video" if key == "out_bitrate" => cfg.out_bitrate = value.parse().unwrap_or(0),
            "video" if key == "out_fourcc" => cfg.out_fourcc = value,
            "video" if key == "out_rcsource_zero" => {
                cfg.out_rcsource_zero = value == "1" || value.eq_ignore_ascii_case("true")
            }
            "ffmpeg" if key == "debug" => {
                cfg.debug = value == "1" || value.eq_ignore_ascii_case("true")
            }
            "video" if key == "container" => cfg.container = value,
            "video" if key == "container_path" => cfg.container_path = value,
            "video" if key == "alpha_format" => cfg.alpha_format = value,
            "video" if key == "delete_avi" => {
                cfg.delete_avi = value == "1" || value.eq_ignore_ascii_case("true")
            }
            "video" if key == "merge_audio" => {
                cfg.merge_audio = value == "1" || value.eq_ignore_ascii_case("true")
            }
            _ => {}
        }
    }
    // Backwards compatibility: an old ini with bitrate set but no rate_mode
    // was a variable (average) bitrate configuration.
    if cfg.rate_mode.is_empty() {
        cfg.rate_mode = if cfg.bitrate > 0 {
            "vbr".to_string()
        } else {
            "crf".to_string()
        };
    }
    cfg
}
