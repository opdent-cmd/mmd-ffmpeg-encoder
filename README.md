# MMD FFmpeg 编码器

让 MikuMikuDance（64 位）的 AVI 输出使用现代编码（H.264 / HEVC / AV1），
并自动产出 **单个带音轨的 MP4/MKV**，渲染完成后自动清理 MMD 生成的 AVI。

核心是一个用 **Rust** 编写的 64 位 DirectShow「视频压缩器」滤镜：
MMD 把渲染帧交给它，它把帧送入 ffmpeg 子进程编码，再把编码流写回
MMD 的 AVI Mux；同时 ffmpeg 用同一个编码流封装一份 MP4/MKV。

## 功能

- 硬件编码：NVIDIA NVENC / Intel QSV / AMD AMF，或 CPU（libx264/libx265）
- 格式：H.264 / HEVC / AV1（不支持的硬件组合自动回退 CPU）
- 固定码率（kbps 自填）或质量优先（CRF）
- 输出：MP4（默认）/ MKV / 传统 AVI，渲染后自动清理 AVI
- 自动把 MMD AVI 里的音轨无损合并进 MP4（AAC）
- 图形化配置界面（Rust + Slint）
- Windows 安装包（NSIS，约 27MB，内含 ffmpeg 7.1）
- 编码器与驱动不兼容时自动降级 CPU，不卡死、不崩溃

## 项目结构

```
├── filter/       DirectShow 滤镜（Rust cdylib）
│   └── src/      COM 滤镜、输入/输出针、ffmpeg 子进程、Annex B 切包
├── ui/           Slint 配置界面（Rust 二进制）
│   ├── src/      ini 读写、编码器映射
│   └── ui/       界面定义（main.slint）
├── installer/    NSIS 安装脚本
├── tools/        内部 C++ 测试/探针工具（需 DirectShow baseclasses）
├── Cargo.toml    workspace
└── LICENSE       GPL-3.0
```

## 构建

需要 Rust（stable）、Windows 10/11、VS Build Tools（C++ 链接）。

```powershell
# workspace 构建（滤镜 + 配置界面）
cargo build --release

# 产物
# target/release/ffmpeg_encoder.dll    DirectShow 滤镜
# target/release/mmd_encoder_config.exe  配置界面
```

注册滤镜（需要管理员）：

```powershell
regsvr32 "target\release\ffmpeg_encoder.dll"
```

打包安装程序（需要 NSIS 3.x）：

```powershell
makensis installer\install.nsi
```

> 安装包内的 ffmpeg（7.1 essentials，GPL build）体积约 87MB，不包含在本
> 仓库中；发布安装包时从 [gyan.dev](https://www.gyan.dev/ffmpeg/builds/)
> 获取并放入 `bin\` 后执行上述打包命令。

## 配置

安装后打开「MMDEncoderConfig.exe」：

- 硬件加速：NVIDIA (NVENC) / Intel (QSV) / AMD (AMF) / 不使用（CPU）
- 编码格式：H.264 / HEVC / AV1
- 码率：自动（质量优先）或固定码率（kbps）
- 输出：MP4（推荐）/ MKV / AVI（旧）

配置文件 `FFmpegEncoder.ini` 与程序放在同一目录，卸载时一并删除。

## 在 MMD 中使用

1. 打开 MMD → 文件 → 渲染到 AVI 文件
2. 视频压缩器选择 **FFmpeg Video Encoder (H.264/HEVC/AV1)**
3. 若需要声音，在 MMD 的 AVI 输出设置中指定音乐 WAV 文件
4. 渲染完成后，输出目录只有 `同名.mp4`（含音轨）

## 工作原理

```
MMD 渲染帧 → MMDxShow → SampleGrabber → 本滤镜 → AVI Mux → MMD 写 .avi
                                        │
                                        ├─ ffmpeg 编码（NVENC/QSV/AMF/CPU）
                                        ├─ 同时封装 .mp4/.mkv
                                        └─ 完成后：音轨合并 + 删除 .avi
```

滤镜用 Rust 实现全部 COM 接口（IBaseFilter / IPin / IMemInputPin /
IMemAllocator / IMediaSample），并针对 AVI Mux 的兼容性处理了
媒体类型、分配器生命周期、样本时间戳、Annex B 切包等细节。

## 许可

本项目以 **GPL-3.0** 发布（见 [LICENSE](LICENSE)）。

- 滤镜与配置界面为本项目原创代码；
- 依赖：`windows` / `windows-core`（MIT/Apache-2.0）、`slint`
  （GPL-3.0 或商业）、ffmpeg（GPL 构建）。
- 第三方组件清单与许可详见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
- 随安装包分发的 ffmpeg（GPL 构建）对应源码可从
  <https://ffmpeg.org/download.html> 获取；Slint 的 GPL 版本源码见
  <https://github.com/slint-ui/slint>。
- 安装程序包含 GPL-3.0 许可协议页，并在安装目录附带
  `LICENSE` 与 `THIRD_PARTY_NOTICES.md`。

## 致谢

- [ffmpeg](https://ffmpeg.org/)
- [Slint](https://slint.dev/)
- [NSIS](https://nsis.sourceforge.io/)
- MikuMikuDance 社区
