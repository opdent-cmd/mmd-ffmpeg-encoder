Unicode true
!include "MUI2.nsh"

Name "MMD FFmpeg 编码器"
OutFile "..\..\dist\MMDFfmpegEncoder-Setup.exe"
; Install into the user's own folder so the config file (FFmpegEncoder.ini)
; lives next to the app and is removed together with it on uninstall.
InstallDir "$LOCALAPPDATA\MMD Ffmpeg Encoder"
RequestExecutionLevel admin
SetCompressor /SOLID lzma

!define APP_NAME "MMD FFmpeg 编码器"
!define APP_VERSION "1.1.7"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\MMDFfmpegEncoder"

!insertmacro MUI_PAGE_WELCOME
!define MUI_LICENSEPAGE_RADIOBUTTONS
!insertmacro MUI_PAGE_LICENSE "..\LICENSE"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\MMDEncoderConfig.exe"
!define MUI_FINISHPAGE_RUN_TEXT "打开配置界面"
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "SimpChinese"

Section "安装" SecMain
  SetOutPath "$INSTDIR\bin"
  File "..\bin\ffmpeg.exe"

  SetOutPath "$INSTDIR"
  File "..\bin\FFmpegVideoEncoder.dll"
  File /oname=MMDEncoderConfig.exe "..\target\release\mmd_encoder_config.exe"

  ; GPL-3.0 license text and third-party notices travel with the installer.
  File "..\LICENSE"
  File "..\THIRD_PARTY_NOTICES.md"

  ; Default ini next to the filter DLL (fallback when the user never opens UI).
  WriteINIStr "$INSTDIR\FFmpegEncoder.ini" "ffmpeg" "path" "$INSTDIR\bin\ffmpeg.exe"
  WriteINIStr "$INSTDIR\FFmpegEncoder.ini" "ffmpeg" "debug" "0"
  WriteINIStr "$INSTDIR\FFmpegEncoder.ini" "video" "codec" "auto"
  WriteINIStr "$INSTDIR\FFmpegEncoder.ini" "video" "preset" "veryfast"
  WriteINIStr "$INSTDIR\FFmpegEncoder.ini" "video" "crf" "18"
  WriteINIStr "$INSTDIR\FFmpegEncoder.ini" "video" "bitrate" "0"
  WriteINIStr "$INSTDIR\FFmpegEncoder.ini" "video" "rate_mode" "crf"
  WriteINIStr "$INSTDIR\FFmpegEncoder.ini" "video" "extra" ""
  WriteINIStr "$INSTDIR\FFmpegEncoder.ini" "video" "container" "mp4"
  WriteINIStr "$INSTDIR\FFmpegEncoder.ini" "video" "container_path" ""
  WriteINIStr "$INSTDIR\FFmpegEncoder.ini" "video" "alpha_format" ""
  WriteINIStr "$INSTDIR\FFmpegEncoder.ini" "video" "delete_avi" "1"
  WriteINIStr "$INSTDIR\FFmpegEncoder.ini" "video" "merge_audio" "1"

  ; Migrate the previous APPDATA-based config, if present, so settings
  ; survive the move (then the old folder is cleaned up).
  IfFileExists "$APPDATA\MMDFfmpegEncoder\FFmpegEncoder.ini" has_old_config no_old_config
  has_old_config:
    CopyFiles /SILENT "$APPDATA\MMDFfmpegEncoder\FFmpegEncoder.ini" "$INSTDIR\FFmpegEncoder.ini"
    RMDir /r "$APPDATA\MMDFfmpegEncoder"
  no_old_config:

  ; Always point the config at the installed ffmpeg, even after migration.
  WriteINIStr "$INSTDIR\FFmpegEncoder.ini" "ffmpeg" "path" "$INSTDIR\bin\ffmpeg.exe"

  ; Register the 64-bit filter. Sysnative forces the real System32 regsvr32
  ; even though the installer itself is a 32-bit process.
  ExecWait '"$WINDIR\Sysnative\regsvr32.exe" /s "$INSTDIR\FFmpegVideoEncoder.dll"'

  CreateDirectory "$SMPROGRAMS\MMD FFmpeg 编码器"
  CreateShortcut "$SMPROGRAMS\MMD FFmpeg 编码器\配置.lnk" "$INSTDIR\MMDEncoderConfig.exe"
  CreateShortcut "$SMPROGRAMS\MMD FFmpeg 编码器\卸载.lnk" "$INSTDIR\uninstall.exe"

  ; Uninstaller + Add/Remove Programs entry (64-bit registry view).
  WriteUninstaller "$INSTDIR\uninstall.exe"
  SetRegView 64
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayName" "${APP_NAME}"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayVersion" "${APP_VERSION}"
  WriteRegStr HKLM "${UNINST_KEY}" "Publisher" "Codex"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\MMDEncoderConfig.exe"
  WriteRegStr HKLM "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegDWORD HKLM "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKLM "${UNINST_KEY}" "NoRepair" 1
SectionEnd

Section "Uninstall"
  ExecWait '"$WINDIR\Sysnative\regsvr32.exe" /u /s "$INSTDIR\FFmpegVideoEncoder.dll"'

  Delete "$SMPROGRAMS\MMD FFmpeg 编码器\配置.lnk"
  Delete "$SMPROGRAMS\MMD FFmpeg 编码器\卸载.lnk"
  RMDir "$SMPROGRAMS\MMD FFmpeg 编码器"

  ; Remove everything, including the user config file, and any legacy
  ; APPDATA folder from older versions.
  RMDir /r "$INSTDIR"
  RMDir /r "$APPDATA\MMDFfmpegEncoder"

  SetRegView 64
  DeleteRegKey HKLM "${UNINST_KEY}"
SectionEnd
