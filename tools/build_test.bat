@echo off
setlocal

set ROOT=%~dp0
set SRC=%ROOT%src
set BUILD=%ROOT%build
set BC=%ROOT%third_party\windows-classic-samples\Samples\Win7Samples\multimedia\directshow\baseclasses
set SDKINC=C:\Program Files (x86)\Windows Kits\10\Include\10.0.26100.0
set SDKLIB=C:\Program Files (x86)\Windows Kits\10\Lib\10.0.26100.0

call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul
if not exist "%BUILD%" mkdir "%BUILD%"

echo Compiling test harness...
cl /nologo /c /O2 /MT /W3 /wd4996 /EHsc /DWIN32 /D_WINDOWS /D_WIN32_WINNT=0x0600 /DWINVER=0x0600 /I"%SRC%" /I"%BC%" /I"%SDKINC%\ucrt" /I"%SDKINC%\um" /I"%SDKINC%\shared" /Fo"%BUILD%\\" "%SRC%\test_graph.cpp" || exit /b 1
echo Linking test_graph.exe...
link /nologo /OUT:"%BUILD%\test_graph.exe" "%BUILD%\test_graph.obj" "%SDKLIB%\um\x64\strmiids.lib" ole32.lib oleaut32.lib user32.lib advapi32.lib winmm.lib || exit /b 1
echo Built: %BUILD%\test_graph.exe

echo Compiling vtable probe...
cl /nologo /c /O2 /MT /W3 /wd4996 /EHsc /DWIN32 /D_WINDOWS /D_WIN32_WINNT=0x0600 /DWINVER=0x0600 /I"%SRC%" /I"%BC%" /I"%SDKINC%\ucrt" /I"%SDKINC%\um" /I"%SDKINC%\shared" /Fo"%BUILD%\\" "%SRC%\probe_vtable.cpp" || exit /b 1
echo Linking probe_vtable.exe...
link /nologo /OUT:"%BUILD%\probe_vtable.exe" "%BUILD%\probe_vtable.obj" "%SDKLIB%\um\x64\strmiids.lib" ole32.lib oleaut32.lib user32.lib advapi32.lib winmm.lib || exit /b 1
echo Built: %BUILD%\probe_vtable.exe

echo Compiling type probe...
cl /nologo /c /O2 /MT /W3 /wd4996 /EHsc /DWIN32 /D_WINDOWS /D_WIN32_WINNT=0x0600 /DWINVER=0x0600 /I"%SRC%" /I"%BC%" /I"%SDKINC%\ucrt" /I"%SDKINC%\um" /I"%SDKINC%\shared" /Fo"%BUILD%\\" "%SRC%\probe_types.cpp" || exit /b 1
echo Linking probe_types.exe...
link /nologo /OUT:"%BUILD%\probe_types.exe" "%BUILD%\probe_types.obj" "%SDKLIB%\um\x64\strmiids.lib" ole32.lib oleaut32.lib user32.lib advapi32.lib winmm.lib || exit /b 1
echo Built: %BUILD%\probe_types.exe

echo Compiling mux probe...
cl /nologo /c /O2 /MT /W3 /wd4996 /EHsc /DWIN32 /D_WINDOWS /D_WIN32_WINNT=0x0600 /DWINVER=0x0600 /I"%SRC%" /I"%BC%" /I"%SDKINC%\ucrt" /I"%SDKINC%\um" /I"%SDKINC%\shared" /Fo"%BUILD%\\" "%SRC%\probe_mux.cpp" || exit /b 1

echo Linking probe_mux.exe...
link /nologo /OUT:"%BUILD%\probe_mux.exe" "%BUILD%\probe_mux.obj" "%SDKLIB%\um\x64\strmiids.lib" ole32.lib oleaut32.lib user32.lib advapi32.lib winmm.lib || exit /b 1

echo Built: %BUILD%\probe_mux.exe

echo Compiling xshow probe...
cl /nologo /c /O2 /MT /W3 /wd4996 /EHsc /DWIN32 /D_WINDOWS /D_WIN32_WINNT=0x0600 /DWINVER=0x0600 /I"%SRC%" /I"%BC%" /I"%SDKINC%\ucrt" /I"%SDKINC%\um" /I"%SDKINC%\shared" /Fo"%BUILD%\\" "%SRC%\probe_xshow.cpp" || exit /b 1

echo Linking probe_xshow.exe...
link /nologo /OUT:"%BUILD%\probe_xshow.exe" "%BUILD%\probe_xshow.obj" "%SDKLIB%\um\x64\strmiids.lib" ole32.lib oleaut32.lib user32.lib advapi32.lib winmm.lib || exit /b 1

echo Built: %BUILD%\probe_xshow.exe
endlocal
