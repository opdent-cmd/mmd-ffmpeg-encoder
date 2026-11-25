# MMD FFmpeg Encoder

> [简体中文](README.zh-CN.md)

A x64 DirectShow video encoder for MikuMikuDance. It sends MMD's RGB frames to
an isolated `ffmpeg.exe` process and can encode H.264, HEVC, or AV1 into MP4,
MKV, or AVI. The filter does not load FFmpeg codec libraries inside MMD.

## Use

1. Run `dist/MMDFfmpegEncoder-Setup.exe`. The installer registers the x64
   DirectShow encoder and installs the settings utility and FFmpeg.
2. Open **MMD FFmpeg Encoder Settings** and choose the encoder, video codec,
   rate-control mode, and output container.
3. In MMD, open **File → Render to AVI File**, select
   **FFmpeg Video Encoder (H.264/HEVC/AV1)** as the compressor, and render.
4. To include audio, select a WAV track in MMD's AVI output settings. After
   rendering, the filter attempts to merge audio into the configured output
   and remove the temporary AVI.

The default is MP4, H.264, CPU encoding, and CRF quality mode. Hardware
encoding requires a compatible GPU, driver, and FFmpeg encoder. If the startup
preflight fails, the filter falls back to the corresponding CPU encoder.

## Rate Control

The bitrate field is in kbps and applies to the video stream, not audio or
container overhead. Measured average bitrate varies with clip duration, scene
complexity, and encoder latency; rate control does not guarantee a file size.

| Mode | Current behavior | Use when |
|---|---|---|
| CRF | Prioritizes quality without a target bitrate; default CRF is 18 | Bandwidth and file size are not fixed requirements |
| VBR | Uses the entered value as the average target, with a peak limit of 1.5× target and a VBV buffer of 2× target | Average rate matters, but complex scenes may use more bits |
| CBR | Sets target, minimum, and maximum to the same value, with a VBV buffer of 2× target; enables encoder HRD/filler controls where supported | A steadier delivery or transport rate is required |

The encoder determines how closely those constraints are met. NVENC uses an
explicit rate-control mode; AMF VBR uses peak-constrained VBR; QSV derives its
mode from the target and peak rates. The CPU H.264/HEVC paths configure VBV;
H.264 uses filler NAL units, while HEVC uses x265 HRD and strict-CBR options.
Filler behavior and rate variation differ by encoder, GPU, driver, and
version. CBR therefore does not promise an exact rate in every short interval
or an exact final file size. Total file bitrate also includes audio and
container overhead.

Choose VBR to cap peaks. To reduce file size, lower the VBR target rather than
using CBR as a file-size limit. Alpha output uses a separate CPU encoding path
and does not use these VBR/CBR settings.

## Alpha Output

MMD hands the filter 32-bit frames with premultiplied alpha (the colors have
already been scaled by alpha), while FFmpeg's `yuva*`/`gbrap` pixel formats
expect straight alpha. The filter divides the colors by alpha before writing
them to `ffmpeg.exe`, so semi-transparent areas export with the same colors MMD
shows in the viewport; fully opaque pixels are untouched. When the upstream type
declares straight alpha (the `AMCONTROL_STRAIGHT_ALPHA` flag in
VIDEOINFOHEADER2) the conversion is skipped. The decision is logged as an
`Input alpha:` line in `%TEMP%\MMDFfmpegEncoder\ffmpeg_encoder_debug.log`.

| Option | Encoder arguments | Why |
|---|---|---|
| WebM (VP9 alpha) | `libvpx-vp9`, `yuva444p`, `-strict experimental`, `-auto-alt-ref 0`, CRF 24 | 4:4:4 avoids the color shift that 4:2:0 chroma upsampling adds on semi-transparent edges |
| MOV (ProRes 4444 alpha) | `prores_ks`, `yuva444p10le`, `-alpha_bits 8` | matches an 8-bit source and avoids alpha rounding differences in editors |
| MKV (FFV1 lossless alpha) | `ffv1`, `yuva444p10le`, `-level 3` | 10-bit planes keep the RGB→YUV round trip lossless |

These arguments were verified against FFmpeg 7.1 and FFmpeg 9, so the same
alpha path ships on both release lines. The alpha path uses fixed arguments;
the bitrate mode in the settings UI does not apply to VP9 alpha.
Encoded output is delivered directly from the stdout reader to AVI Mux; the
release filter no longer stores an encoded-frame queue. If downstream delivery
falls behind, stdout reading pauses and the pipe applies backpressure, so memory
no longer grows with render length.

## Encoders and Output

