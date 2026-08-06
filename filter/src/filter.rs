//! The DirectShow filter object.

use std::sync::Arc;
use std::sync::Mutex;

use windows::core::*;
use windows::Win32::Media::IReferenceClock;
use windows::Win32::Media::DirectShow::*;
use windows::Win32::System::Com::CoTaskMemFree;

use crate::pins::{InputPin, OutputPin};
use crate::state::Shared;

const FILTER_NAME: &str = "FFmpeg Video Encoder (H.264/HEVC/AV1)";

fn fill_ach_name(dst: &mut [u16; 128], name: &str) {
    for (i, ch) in name.encode_utf16().take(127).enumerate() {
        dst[i] = ch;
    }
    dst[name.encode_utf16().take(127).count()] = 0;
}

#[implement(IBaseFilter)]
pub struct Filter {
    shared: Arc<Shared>,
    graph: Mutex<Option<IFilterGraph>>,
}

impl Filter {
    pub fn new(shared: Arc<Shared>) -> Self {
        Filter {
            shared,
            graph: Mutex::new(None),
        }
    }
}

pub fn make_pins(shared: Arc<Shared>) -> (IPin, IPin) {
    let input: IPin = InputPin::new(shared.clone()).into();
    let output: IPin = OutputPin::new(shared.clone()).into();
    (input, output)
}

impl Drop for Filter {
    fn drop(&mut self) {
        if let Some(mut e) = self.shared.encoder.lock().unwrap().take() {
            e.stop();
        }
    }
}

impl windows::Win32::System::Com::IPersist_Impl for Filter_Impl {
    fn GetClassID(&self) -> Result<GUID> {
        Ok(crate::CLSID_FFmpegEncoder)
    }
}

impl IMediaFilter_Impl for Filter_Impl {
    fn Stop(&self) -> Result<()> {
        if let Some(mut e) = self.shared.encoder.lock().unwrap().take() {
            e.stop();
        }
        // Post-render cleanup: merge the AVI audio track into the MP4/MKV,
        // then remove the MMD-generated .avi so only one file remains.
        {
            let cfg = crate::config::load();
            let cfg = crate::config::Config {
                ffmpeg_path: crate::config::resolve_ffmpeg_path(&cfg),
                ..cfg
            };
            let (completed, avi_path, extra_path) = {
                let core = self.shared.core.lock().unwrap();
                (core.completed, core.avi_path.clone(), core.extra_path.clone())
            };
            if completed && !avi_path.is_empty() && !extra_path.is_empty() {
                let extra_ok = std::path::Path::new(&extra_path)
                    .metadata()
                    .map(|m| m.len() > 0)
                    .unwrap_or(false);
                if extra_ok && cfg.merge_audio {
                    crate::state::debug_log(
                        "Filter::Stop merging AVI audio into extra file",
                    );
                    let merged = merge_audio(&cfg, &avi_path, &extra_path);
                    crate::state::debug_log(&format!(
                        "Filter::Stop merge_audio -> {}",
                        merged
                    ));
                    if !merged {
                        crate::state::debug_log(
                            "Filter::Stop keeping AVI: audio merge failed",
                        );
                        return Ok(());
                    }
                }
                if cfg.delete_avi && extra_ok {
                    match std::fs::remove_file(&avi_path) {
                        Ok(()) => crate::state::debug_log(&format!(
                            "Filter::Stop removed AVI after successful render: {}",
                            avi_path
                        )),
                        Err(e) => crate::state::debug_log(&format!(
                            "Filter::Stop could not remove AVI {}: {}",
                            avi_path, e
                        )),
                    }
                } else {
                    crate::state::debug_log(
                        "Filter::Stop keeping AVI (delete disabled or extra missing)",
                    );
                }
            }
        }
        {
            let out = self.shared.output.lock().unwrap();
            if let Some(a) = out.allocator.as_ref() {
                unsafe {
                    let _ = a.Decommit();
                }
            }
        }
        let mut core = self.shared.core.lock().unwrap();
        core.started = false;
        core.ts_queue.clear();
        core.first_pkt = true;
        core.state = State_Stopped;
        Ok(())
    }

    fn Pause(&self) -> Result<()> {
        self.shared.core.lock().unwrap().state = State_Paused;
        Ok(())
    }

