# MMD FFmpeg Encoder

> [English](README.md)

MMD 的 x64 DirectShow 视频编码器。它把 MMD 输出的 RGB 帧交给独立的
`ffmpeg.exe` 编码，可生成 H.264、HEVC 或 AV1 视频，并按配置封装为 MP4、
MKV 或 AVI。滤镜不会在 MMD 进程中加载 FFmpeg 编解码库。

## 使用

1. 运行 `dist/MMDFfmpegEncoder-Setup.exe` 安装。安装程序注册 x64 DirectShow
   编码器，并安装配置工具和 FFmpeg。
2. 打开「MMD FFmpeg 编码器配置」，选择编码器、视频格式、码率模式和输出容器。
3. 在 MMD 中打开「文件 → 渲染到 AVI 文件」，将视频压缩器设为
   **FFmpeg Video Encoder (H.264/HEVC/AV1)**，然后开始渲染。
4. 如需音频，在 MMD 的 AVI 输出设置中选择 WAV 音轨。渲染结束后，程序会尝试
   将音轨合并到配置的输出文件，并清理临时 AVI。

默认输出 MP4、H.264、CPU 编码和 CRF 质量模式。硬件编码需匹配的 GPU、驱动和
FFmpeg 编码器；启动预检失败时会回退到对应的 CPU 编码器。

## 码率控制

码率输入单位为 kbps。这里的数值用于视频流，不包含音频和容器开销；短片、静态
画面和编码器延迟都会影响测得的平均码率。码率控制不是文件大小保证。

| 模式 | 当前行为 | 适用情况 |
|---|---|---|
| CRF | 以画质为优先，不指定目标码率；CRF 默认值为 18 | 不要求固定带宽或文件大小时使用 |
| VBR | 输入值作为目标平均码率；峰值上限为目标值的 1.5 倍，VBV 缓冲为目标值的 2 倍 | 需要控制平均码率，同时允许复杂画面使用更多码率 |
| CBR | 目标、最小和最大码率设为相同值，VBV 缓冲为目标值的 2 倍；可用时启用编码器的 HRD/filler 控制 | 需要较稳定的传输码率或交付码率 |

VBR/CBR 的具体执行由编码器决定：NVENC 使用显式 rate-control 模式，AMF 的 VBR
使用峰值约束模式，QSV 依据目标码率与峰值设置工作。CPU 的 H.264/HEVC 路径会配置
VBV；H.264 使用 filler NAL，HEVC 使用 x265 的 HRD/strict-CBR 选项。不同硬件、
驱动和编码器版本对填充数据及码率波动的处理不同，因此 CBR 不承诺每个短时间窗口
或最终文件都精确等于输入值。最终文件的总码率还包含音频和封装开销。

如果目标是限制带宽峰值，选 VBR；如果目标是压低文件体积，优先调整 VBR 目标值，
不要把 CBR 当作文件大小限制。透明输出使用独立的 CPU 编码路径，不应用此处的
VBR/CBR 设置。

## 编码器与输出

仓库按 FFmpeg 大版本维护两个发行分支：`main` 使用 FFmpeg 9，面向当前硬件；
`ffmpeg-7` 使用 FFmpeg 7.1，供仍依赖旧 NVENC/QSV/AMF 支持的设备使用。两者的
滤镜、配置工具和 VBR/CBR 修复保持一致，但硬件编码参数按各自 FFmpeg 版本分别维护，
不要把一个分支的 FFmpeg 参数直接移植到另一个分支。FFmpeg 7.1 的 NVENC CBR
需要 `-cbr 1`；FFmpeg 9 已移除此选项，继续传入会导致硬件预检失败并回退到 CPU。
两个分支分别维护该参数，修复旧版 CBR 的同时避免新版使用不兼容参数。

- 视频编码：H.264、HEVC、AV1；可选 CPU、NVIDIA NVENC、Intel QSV 或 AMD AMF。
- 输出容器：MP4、MKV，或只保留 MMD AVI。
- Alpha 输出：WebM/VP9、MOV/ProRes 4444、MKV/FFV1。透明模式固定走 CPU 编码。
- 配置工具会探测 FFmpeg 编码器和 Windows GPU 适配器；探测结果不代表每种编码模式
  都能在当前驱动上启动，实际参数仍会在渲染前预检。
- `main` 安装包使用 gyan.dev 静态 GPL essentials FFmpeg
  `2026-10-19-git-2344518cc5`（FFmpeg 9）；兼容旧硬件的 FFmpeg 7.1 安装包从
  `ffmpeg-7` 分支单独构建。下载时根据显卡代际选择 FFmpeg 9 或 FFmpeg 7 安装包。

## 故障诊断

FFmpeg 进程日志位于：

```text
%TEMP%\MMDFfmpegEncoder\ffmpeg_encoder_debug.log
```

报告问题时请附上日志、Windows 版本、所选编码器/码率模式/码率，以及可复现步骤。
如果成片开头缺少数秒画面，请临时在安装目录的 `FFmpegEncoder.ini` 中设置：

```ini
[video]
delete_avi=0
merge_audio=0
```

重新渲染后保留临时 AVI，并与编码文件对照：两者缺少相同画面时，检查 MMD 帧输入和
共用的编码路径；只有 AVI 缺帧时，检查 AVI Mux；只有配置输出缺帧时，检查并行封装和
音频后处理。分享前请确认样片不含隐私或敏感内容。定位完成后可恢复这两个设置。

## 从源码构建

需要 Windows 10/11 x64、Rust stable、Visual Studio 2022 C++ Build Tools、Windows
SDK、NSIS 3.x，以及放在仓库 `bin/ffmpeg.exe` 的 FFmpeg 构建。

```powershell
native-filter\build.cmd
native-filter\build\rate_control_args.exe
native-filter\build\abi_smoke.exe native-filter\build\FFmpegVideoEncoder.dll
cargo test -p ffmpeg_encoder
cargo build --release -p mmd_encoder_config
New-Item -ItemType Directory -Force dist | Out-Null
$output = Join-Path (Resolve-Path dist).Path 'MMDFfmpegEncoder-Setup-FFmpeg7.exe'
$ffmpeg = 'bin\ffmpeg.exe' # Replace with the FFmpeg 7.1 executable path.
& "${env:ProgramFiles(x86)}\NSIS\makensis.exe" /V2 "/DOUTPUT_PATH=$output" "/DFFMPEG_PATH=$ffmpeg" installer\install.nsi
```

安装包生成于 `dist/MMDFfmpegEncoder-Setup.exe`。滤镜和配置程序可单独构建；只有在
确认打包内容后才应分发安装程序。安装需要管理员权限以注册 DirectShow 滤镜。

## 项目结构

- `native-filter/`：发布使用的 C++ DirectShow 滤镜及 ABI、码率参数测试。
- `filter/`：保留的 Rust 编码实现和码率参数测试。
- `ui/`：Rust + Slint 配置工具。
- `installer/`：NSIS 安装脚本。
- `bin/ffmpeg.exe`：本地打包使用的 FFmpeg；可执行文件由 `.gitignore` 排除。

## 许可

本项目以 GPL-3.0 发布。FFmpeg 为 GPL 构建；其版本、源码提交、校验值和第三方组件
许可见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。安装包包含项目的 GPL
许可文本。DirectShow BaseClasses 许可见
[`native-filter/third_party/WINDOWS_CLASSIC_SAMPLES_LICENSE`](native-filter/third_party/WINDOWS_CLASSIC_SAMPLES_LICENSE)。
