# MMD FFmpeg 编码器

> [English](README.md)

让 MikuMikuDance（64 位）的 AVI 输出使用现代编码（H.264 / HEVC / AV1），
自动产出**单个带音轨的 MP4/MKV**，支持带透明通道的 WebM / MOV / MKV，
渲染完成后自动清理 MMD 生成的临时 AVI。

兼容版核心改为微软参考 BaseClasses 上构建的原生 x64 C++ DirectShow
滤镜。MMD 进程内只加载很薄的 COM/滤镜外壳，所有编解码工作都在独立的
`ffmpeg.exe` 进程完成；Rust + Slint 配置界面继续保留。

## 功能

- 默认使用兼容性最高的 CPU 编码；也可手动指定 NVIDIA NVENC / Intel
  QSV / AMD AMF，启动前会预检，编码器或驱动不可用时回退 CPU
- 格式：H.264 / HEVC / AV1（不支持的硬件组合自动回退 CPU）
- 码率模式：质量优先（CRF）/ 动态码率（VBR）/ 恒定码率（CBR），kbps 自填；
  CBR 会同时设置目标、最小、最大码率和缓冲区，适合恒定高码率交付视频
- 配置界面会列出 Windows 当前 GPU 适配器和 bundled FFmpeg 中注册的
  NVENC/QSV/AMF 编码器，可手动指定硬件；运行时打开失败会自动回退 CPU
- 输出：MP4（默认）/ MKV / 传统 AVI，渲染成功后自动清理临时 AVI
- 透明输出：WebM (VP9) / MOV (ProRes 4444) / MKV (FFV1 无损)，
  保留 MMD 帧里的 Alpha 通道；透明格式统一走 CPU 编码
- 自动把 MMD AVI 里的音轨无损合并进 MP4（AAC）
- 重复渲染同名文件时自动覆盖，不会因旧文件存在而失败
- FFmpeg 诊断日志写入系统临时目录，方便提交 issue 排查
- 图形化配置界面（Rust + Slint）
- Windows 安装包（NSIS，约 27MB，内含 ffmpeg 7.1）
- 编码器与驱动不兼容时自动降级 CPU，不卡死、不崩溃

> 透明说明：VP9 的 WebM 会用 `alpha_mode=1` 标记透明通道，部分工具
> （包括 ffprobe）会显示成 yuv420p，这是已知现象，Chrome/OBS 等支持
> WebM alpha 的播放器可以看到透明效果。AV1 alpha 需要编码器本身支持，
> 当前内置 libaom 不支持，因此界面不提供「WebM AV1 透明」。

## 崩溃与参与修复

`quartz.dll` 是 Windows 自带的 DirectShow 运行时。从 1.2.0 兼容版开始，
COM 生命周期、引脚、媒体类型和分配器统一交给微软原生 BaseClasses，
不再使用 Rust 自动生成的 COM thunk，专门移除了 1.1.7/1.1.8 在 Win10 上
触发的 `OutputPin::ConnectedTo` 崩溃路径。提 issue 时请附上：

- `%TEMP%\MMDFfmpegEncoder\ffmpeg_encoder_debug.log`
- Windows 错误报告生成的 dmp（如果有）
- Windows 版本号（`winver`）以及 `C:\Windows\System32\quartz.dll`
  的文件版本

如果你有能力自己调试，最好的方式是 **fork 本仓库、动手尝试修复并提交
Pull Request**——没有谁能覆盖所有 Windows / quartz.dll 组合，社区的修复
是让编码器对所有人更稳定的最快途径。

## 项目结构

```
├── native-filter/ 原生 C++ DirectShow 滤镜（发布实现）
│   ├── src/       BaseClasses 变换滤镜与 ffmpeg 子进程
│   ├── tests/     无需注册的 COM ABI 压力测试
│   └── third_party/ 微软 DirectShow BaseClasses
├── filter/       保留用于诊断/回退的旧 Rust 滤镜
├── ui/           Slint 配置界面（Rust 二进制）
│   ├── src/      ini 读写、编码器映射
│   └── ui/       界面定义（main.slint）
├── installer/    NSIS 安装脚本
├── Cargo.toml    workspace
└── LICENSE       GPL-3.0
```

