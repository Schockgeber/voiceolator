; Voiceolator installer (NSIS 3, Modern UI 2). Per-user install: no admin rights needed.
; Build: makensis -DVERSION=0.11.0 -DSRCDIR=<staged app folder> -DOUTFILE=<setup.exe> voiceolator.nsi

Unicode true
ManifestDPIAware true
SetCompressor /SOLID lzma
RequestExecutionLevel user

!ifndef VERSION
  !define VERSION "0.0.0"
!endif
!define APPNAME "Voiceolator"
!define UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APPNAME}"

Name "${APPNAME} ${VERSION}"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\${APPNAME}"
InstallDirRegKey HKCU "Software\${APPNAME}" "InstallDir"
BrandingText "${APPNAME} ${VERSION}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${APPNAME}"
VIAddVersionKey "FileDescription" "${APPNAME} installer"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "MIT OR Apache-2.0"

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"
!include "WinVer.nsh"

!define MUI_ICON "${SRCDIR}\icon.ico"
!define MUI_UNICON "${SRCDIR}\icon.ico"
!define MUI_ABORTWARNING
!define MUI_WELCOMEPAGE_TEXT "Voiceolator plays only the streamer's voice from a Twitch stream (or any app) to your headphones, removing background music, game sounds and alerts in real time. Your game's sound stays untouched.$\r$\n$\r$\nEverything runs locally on your PC. No account, no internet connection.$\r$\n$\r$\nClick Next to continue."
!define MUI_FINISHPAGE_RUN "$INSTDIR\Voiceolator.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Start Voiceolator"
!define MUI_FINISHPAGE_SHOWREADME "$INSTDIR\README.html"
!define MUI_FINISHPAGE_SHOWREADME_TEXT "Show the user guide"
!define MUI_FINISHPAGE_SHOWREADME_NOTCHECKED

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${SRCDIR}\LICENSE.txt"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "Voiceolator requires 64-bit Windows 10 (version 2004) or Windows 11." /SD IDOK
    Abort
  ${EndIf}
  ${IfNot} ${AtLeastWin10}
    MessageBox MB_ICONSTOP "Voiceolator requires Windows 10 (version 2004) or Windows 11." /SD IDOK
    Abort
  ${EndIf}
  ; Process loopback capture needs build 19041 (Windows 10 2004) or newer.
  ReadRegStr $0 HKLM "SOFTWARE\Microsoft\Windows NT\CurrentVersion" "CurrentBuildNumber"
  ${If} $0 < 19041
    MessageBox MB_ICONEXCLAMATION|MB_YESNO "Your Windows build ($0) is older than 19041 (Windows 10 version 2004). Capturing a single app will not work until you update Windows.$\r$\n$\r$\nInstall anyway?" /SD IDYES IDYES +2
    Abort
  ${EndIf}
FunctionEnd

Function CloseRunning
  FindWindow $0 "" "Voiceolator"
  ${If} $0 <> 0
    ; It may be hidden in the tray: ask it to quit (Voiceolator --quit), then check again.
    ${If} ${FileExists} "$INSTDIR\Voiceolator.exe"
      ExecWait '"$INSTDIR\Voiceolator.exe" --quit'
      Sleep 1500
    ${EndIf}
    FindWindow $0 "" "Voiceolator"
    ${If} $0 <> 0
      MessageBox MB_ICONINFORMATION|MB_OKCANCEL "Voiceolator is still running. Quit it (right-click its tray icon → Quit Voiceolator), then click OK." /SD IDOK IDOK +2
      Abort
    ${EndIf}
  ${EndIf}
FunctionEnd

; "Isolate the voice with Voiceolator" in the Explorer right-click menu of media files
!macro VoMenu EXT
  WriteRegStr HKCU "Software\Classes\SystemFileAssociations\.${EXT}\shell\Voiceolator" "" "Isolate the voice with Voiceolator"
  WriteRegStr HKCU "Software\Classes\SystemFileAssociations\.${EXT}\shell\Voiceolator" "Icon" "$INSTDIR\Voiceolator.exe"
  WriteRegStr HKCU "Software\Classes\SystemFileAssociations\.${EXT}\shell\Voiceolator\command" "" '"$INSTDIR\Voiceolator.exe" "%1"'
!macroend
!macro VoUnMenu EXT
  DeleteRegKey HKCU "Software\Classes\SystemFileAssociations\.${EXT}\shell\Voiceolator"
!macroend
!macro VoForEach MACRO
  !insertmacro ${MACRO} mp4
  !insertmacro ${MACRO} mkv
  !insertmacro ${MACRO} webm
  !insertmacro ${MACRO} mov
  !insertmacro ${MACRO} m4v
  !insertmacro ${MACRO} avi
  !insertmacro ${MACRO} flv
  !insertmacro ${MACRO} ts
  !insertmacro ${MACRO} mp3
  !insertmacro ${MACRO} m4a
  !insertmacro ${MACRO} aac
  !insertmacro ${MACRO} wav
  !insertmacro ${MACRO} flac
  !insertmacro ${MACRO} ogg
  !insertmacro ${MACRO} opus
!macroend

