# Native DirectShow filter

This directory contains the Windows 10/11 compatibility implementation. The
DLL uses Microsoft's native C++ DirectShow BaseClasses for COM lifetime, pins,
media negotiation, and allocators. Encoding remains out of process in
`ffmpeg.exe`.

Build on x64 Windows with Visual Studio 2022 Build Tools and a Windows 10/11
SDK:

```cmd
native-filter\build.cmd
native-filter\build\abi_smoke.exe native-filter\build\FFmpegVideoEncoder.dll
native-filter\build\test_graph.exe --list
native-filter\build\alpha_color.exe
```

The ABI smoke test loads the DLL without registration and repeats the MMD pin
interrogation sequence 3000 times, including `IPin::ConnectedTo` while both
pins are disconnected. `test_graph.exe` is a DirectShow graph diagnostic
harness; `--list` verifies that the system compressor category can be queried,
and its `--raw` mode is safe even when no encoder filter is created.
`alpha_color.exe` pins the premultiplied→straight alpha conversion used before
frames reach `ffmpeg.exe`; it must print `PASS premultiplied alpha conversion`.
