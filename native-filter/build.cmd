@echo off
setlocal EnableExtensions

set "ROOT=%~dp0"
set "BUILD=%ROOT%build"
set "BASE=%ROOT%third_party\baseclasses"
set "SRC=%ROOT%src"

set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
if not exist "%VSWHERE%" (
  echo ERROR: Visual Studio Build Tools 2022 was not found.
  exit /b 1
)
for /f "usebackq tokens=*" %%I in (`"%VSWHERE%" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "VSROOT=%%I"
if not defined VSROOT (
  echo ERROR: The Visual C++ x64 toolchain was not found.
  exit /b 1
)
call "%VSROOT%\VC\Auxiliary\Build\vcvars64.bat" >nul
if errorlevel 1 exit /b 1

if not exist "%BUILD%\obj\base" mkdir "%BUILD%\obj\base"
if not exist "%BUILD%\obj\filter" mkdir "%BUILD%\obj\filter"

set "COMMON=/nologo /c /O2 /MT /EHsc /std:c++17 /utf-8 /guard:cf /DWIN32 /D_WINDOWS /D_UNICODE /DUNICODE /D_WIN32_WINNT=0x0601 /DWINVER=0x0601 /I"%BASE%""

if not exist "%BUILD%\strmbase.lib" (
  echo Building Microsoft DirectShow BaseClasses...
  for %%F in ("%BASE%\*.cpp") do (
    if /I not "%%~nxF"=="dllentry.cpp" (
      cl %COMMON% /W3 /Fo"%BUILD%\obj\base\%%~nF.obj" "%%~fF" || exit /b 1
    )
  )
  lib /nologo /out:"%BUILD%\strmbase.lib" "%BUILD%\obj\base\*.obj" || exit /b 1
)

echo Building native x64 DirectShow filter...
cl %COMMON% /W4 /I"%SRC%" /Fo"%BUILD%\obj\filter\ffmpeg_encoder.obj" "%SRC%\ffmpeg_encoder.cpp" || exit /b 1
cl %COMMON% /W4 /I"%SRC%" /Fo"%BUILD%\obj\filter\dll.obj" "%SRC%\dll.cpp" || exit /b 1
rc /nologo /fo"%BUILD%\obj\filter\version.res" "%ROOT%version.rc" || exit /b 1

link /nologo /DLL /MACHINE:X64 /DYNAMICBASE /NXCOMPAT /GUARD:CF /OPT:REF /OPT:ICF ^
  /OUT:"%BUILD%\FFmpegVideoEncoder.dll" /PDB:"%BUILD%\FFmpegVideoEncoder.pdb" ^
  /DEF:"%ROOT%ffmpeg_encoder.def" "%BUILD%\obj\filter\ffmpeg_encoder.obj" ^
  "%BUILD%\obj\filter\dll.obj" "%BUILD%\obj\filter\version.res" ^
  "%BUILD%\strmbase.lib" strmiids.lib ^
  winmm.lib ole32.lib oleaut32.lib user32.lib gdi32.lib advapi32.lib
if errorlevel 1 exit /b 1

echo Building ABI smoke test...
cl /nologo /O2 /MT /W4 /EHsc /std:c++17 /permissive- /utf-8 ^
  /D_UNICODE /DUNICODE /D_WIN32_WINNT=0x0601 /DWINVER=0x0601 ^
  /Fo"%BUILD%\obj\abi_smoke.obj" "%ROOT%tests\abi_smoke.cpp" ^
  /Fe:"%BUILD%\abi_smoke.exe" ole32.lib strmiids.lib
if errorlevel 1 exit /b 1

echo Building graph diagnostic test...
cl /nologo /O2 /MT /W4 /EHsc /std:c++17 /permissive- /utf-8 ^
  /D_UNICODE /DUNICODE /D_WIN32_WINNT=0x0601 /DWINVER=0x0601 ^
  /I"%BASE%" /I"%SRC%" /Fo"%BUILD%\obj\test_graph.obj" "%ROOT%tests\test_graph.cpp" ^
  /Fe:"%BUILD%\test_graph.exe" ole32.lib oleaut32.lib strmiids.lib
if errorlevel 1 exit /b 1

echo Built %BUILD%\FFmpegVideoEncoder.dll
echo Built %BUILD%\abi_smoke.exe
echo Built %BUILD%\test_graph.exe
endlocal