    fn Run(&self, _tstart: i64) -> Result<()> {
        crate::state::debug_log("Filter::Run enter");
        let mut cfg = crate::config::load();
        crate::state::set_debug(cfg.debug);
        cfg.ffmpeg_path = crate::config::resolve_ffmpeg_path(&cfg);
        if !cfg.container.is_empty() && cfg.container_path.trim().is_empty() {
            if let Some(graph) = self.graph.lock().unwrap().clone() {
                if let Some(avi) = find_sink_path(&graph) {
                    cfg.container_path = replace_ext(&avi, &cfg.container);
                    self.shared.core.lock().unwrap().avi_path = avi.clone();
                    self.shared.core.lock().unwrap().extra_path =
                        cfg.container_path.clone();
                    crate::state::debug_log(&format!(
                        "Filter::Run container path auto -> {}",
                        cfg.container_path
                    ));
                }
            }
        }
        let (fourcc, mux) = crate::encoder::pick_fourcc_mux(&cfg.codec);
        {
            let mut core = self.shared.core.lock().unwrap();
            core.fourcc = fourcc;
            core.out_mux = mux.to_string();
            core.codec = cfg.codec.clone();
            core.started = true;
            core.first_pkt = true;
            core.ts_queue.clear();
            core.state = State_Running;
        }

        let fmt = self.shared.core.lock().unwrap().fmt.clone();
        let Some(fmt) = fmt else {
            crate::state::debug_log("Filter::Run no input fmt");
            return Ok(());
        };

        let mut enc_guard = self.shared.encoder.lock().unwrap();
        if enc_guard.is_none() {
            let is_gpu = cfg.codec.contains("nvenc")
                || cfg.codec.contains("amf")
                || cfg.codec.contains("qsv");
            let mut effective = cfg.clone();
            if is_gpu && !crate::encoder::codec_supported(&cfg) {
                effective = crate::encoder::cpu_fallback(&cfg);
                crate::state::debug_log(&format!(
                    "Filter::Run GPU encoder unavailable, fell back to CPU codec {}",
                    effective.codec
                ));
            }
            match crate::encoder::Encoder::start(&effective, &fmt) {
                Ok(e) => {
                    crate::state::debug_log(&format!(
                        "Filter::Run encoder started codec={} fps={}",
                        effective.codec, 10_000_000.0 / fmt.frame_dur as f64
                    ));
                    self.shared.core.lock().unwrap().codec = effective.codec.clone();
                    *enc_guard = Some(e);
                }
                Err(first_err) => {
                    if is_gpu {
                        let cfg2 = crate::encoder::cpu_fallback(&cfg);
                        match crate::encoder::Encoder::start(&cfg2, &fmt) {
                            Ok(e) => {
                                crate::state::debug_log(&format!(
                                    "Filter::Run GPU encoder start failed ({}); fell back to CPU codec {}",
                                    first_err,
                                    cfg2.codec
                                ));
                                self.shared.core.lock().unwrap().codec = cfg2.codec.clone();
                                *enc_guard = Some(e);
                            }
                            Err(e2) => {
                                self.shared.core.lock().unwrap().started = false;
                                crate::state::debug_log(&format!(
                                    "Filter::Run Encoder::start failed (nvenc: {}, cpu: {})",
                                    first_err, e2
                                ));
                                return Err(e2.into());
                            }
                        }
                    } else {
                        self.shared.core.lock().unwrap().started = false;
                        crate::state::debug_log(&format!(
                            "Filter::Run Encoder::start failed: {}",
                            first_err
                        ));
                        return Err(first_err.into());
                    }
                }
            }
        }
        {
            let out = self.shared.output.lock().unwrap();
            if let Some(a) = out.allocator.as_ref() {
                unsafe {
                    let _ = a.Commit();
                }
            }
        }
        crate::state::debug_log("Filter::Run OK");
        Ok(())
    }

    fn GetState(&self, _dwmillisecstimeout: u32) -> Result<FILTER_STATE> {
        Ok(self.shared.core.lock().unwrap().state)
    }

    fn SetSyncSource(&self, pclock: Ref<'_, IReferenceClock>) -> Result<()> {
        *self.shared.clock.lock().unwrap() = pclock.cloned();
        Ok(())
    }

    fn GetSyncSource(&self) -> Result<IReferenceClock> {
        match self.shared.clock.lock().unwrap().clone() {
            Some(c) => Ok(c),
            None => Ok(unsafe { IReferenceClock::from_raw(std::ptr::null_mut()) }),
        }
    }
}

impl IBaseFilter_Impl for Filter_Impl {
    fn EnumPins(&self) -> Result<IEnumPins> {
        let input = self.shared.input_pin.lock().unwrap().clone();
        let output = self.shared.output_pin.lock().unwrap().clone();
        let mut v = Vec::new();
        if let Some(i) = input {
            v.push(i);
        }
        if let Some(o) = output {
            v.push(o);
        }
        Ok(crate::enums::EnumPins::new(v).into())
    }