Section "Voiceolator (required)" SecMain
  SectionIn RO
  Call CloseRunning
  SetOutPath "$INSTDIR"
  File "${SRCDIR}\Voiceolator.exe"
  File "${SRCDIR}\onnxruntime.dll"
  File "${SRCDIR}\README.html"
  File "${SRCDIR}\LICENSE.txt"
  File "${SRCDIR}\THIRD_PARTY_LICENSES.txt"
  File "${SRCDIR}\LICENSE-onnxruntime.txt"
  File "${SRCDIR}\icon.ico"
  SetOutPath "$INSTDIR\models"
  File "${SRCDIR}\models\dpdfnet2_48khz_hr.onnx"
  SetOutPath "$INSTDIR"

  CreateDirectory "$SMPROGRAMS\${APPNAME}"
  CreateShortcut "$SMPROGRAMS\${APPNAME}\${APPNAME}.lnk" "$INSTDIR\Voiceolator.exe" "" "$INSTDIR\Voiceolator.exe" 0
  CreateShortcut "$SMPROGRAMS\${APPNAME}\User guide.lnk" "$INSTDIR\README.html"
  CreateShortcut "$SMPROGRAMS\${APPNAME}\Uninstall ${APPNAME}.lnk" "$INSTDIR\Uninstall.exe"

  WriteUninstaller "$INSTDIR\Uninstall.exe"
  WriteRegStr HKCU "Software\${APPNAME}" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayName" "${APPNAME}"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTKEY}" "Publisher" "Voiceolator contributors"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayIcon" "$INSTDIR\Voiceolator.exe"
  WriteRegStr HKCU "${UNINSTKEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTKEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoRepair" 1
  WriteRegDWORD HKCU "${UNINSTKEY}" "EstimatedSize" 90000

  ; The Quality engine (ONNX Runtime) needs the VC++ 2015-2022 x64 runtime.
  SetRegView 64
  ReadRegDWORD $1 HKLM "SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64" "Installed"
  SetRegView lastused
  ${If} $1 != 1
  ${AndIfNot} ${FileExists} "$SYSDIR\vcruntime140_1.dll"
    MessageBox MB_ICONINFORMATION|MB_YESNO "For the best voice quality Voiceolator needs the free Microsoft Visual C++ Redistributable (x64), which is not installed.$\r$\n$\r$\nWithout it Voiceolator still works with its Light engine.$\r$\n$\r$\nOpen the official Microsoft download now?" /SD IDNO IDNO +2
    ExecShell "open" "https://aka.ms/vs/17/release/vc_redist.x64.exe"
  ${EndIf}
SectionEnd

Section "Desktop shortcut" SecDesktop
  CreateShortcut "$DESKTOP\${APPNAME}.lnk" "$INSTDIR\Voiceolator.exe" "" "$INSTDIR\Voiceolator.exe" 0
SectionEnd

Section "Explorer menu for audio and video files" SecMenu
  !insertmacro VoForEach VoMenu
SectionEnd

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecMain} "The Voiceolator app, voice models and command-line tool."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecDesktop} "Put a Voiceolator icon on the desktop."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecMenu} "Right-click an audio or video file → Isolate the voice with Voiceolator."
!insertmacro MUI_FUNCTION_DESCRIPTION_END

Section "Uninstall"
  FindWindow $0 "" "Voiceolator"
  ${If} $0 <> 0
    ExecWait '"$INSTDIR\Voiceolator.exe" --quit'
    Sleep 1500
    FindWindow $0 "" "Voiceolator"
  ${EndIf}
  ${If} $0 <> 0
    MessageBox MB_ICONINFORMATION "Please quit Voiceolator first (right-click its tray icon → Quit Voiceolator), then run the uninstaller again." /SD IDOK
    Abort
  ${EndIf}
  !insertmacro VoForEach VoUnMenu
  Delete "$INSTDIR\Voiceolator.exe"
  Delete "$INSTDIR\voiceolator-cli.exe" ; optional, if the user added it
  Delete "$INSTDIR\onnxruntime.dll"
  Delete "$INSTDIR\README.html"
  Delete "$INSTDIR\LICENSE.txt"
  Delete "$INSTDIR\THIRD_PARTY_LICENSES.txt"
  Delete "$INSTDIR\LICENSE-onnxruntime.txt"
  Delete "$INSTDIR\icon.ico"
  Delete "$INSTDIR\models\dpdfnet2_48khz_hr.onnx"
  RMDir "$INSTDIR\models"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${APPNAME}\${APPNAME}.lnk"
  Delete "$SMPROGRAMS\${APPNAME}\User guide.lnk"
  Delete "$SMPROGRAMS\${APPNAME}\Uninstall ${APPNAME}.lnk"
  RMDir "$SMPROGRAMS\${APPNAME}"
  Delete "$DESKTOP\${APPNAME}.lnk"
  ; logs and a downloaded ffmpeg are removed; settings in %APPDATA%\Voiceolator are kept
  RMDir /r "$LOCALAPPDATA\${APPNAME}"
  DeleteRegKey HKCU "${UNINSTKEY}"
  DeleteRegKey HKCU "Software\${APPNAME}"
SectionEnd