The repository maintains two FFmpeg-major release lines: `main` bundles FFmpeg
9 for current hardware, while `ffmpeg-7` bundles FFmpeg 7.1 for devices that
still require older NVENC/QSV/AMF support. Both lines share the filter, settings
UI, and VBR/CBR fixes, but keep version-specific hardware encoder arguments in
their respective branches. Do not move FFmpeg-specific options between the
branches without checking that build's encoder help.
FFmpeg 7.1 NVENC CBR requires `-cbr 1`; FFmpeg 9 removed that option, and
passing it makes hardware preflight fail and fall back to CPU. The branches
keep this difference isolated, restoring CBR on the legacy line without
reintroducing an invalid option on the current line.

- Video codecs: H.264, HEVC, and AV1; CPU, NVIDIA NVENC, Intel QSV, or AMD AMF.
- Containers: MP4, MKV, or MMD AVI only.
- Alpha output: WebM/VP9, MOV/ProRes 4444, or MKV/FFV1. Alpha modes use CPU
  encoding.
- The settings utility detects FFmpeg encoders and Windows GPU adapters. A
  listed encoder may still fail to initialize with a particular rate mode or
  driver; the filter preflights the selected parameters before rendering.
- `main` bundles gyan.dev's static GPL essentials FFmpeg
  `2026-10-19-git-2344518cc5` (FFmpeg 9). The compatible FFmpeg 7.1 installer
  is built separately from `ffmpeg-7`. Choose the FFmpeg 9 or FFmpeg 7
  installer to match the GPU generation.

## Troubleshooting

FFmpeg process logs are written to:

```text
%TEMP%\MMDFfmpegEncoder\ffmpeg_encoder_debug.log
```

Include the log, Windows version, selected encoder/rate mode/bitrate, and
reproduction steps when reporting an issue. If the rendered video is missing
several seconds at the beginning, temporarily set the following in
`FFmpegEncoder.ini` beside the installed filter:

```ini
[video]
delete_avi=0
merge_audio=0
```

Render again and compare the retained AVI with the encoded output. If both
files miss the same frames, investigate MMD's frame input and the shared
encoding path. If only the AVI is affected, inspect AVI Mux; if only the
configured output is affected, inspect parallel muxing and audio post-processing.
Check that any sample shared for diagnosis contains no private or sensitive
footage. Restore the settings after testing.

## Build from Source

Requirements: Windows 10/11 x64, Rust stable, Visual Studio 2022 C++ Build
Tools, Windows SDK, NSIS 3.x, and an FFmpeg build. Releases ship one installer
that carries both FFmpeg major lines: the bundled `bin\ffmpeg.exe` is the
default, and the settings UI can point at a different build.

```powershell
native-filter\build.cmd
native-filter\build\rate_control_args.exe
native-filter\build\alpha_color.exe
native-filter\build\abi_smoke.exe native-filter\build\FFmpegVideoEncoder.dll
cargo test -p ffmpeg_encoder
cargo build --release -p mmd_encoder_config
New-Item -ItemType Directory -Force dist | Out-Null
$output = Join-Path (Resolve-Path dist).Path 'MMDFfmpegEncoder-Setup.exe'
$ffmpeg = '<path to the FFmpeg 7.1 or FFmpeg 9 ffmpeg.exe>'
& "${env:ProgramFiles(x86)}\NSIS\makensis.exe" /V2 "/DOUTPUT_PATH=$output" "/DFFMPEG_PATH=$ffmpeg" installer\install.nsi
```

`/DFFMPEG_PATH` is optional; without it the installer leaves `bin\ffmpeg.exe`
out and the filter uses the path stored in `FFmpegEncoder.ini`. The installer is
written to the `/DOUTPUT_PATH` you pass. The filter and settings utility can be
built independently; distribute an installer only after checking its contents.
Installation requires administrator privileges to register the DirectShow
filter.

## Repository Layout

- `native-filter/`: release C++ DirectShow filter, ABI checks, and rate-control tests.
- `filter/`: retained Rust encoder implementation and rate-control tests.
- `ui/`: Rust + Slint settings utility.
- `installer/`: NSIS installer script.
- `bin/ffmpeg.exe`: local FFmpeg packaging input; executable files are ignored by Git.

## License

This project is licensed under GPL-3.0. FFmpeg is a GPL build; its version,
source revision, checksum, and third-party notices are listed in
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). The installer includes the
project's GPL license text. The DirectShow BaseClasses license is in
[`native-filter/third_party/WINDOWS_CLASSIC_SAMPLES_LICENSE`](native-filter/third_party/WINDOWS_CLASSIC_SAMPLES_LICENSE).
