slint::include_modules!();

use std::path::{Path, PathBuf};

#[derive(Clone)]
struct Config {
    ffmpeg_path: String,
    codec: String,
    preset: String,
    crf: i32,
    bitrate: i32,
    rate_mode: String,
    extra: String,
    container: String,
    container_path: String,
    alpha_format: String,
    debug: bool,
    delete_avi: bool,
    merge_audio: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            ffmpeg_path: "ffmpeg".to_string(),
            codec: "auto".to_string(),
            preset: "veryfast".to_string(),
            crf: 18,
            bitrate: 0,
            rate_mode: "crf".to_string(),
            extra: String::new(),
            container: "mp4".to_string(),
            container_path: String::new(),
            alpha_format: String::new(),
            debug: false,
            delete_avi: true,
            merge_audio: true,
        }
    }
}

fn config_dir() -> PathBuf {
    // Config lives next to this program (the install folder), so it is
    // removed together with the app on uninstall.
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn config_file() -> PathBuf {
    config_dir().join("FFmpegEncoder.ini")
}

fn load_config() -> Config {
    let mut cfg = Config::default();
    let Ok(text) = std::fs::read_to_string(config_file()) else {
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
            "ffmpeg" if key == "debug" => {
                cfg.debug = value == "1" || value.eq_ignore_ascii_case("true")
            }
            "video" if key == "codec" => cfg.codec = value.to_ascii_lowercase(),
            "video" if key == "preset" => cfg.preset = value,
            "video" if key == "crf" => cfg.crf = value.parse().unwrap_or(18),
            "video" if key == "bitrate" => cfg.bitrate = value.parse().unwrap_or(0),
            "video" if key == "rate_mode" => cfg.rate_mode = value.to_ascii_lowercase(),
            "video" if key == "extra" => cfg.extra = value,
            "video" if key == "container" => cfg.container = value.to_ascii_lowercase(),
            "video" if key == "container_path" => cfg.container_path = value,
            "video" if key == "alpha_format" => {
                cfg.alpha_format = value.to_ascii_lowercase()
            }
            "video" if key == "delete_avi" => {
                cfg.delete_avi = value == "1" || value.eq_ignore_ascii_case("true")
            }
            "video" if key == "merge_audio" => {
                cfg.merge_audio = value == "1" || value.eq_ignore_ascii_case("true")
            }
            _ => {}
        }
    }
    cfg
}

fn save_config(cfg: &Config) -> std::io::Result<()> {
    std::fs::create_dir_all(config_dir())?;
    let ini = format!(
        "[ffmpeg]\n\
         ; ffmpeg.exe path (auto-filled on save)\n\
         path={}\n\
         ; 1 = write debug log to %TEMP%\\MMDFfmpegEncoder\\ffmpeg_encoder_debug.log\n\
         debug={}\n\
         \n\
         [video]\n\
         codec={}\n\
         preset={}\n\
         crf={}\n\
         bitrate={}\n\
         ; crf = quality-first, vbr = dynamic bitrate\n\
         rate_mode={}\n\
         extra={}\n\
         ; mp4 / mkv / mov / empty=AVI only\n\
         container={}\n\
         container_path={}\n\
         ; transparent output: empty / webm_vp9 / webm_av1 / mov_prores / mkv_ffv1\n\
         alpha_format={}\n\
         delete_avi={}\n\
         merge_audio={}\n",
        cfg.ffmpeg_path,
        if cfg.debug { 1 } else { 0 },
        cfg.codec,
        cfg.preset,
        cfg.crf,
        cfg.bitrate,
        cfg.rate_mode,
        cfg.extra,
        cfg.container,
        cfg.container_path,
        cfg.alpha_format,
        if cfg.delete_avi { 1 } else { 0 },
        if cfg.merge_audio { 1 } else { 0 },
    );
    std::fs::write(config_file(), ini)
}

fn codec_for(vendor: usize, codec: usize) -> &'static str {
    let fmt = match codec {
        0 => "264",
        1 => "265",
        _ => "av1",
    };
    match vendor {
        0 => match fmt {
            "264" => "auto",
            "265" => "auto265",
            _ => "autoav1",
        },
        1 => match fmt {
            "264" => "h264_nvenc",
            "265" => "hevc_nvenc",
            _ => "av1_nvenc",
        },
        2 => match fmt {
            "264" => "h264_qsv",
            "265" => "hevc_qsv",
            _ => "av1_qsv",
        },
        3 => match fmt {
            "264" => "h264_amf",
            "265" => "hevc_amf",
            _ => "av1_amf",
        },
        _ => match fmt {
            "264" => "libx264",
            "265" => "libx265",
            _ => "libaom-av1",
        },
    }
}

fn preset_for(codec: &str) -> &'static str {
    if codec.starts_with("auto") {
        "veryfast"
    } else if codec.contains("nvenc") {
        "p4"
    } else if codec.contains("amf") {
        "balanced"
    } else {
        "veryfast"
    }
}

fn vendor_of(codec: &str) -> usize {
    if codec.starts_with("auto") {
        0
    } else if codec.contains("nvenc") {
        1
    } else if codec.contains("qsv") {
        2
    } else if codec.contains("amf") {
        3
    } else {
        4
    }
}