## 构建

需要 Rust（stable）、Visual Studio 2022 C++ Build Tools 和 Windows 10/11 SDK。

```powershell
# 原生 x64 DirectShow DLL + ABI 压力测试
native-filter\build.cmd
native-filter\build\abi_smoke.exe native-filter\build\FFmpegVideoEncoder.dll

# Rust/Slint 配置界面
cargo build --release -p mmd_encoder_config

# 产物
# native-filter/build/FFmpegVideoEncoder.dll  DirectShow 滤镜
# target/release/mmd_encoder_config.exe  配置界面
```

注册滤镜（需要管理员）：

```powershell
regsvr32 "native-filter\build\FFmpegVideoEncoder.dll"
```

打包安装程序（需要 NSIS 3.x）：

```powershell
makensis installer\install.nsi
```

## 配置

安装后打开「MMDEncoderConfig.exe」：

- 硬件加速：自动（推荐）/ NVIDIA (NVENC) / Intel (QSV) / AMD (AMF) /
  不使用（CPU）
- 编码格式：H.264 / HEVC / AV1
- 码率：质量优先（CRF）/ 动态码率（VBR），kbps 自填
- 输出：MP4（推荐）/ MKV / AVI（旧）
- 透明输出：关闭 / WebM (VP9) / MOV (ProRes 4444) / MKV (FFV1)

配置文件 `FFmpegEncoder.ini` 与程序放在同一目录，卸载时一并删除。

## 在 MMD 中使用

1. 打开 MMD → 文件 → 渲染到 AVI 文件
2. 视频压缩器选择 **FFmpeg Video Encoder (H.264/HEVC/AV1)**
3. 若需要声音，在 MMD 的 AVI 输出设置中指定音乐 WAV 文件
4. 需要透明背景时，在配置界面选择透明输出格式，并确保 MMD 输出本身
   包含 Alpha 通道
5. 渲染完成后，输出目录只有 `同名.mp4/.webm/.mov/.mkv`（含音轨）

## 工作原理

```
MMD 渲染帧 → MMDxShow → SampleGrabber → 本滤镜 → AVI Mux → MMD 写 .avi
                                        │
                                        ├─ ffmpeg 编码（NVENC/QSV/AMF/CPU）
                                        ├─ 同时封装 .mp4/.mkv
                                        │  （透明模式另写 .webm/.mov/.mkv）
                                        └─ 完成后：音轨合并 + 删除 .avi
```

COM 生命周期、`IBaseFilter`、输入/输出引脚、媒体协商和分配器由微软原生
DirectShow BaseClasses 实现。项目代码负责校验 MMD 的 RGB 媒体类型、把帧
送往进程外 ffmpeg，并为 AVI Mux 切分 Annex B 数据；MMD 不加载任何 FFmpeg
编解码 DLL。

## 许可

本项目以 **GPL-3.0** 发布（见 [LICENSE](LICENSE)）。

- 滤镜与配置界面为本项目原创代码；原生滤镜使用微软 MIT 许可的 Windows
  Classic Samples DirectShow BaseClasses；
- 依赖：`windows` / `windows-core`（MIT/Apache-2.0）、`slint`
  （GPL-3.0 或商业）、ffmpeg（GPL 构建）。
- 第三方组件清单与许可详见
  [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
- 随安装包分发的 ffmpeg（GPL 构建）对应源码可从
  <https://ffmpeg.org/download.html> 获取；Slint 的 GPL 版本源码见
  <https://github.com/slint-ui/slint>。
- 安装程序包含 GPL-3.0 许可协议页，并在安装目录附带 `LICENSE` 与
  `THIRD_PARTY_NOTICES.md`。

## 致谢

- [ffmpeg](https://ffmpeg.org/)
- [Slint](https://slint.dev/)
- [NSIS](https://nsis.sourceforge.io/)
- MikuMikuDance 社区