    fn FindPin(&self, id: &PCWSTR) -> Result<IPin> {
        let mut s = String::new();
        let mut i = 0usize;
        unsafe {
            while *id.0.add(i) != 0 {
                s.push(char::from_u32(*id.0.add(i) as u32).unwrap_or('?'));
                i += 1;
            }
        }
        if s.eq_ignore_ascii_case("FFmpegIn") || s.eq_ignore_ascii_case("Input") {
            self.shared
                .input_pin
                .lock()
                .unwrap()
                .clone()
                .ok_or(crate::state::err(VFW_E_NOT_FOUND))
        } else if s.eq_ignore_ascii_case("FFmpegOut") || s.eq_ignore_ascii_case("Output") {
            self.shared
                .output_pin
                .lock()
                .unwrap()
                .clone()
                .ok_or(crate::state::err(VFW_E_NOT_FOUND))
        } else {
            Err(crate::state::err(VFW_E_NOT_FOUND))
        }
    }

    fn QueryFilterInfo(&self, pinfo: *mut FILTER_INFO) -> Result<()> {
        let mut info = FILTER_INFO::default();
        fill_ach_name(&mut info.achName, FILTER_NAME);
        info.pGraph = core::mem::ManuallyDrop::new(self.graph.lock().unwrap().clone());
        unsafe {
            pinfo.write(info);
        }
        Ok(())
    }

    fn JoinFilterGraph(&self, pgraph: Ref<'_, IFilterGraph>, _pname: &PCWSTR) -> Result<()> {
        if pgraph.is_null() {
            *self.graph.lock().unwrap() = None;
        } else {
            *self.graph.lock().unwrap() = pgraph.cloned();
        }
        Ok(())
    }

    fn QueryVendorInfo(&self) -> Result<PWSTR> {
        Err(crate::state::err(crate::state::E_NOTIMPL_HR))
    }
}

/// Find the output file name configured on the graph's File Writer.
fn find_sink_path(graph: &IFilterGraph) -> Option<String> {
    let e = unsafe { graph.EnumFilters() }.ok()?;
    loop {
        let mut arr = [None; 1];
        let hr = unsafe { e.Next(&mut arr, None) };
        if hr.0 != 0 {
            break;
        }
        let Some(f) = arr[0].take() else {
            break;
        };
        if let Ok(sink) = f.cast::<IFileSinkFilter>() {
            let mut p = PWSTR::null();
            if unsafe { sink.GetCurFile(&mut p, std::ptr::null_mut()) }.is_ok() && !p.is_null() {
                let mut s = String::new();
                let mut i = 0usize;
                unsafe {
                    while *p.0.add(i) != 0 {
                        s.push(char::from_u32(*p.0.add(i) as u32).unwrap_or('?'));
                        i += 1;
                    }
                }
                unsafe {
                    CoTaskMemFree(Some(p.0 as *const _));
                }
                return Some(s);
            }
        }
    }
    None
}

/// Replace the extension of an AVI path with the container's extension.
fn replace_ext(avi: &str, container: &str) -> String {
    let ext = match container.to_ascii_lowercase().as_str() {
        "mp4" | "m4v" => "mp4",
        "mkv" | "matroska" => "mkv",
        "mov" => "mov",
        "webm" => "webm",
        "ts" | "mpegts" => "ts",
        _ => "mp4",
    };
    let mut p = std::path::PathBuf::from(avi);
    p.set_extension(ext);
    p.to_string_lossy().to_string()
}

/// Copy the audio track from the MMD AVI into the extra container using the
/// bundled ffmpeg. The video stream is copied losslessly.
fn merge_audio(cfg: &crate::config::Config, avi_path: &str, extra_path: &str) -> bool {
    use std::process::{Command, Stdio};
    #[cfg(windows)]
    use std::os::windows::process::CommandExt;
    let ext = std::path::Path::new(extra_path)
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_else(|| "mp4".to_string());
    let tmp = format!("{}.merge.{}", extra_path, ext);
    let mut cmd = Command::new(&cfg.ffmpeg_path);
    cmd.args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            extra_path,
            "-i",
            avi_path,
            "-map",
            "0:v:0",
            "-map",
            "1:a:0?",
            "-c:v",
            "copy",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-shortest",
            &tmp,
        ])
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let out = cmd.output();
    let ok = match out {
        Ok(o) => {
            if !o.status.success() {
                crate::state::debug_log(&format!(
                    "merge_audio ffmpeg failed: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                ));
            }
            o.status.success()
        }
        Err(e) => {
            crate::state::debug_log(&format!("merge_audio spawn failed: {}", e));
            false
        }
    };
    if !ok {
        let _ = std::fs::remove_file(&tmp);
        return false;
    }
    let tmp_ok = std::path::Path::new(&tmp)
        .metadata()
        .map(|m| m.len() > 0)
        .unwrap_or(false);
    if !tmp_ok {
        return false;
    }
    // Replace the original extra file with the merged one.
    let _ = std::fs::remove_file(extra_path);
    if std::fs::rename(&tmp, extra_path).is_err() {
        // Keep the merged file under its temp name rather than losing it.
        crate::state::debug_log(&format!("merge_audio rename failed, keeping {}", tmp));
        return false;
    }
    true
}
