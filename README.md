# MMD FFmpeg Encoder

> [中文版](README.zh-CN.md)

Let MikuMikuDance (x64) use modern encoders (H.264 / HEVC / AV1) for its AVI
output, automatically produce a **single MP4/MKV with audio**, support
alpha-channel WebM / MOV / MKV, and clean up the temporary AVI after
rendering.

The compatibility implementation is a native x64 C++ DirectShow filter built
on Microsoft's reference BaseClasses. MMD only loads this small COM/filter
shell; all codec work stays in a separate `ffmpeg.exe` process. The Rust +
Slint configuration UI is unchanged.

## Features

- Portable CPU encoding is the default. NVIDIA NVENC / Intel QSV / AMD AMF
  remain explicit options and are preflight-tested, with CPU fallback when
  the selected encoder or driver cannot start.
- Formats: H.264 / HEVC / AV1 (unsupported hardware combinations
  automatically fall back to CPU).
- Bitrate modes: quality-first (CRF) / variable bitrate (VBR) / constant
  bitrate (CBR), with a user-defined kbps value. CBR sets target, minimum,
  maximum and buffer size for stable high-bitrate delivery.
- The configuration UI lists Windows GPU adapters and hardware encoders
  registered by the bundled FFmpeg build (NVENC/QSV/AMF), while still allowing
  an explicit hardware choice; runtime preflight falls back to CPU if needed.
- The settings UI and installer follow the system language: Simplified Chinese
  for Chinese locales, English everywhere else.
- Output: MP4 (default) / MKV / legacy AVI; the temporary AVI is removed
  after a successful render.
- Transparent output: WebM (VP9) / MOV (ProRes 4444) / MKV (FFV1 lossless)
  preserving the alpha channel from MMD frames; transparent formats always
  use CPU encoding.
- The audio track in MMD's AVI is merged into the MP4/MKV losslessly (AAC).
- Re-rendering to the same filename overwrites automatically.
- FFmpeg diagnostics are written to the system temp folder for issue reports.
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

`quartz.dll` is a Windows system component (the DirectShow runtime). Starting
with the 1.2.0 compatibility implementation, COM lifetime, pins, media types,
and allocators are handled by Microsoft's native DirectShow BaseClasses rather
than generated Rust COM thunks. This specifically removes the Win10
`OutputPin::ConnectedTo` crash path reported against 1.1.7 and 1.1.8.
When filing an issue, please attach:

- `%TEMP%\MMDFfmpegEncoder\ffmpeg_encoder_debug.log`
- a Windows Error Reporting dump, if one was generated
- Windows build number (`winver`) and the file version of
  `C:\Windows\System32\quartz.dll`

If you are able to debug it yourself, the best way to help is to **fork this
repository, attempt a fix, and open a Pull Request** — no single developer
can test every Windows / quartz.dll combination, so community fixes are the
fastest path to making the encoder stable for everyone.

## Project Structure

```
├── native-filter/ Native C++ DirectShow filter (release implementation)
│   ├── src/       BaseClasses transform filter + ffmpeg child process
│   ├── tests/     registration-free COM ABI stress test
│   └── third_party/ Microsoft DirectShow BaseClasses
├── filter/       Legacy Rust filter retained as a diagnostic fallback
├── ui/           Slint config UI (Rust binary)
│   ├── src/      ini read/write, encoder mapping
│   └── ui/       UI definition (main.slint)
├── installer/    NSIS installer script
├── Cargo.toml    workspace
└── LICENSE       GPL-3.0
```

## Build

Requires Rust (stable), Visual Studio 2022 Build Tools with C++, and a Windows
10/11 SDK.

```powershell
# Native x64 DirectShow DLL + ABI smoke test
native-filter\build.cmd
native-filter\build\abi_smoke.exe native-filter\build\FFmpegVideoEncoder.dll

# Rust/Slint config UI
cargo build --release -p mmd_encoder_config

# Outputs
# native-filter/build/FFmpegVideoEncoder.dll  DirectShow filter
# target/release/mmd_encoder_config.exe    config UI
```

Register the filter (requires administrator):

```powershell
regsvr32 "native-filter\build\FFmpegVideoEncoder.dll"
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

Microsoft's native DirectShow BaseClasses implement COM lifetime, `IBaseFilter`,
pins, media negotiation, and allocators. Project code validates MMD's RGB media
types, streams frames to the out-of-process ffmpeg encoder, and splits Annex B
packets for AVI Mux. No FFmpeg codec DLL is loaded into MMD.

## License

This project is released under **GPL-3.0** (see [LICENSE](LICENSE)).

- The filter and config UI are original project code; the native filter uses
  Microsoft's MIT-licensed Windows Classic Samples DirectShow BaseClasses.
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
