# 第三方组件与许可声明

本项目以 **GPL-3.0** 发布。以下是随项目一起分发或构建时使用的第三方
组件及其许可证。GPL-3.0 全文见 [LICENSE](LICENSE)。

## Rust 依赖（crates.io）

| 组件 | 版本 | 许可证 |
|---|---|---|
| windows / windows-core 及 windows-\* 系列 | 0.61.x / 0.62.x | MIT OR Apache-2.0 |
| slint / slint-build / i-slint-\* | 1.17.1 | GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0 |
| winit、softbuffer、femtovg、accesskit、glutin 等 UI 传递依赖 | 见 Cargo.lock | MIT / Apache-2.0 / BSD / Zlib 等宽松许可 |

本仓库按 Slint 的 GPL-3.0 条款使用 Slint；Slint 源码：
<https://github.com/slint-ui/slint>（GPL-3.0 版本）。

## 随安装包分发的二进制

| 组件 | 版本 | 许可证 | 说明 |
|---|---|---|---|
| ffmpeg | 7.1 essentials（gyan.dev 构建） | GPL-3.0（`--enable-gpl`，含 libx264/libx265/libvpx/libaom/libopus 等） | 仅用于编码/封装；构建配置见 `ffmpeg -version`；对应源码：<https://ffmpeg.org/download.html>（此构建对应 gyan.dev 的 7.1 essentials 源码配置） |
| NSIS | 3.12 | zlib 许可（LZMA 压缩模块为 CPL-1.0） | 仅用于制作安装程序，不随安装包分发 |

## 构建/测试时使用的第三方源码

| 组件 | 许可证 | 说明 |
|---|---|---|
| Microsoft Windows-classic-samples（DirectShow BaseClasses） | MIT | 源码位于 `native-filter/third_party/baseclasses`，静态链接进发布滤镜；许可文本见 `native-filter/third_party/WINDOWS_CLASSIC_SAMPLES_LICENSE` |

## GPL 合规说明

- 本项目全部原创代码以 GPL-3.0 发布，对应源码即本仓库；
- ffmpeg 为 GPL 构建，随安装包分发时已在此声明其许可证与源码获取方式；
- 安装包内附有本文件与 GPL-3.0 全文（`LICENSE`）。

## 联网核实的许可来源

- gyan.dev ffmpeg builds（<https://www.gyan.dev/ffmpeg/builds/>）：
  “All builds are 64-bit, static and licensed as GPLv3”；
- Slint（<https://slint.dev/faqs>）：GPLv3 / royalty-free / commercial
  三许可，开源项目可按 GPLv3 免费使用；
- NSIS（<https://nsis.sourceforge.io/Docs/AppendixI.html>）：
  zlib 许可，压缩模块 bzip2/LZMA 分别为 bzip2/CPL-1.0 许可。
