; Nezzel 0.1.0 had no publisher, so its installer kept the install folder under
; HKCU\Software\ahmed\Nezzel. From 0.1.1 the publisher is Hululsoft and the installer reads
; HKCU\Software\Hululsoft\Nezzel. Without the old value the "uninstall before installing" step
; starts the 0.1.0 uninstaller with an empty folder and fails ("Unable to uninstall!").
; Copy the old value over before any page shows.
!define MUI_CUSTOMFUNCTION_GUIINIT NezzelAdoptOldInstallKey
Function NezzelAdoptOldInstallKey
  Push $R7
  ReadRegStr $R7 HKCU "Software\Hululsoft\Nezzel" ""
  StrCmp $R7 "" 0 adopt_done
  ReadRegStr $R7 HKCU "Software\ahmed\Nezzel" ""
  StrCmp $R7 "" adopt_done
  WriteRegStr HKCU "Software\Hululsoft\Nezzel" "" $R7
  StrCpy $INSTDIR $R7
  adopt_done:
  Pop $R7
FunctionEnd

; Nezzel 0.1.0 shipped ffmpeg.exe and ffprobe.exe next to nezzel.exe (about 340 MB).
; From 0.1.1 they live in tools\ffmpeg, so an update over 0.1.0 must remove the old copies.
!macro NSIS_HOOK_PREINSTALL
  Delete "$INSTDIR\ffmpeg.exe"
  Delete "$INSTDIR\ffprobe.exe"
!macroend

; The 0.1.0 key is not used any more once 0.1.1 is installed.
!macro NSIS_HOOK_POSTINSTALL
  DeleteRegKey HKCU "Software\ahmed\Nezzel"
  DeleteRegKey /ifempty HKCU "Software\ahmed"
!macroend