fn codec_of(codec: &str) -> usize {
    if codec.contains("265") || codec.contains("hevc") {
        1
    } else if codec.contains("av1") || codec.contains("aom") {
        2
    } else {
        0
    }
}

fn container_of(cfg: &Config) -> usize {
    match cfg.container.as_str() {
        "" => 2,
        "mkv" | "matroska" => 1,
        _ => 0,
    }
}

fn container_str(idx: usize) -> &'static str {
    match idx {
        0 => "mp4",
        1 => "mkv",
        _ => "",
    }
}

fn alpha_index_of(cfg: &Config) -> i32 {
    match cfg.alpha_format.to_ascii_lowercase().as_str() {
        "webm_vp9" | "vp9" => 1,
        "mov_prores" | "prores" => 2,
        "mkv_ffv1" | "ffv1" => 3,
        // AV1 alpha is not available in the bundled libaom build.
        "webm_av1" | "av1" => 0,
        _ => 0,
    }
}

fn alpha_str(idx: usize) -> &'static str {
    match idx {
        0 => "",
        1 => "webm_vp9",
        2 => "mov_prores",
        _ => "mkv_ffv1",
    }
}

/// The config UI lives next to ffmpeg.exe in the installed folder.
fn detect_ffmpeg_path() -> String {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for cand in [dir.join("ffmpeg.exe"), dir.join("bin").join("ffmpeg.exe")] {
                if cand.is_file() {
                    return cand.to_string_lossy().to_string();
                }
            }
        }
    }
    "ffmpeg".to_string()
}

/// Compact path for the footer: "…\MMDFfmpegEncoder\FFmpegEncoder.ini".
fn shorten_path(path: &str) -> String {
    let p = Path::new(path);
    let file = p
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();
    let parent = p
        .parent()
        .and_then(|d| d.file_name())
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();
    if file.is_empty() {
        path.to_string()
    } else if parent.is_empty() {
        format!("…\\{}", file)
    } else {
        format!("…\\{}\\{}", parent, file)
    }
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;
    let cfg = load_config();

    ui.set_vendor_index(vendor_of(&cfg.codec) as i32);
    ui.set_codec_index(codec_of(&cfg.codec) as i32);
    ui.set_rate_index(if cfg.rate_mode == "cbr" || cfg.bitrate > 0 {
        1
    } else {
        0
    });
    ui.set_kbps_input(if cfg.bitrate > 0 {
        (cfg.bitrate / 1000).max(1).to_string().into()
    } else {
        "8000".into()
    });
    ui.set_container_index(container_of(&cfg) as i32);
    ui.set_alpha_index(alpha_index_of(&cfg));
    let config_full = config_file().to_string_lossy().to_string();
    ui.set_config_path(config_full.clone().into());
    ui.set_config_short(shorten_path(&config_full).into());

    let weak = ui.as_weak();
    ui.on_save_requested(move || {
        let ui = weak.unwrap();
        let vendor = ui.get_vendor_index().max(0) as usize;
        let codec = ui.get_codec_index().max(0) as usize;
        let rate = ui.get_rate_index().max(0) as usize;
        let kbps: i32 = ui
            .get_kbps_input()
            .trim()
            .parse()
            .unwrap_or(8000)
            .clamp(1, 400_000);
        let container = ui.get_container_index().max(0) as usize;
        let alpha = ui.get_alpha_index().max(0) as usize;

        let (codec_str, out_container) = if alpha != 0 {
            let c = match alpha {
                1 => "libvpx-vp9",
                2 => "prores_ks",
                _ => "ffv1",
            };
            let cont = match alpha {
                1 => "webm",
                2 => "mov",
                _ => "mkv",
            };
            (c.to_string(), cont.to_string())
        } else {
            (
                codec_for(vendor, codec).to_string(),
                container_str(container).to_string(),
            )
        };
        let rate_mode = if alpha != 0 {
            "crf".to_string()
        } else if rate == 1 {
            "vbr".to_string()
        } else {
            "crf".to_string()
        };
        let mut cfg = Config {
            ffmpeg_path: detect_ffmpeg_path(),
            codec: codec_str.clone(),
            preset: preset_for(&codec_str).to_string(),
            crf: 18,
            bitrate: if alpha != 0 {
                0
            } else if rate == 1 {
                kbps * 1000
            } else {
                0
            },
            rate_mode,
            extra: String::new(),
            container: out_container,
            container_path: String::new(),
            alpha_format: alpha_str(alpha).to_string(),
            debug: false,
            delete_avi: !(alpha == 0 && container == 2),
            merge_audio: !(alpha == 0 && container == 2),
        };
        if cfg.ffmpeg_path == "ffmpeg" {
            // Keep whatever path was configured before if we can't detect one.
            let old = load_config();
            cfg.ffmpeg_path = old.ffmpeg_path;
        }
        match save_config(&cfg) {
            Ok(()) => {
                ui.set_status_text("已保存 ✓ 下次 MMD 渲染时生效".into());
            }
            Err(e) => {
                ui.set_status_text(format!("保存失败：{}", e).into());
            }
        }
    });

    ui.run()
}
