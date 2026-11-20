; Nezzel 0.1.0 shipped ffmpeg.exe and ffprobe.exe next to nezzel.exe (about 340 MB).
; From 0.1.1 they live in tools\ffmpeg, so an update over 0.1.0 must remove the old copies.
!macro NSIS_HOOK_PREINSTALL
  Delete "$INSTDIR\ffmpeg.exe"
  Delete "$INSTDIR\ffprobe.exe"
!macroend
