# MMD FFmpeg Encoder

> [中文版](README.zh-CN.md)

Let MikuMikuDance (x64) use modern encoders (H.264 / HEVC / AV1) for its AVI
output, automatically produce a **single MP4/MKV with audio**, support
alpha-channel WebM / MOV / MKV, and clean up the temporary AVI after
rendering.

The core is a 64-bit DirectShow "video compressor" filter written in
**Rust**: MMD hands rendered frames to the filter, which feeds them into an
ffmpeg child process for encoding, writes the encoded stream back to MMD's
AVI Mux, and has ffmpeg mux the same stream into an MP4/MKV at the same
time.

## Features

- Hardware encoding: auto-detect (recommended) or pick NVIDIA NVENC /
  Intel QSV / AMD AMF; falls back to CPU (libx264/libx265) when no GPU
  encoder is available.
- Formats: H.264 / HEVC / AV1 (unsupported hardware combinations
  automatically fall back to CPU).
- Bitrate modes: quality-first (CRF) / variable bitrate (VBR), with a
  user-defined kbps value.
- Output: MP4 (default) / MKV / legacy AVI; the temporary AVI is removed
  after a successful render.
- Transparent output: WebM (VP9) / MOV (ProRes 4444) / MKV (FFV1 lossless)
  preserving the alpha channel from MMD frames; transparent formats always
  use CPU encoding.
- The audio track in MMD's AVI is merged into the MP4/MKV losslessly (AAC).
- Re-rendering to the same filename overwrites automatically.
- On errors, a log is written to the system temp folder and opened in
  Notepad, making crashes / encode failures easy to report.
- Graphical configuration UI (Rust + Slint).
- Windows installer (NSIS, ~27 MB, bundles ffmpeg 7.1).
- Automatic CPU fallback when the GPU encoder is incompatible with the
  installed driver; no hangs, no crashes from that path.

> Transparency note: VP9 WebM marks the alpha plane with `alpha_mode=1`;
> some tools (including ffprobe) will report `yuv420p`, which is a known
> quirk — players that support WebM alpha (Chrome, OBS, etc.) show the
> transparency correctly. AV1 alpha requires encoder support; the bundled
> libaom does not support it, so "WebM AV1 transparent" is not offered.

## Crashes & Contributing Fixes

`quartz.dll` is a Windows system component (the DirectShow runtime), and its
behavior differs between Windows versions. A render graph that works on one
machine may crash on another, so environment-specific crashes are expected.
When filing an issue, please attach:

- `%TEMP%\MMDFfmpegEncoder\ffmpeg_encoder.log`
- `crash_*.dmp` in the same folder, if present (written automatically by
  v1.1.7+ when the process dies with an unhandled exception)
- Windows build number (`winver`) and the file version of
  `C:\Windows\System32\quartz.dll`

If you are able to debug it yourself, the best way to help is to **fork this
repository, attempt a fix, and open a Pull Request** — no single developer
can test every Windows / quartz.dll combination, so community fixes are the
fastest path to making the encoder stable for everyone.

## Project Structure

```
├── filter/       DirectShow filter (Rust cdylib)
│   └── src/      COM filter, input/output pins, ffmpeg child, Annex B splitter
├── ui/           Slint config UI (Rust binary)
│   ├── src/      ini read/write, encoder mapping
│   └── ui/       UI definition (main.slint)
├── installer/    NSIS installer script
├── Cargo.toml    workspace
└── LICENSE       GPL-3.0
```

## Build

Requires Rust (stable), Windows 10/11, and VS Build Tools (C++ linker).

```powershell
# Build the workspace (filter + config UI)
cargo build --release

# Outputs
# target/release/ffmpeg_encoder.dll        DirectShow filter
# target/release/mmd_encoder_config.exe    config UI
```

Register the filter (requires administrator):

```powershell
regsvr32 "target\release\ffmpeg_encoder.dll"
```

Build the installer (requires NSIS 3.x):

```powershell
makensis installer\install.nsi
```

## Configuration

After installation, open `MMDEncoderConfig.exe`:

- Hardware: Auto (recommended) / NVIDIA (NVENC) / Intel (QSV) / AMD (AMF) /
  CPU only
- Codec: H.264 / HEVC / AV1
- Bitrate: quality-first (CRF) / variable bitrate (VBR), kbps user-defined
- Output: MP4 (recommended) / MKV / AVI (legacy)
- Transparent output: Off / WebM (VP9) / MOV (ProRes 4444) / MKV (FFV1)

The config file `FFmpegEncoder.ini` lives next to the program and is removed
together with the app on uninstall.

## Using with MMD

1. Open MMD → File → Render to AVI File
2. Pick **FFmpeg Video Encoder (H.264/HEVC/AV1)** as the video compressor
3. If you need sound, specify the music WAV in MMD's AVI output settings
4. For transparency, choose a transparent output in the config UI and make
   sure MMD's output actually contains an alpha channel
5. After rendering, the output folder contains only
   `same-name.mp4/.webm/.mov/.mkv` (with audio)

## How It Works

```
MMD rendered frames → MMDxShow → SampleGrabber → this filter → AVI Mux → MMD writes .avi
                                        │
                                        ├─ ffmpeg encodes (NVENC/QSV/AMF/CPU)
                                        ├─ muxes .mp4/.mkv at the same time
                                        │  (transparent mode writes .webm/.mov/.mkv)
                                        └─ after render: merge audio + delete .avi
```

The filter implements all COM interfaces in Rust (IBaseFilter / IPin /
IMemInputPin / IMemAllocator / IMediaSample) and handles media types,
allocator lifetime, sample timestamps, and Annex B packet splitting for AVI
Mux compatibility; frames arriving before the encoder starts are buffered
and written once it is ready, so the beginning of the render is never lost.

## License

This project is released under **GPL-3.0** (see [LICENSE](LICENSE)).

- The filter and config UI are original code of this project.
- Dependencies: `windows` / `windows-core` (MIT/Apache-2.0), `slint`
  (GPL-3.0 or commercial), ffmpeg (GPL build).
- Third-party notices: [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)
- The bundled ffmpeg (GPL build) source is available from
  <https://ffmpeg.org/download.html>; Slint GPL source is at
  <https://github.com/slint-ui/slint>.
- The installer ships the GPL-3.0 license page and includes `LICENSE` and
  `THIRD_PARTY_NOTICES.md` in the install directory.

## Acknowledgements

- [ffmpeg](https://ffmpeg.org/)
- [Slint](https://slint.dev/)
- [NSIS](https://nsis.sourceforge.io/)
- The MikuMikuDance community
