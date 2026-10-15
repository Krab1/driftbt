; Ends a running Drift before the installer's own check. That check (Windows Restart Manager) can fail with
; "Failed to kill Drift" when the app is hiding in the tray, and a forced end loses nothing: the library is saved
; every few seconds and downloads resume on the next start. Matches the exact file name, so the installer
; (Drift_x.y.z_x64-setup.exe) is never hit.
!macro DriftStopApp
  nsExec::Exec '"$SYSDIR\taskkill.exe" /F /IM "${MAINBINARYNAME}.exe"'
  Pop $0
  Sleep 800
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro DriftStopApp
!macroend

; Registers Drift as a handler for magnet: links. Per-user (HKCU), and only when nothing else has claimed the
; scheme, so installing never takes a link type away from the user's current client.
!macro NSIS_HOOK_POSTINSTALL
  ReadRegStr $0 HKCU "Software\Classes\magnet" ""
  ${If} $0 == ""
    WriteRegStr HKCU "Software\Classes\magnet" "" "URL:Magnet link"
    WriteRegStr HKCU "Software\Classes\magnet" "URL Protocol" ""
    WriteRegStr HKCU "Software\Classes\magnet" "DriftOwned" "1"
    WriteRegStr HKCU "Software\Classes\magnet\DefaultIcon" "" "$INSTDIR\${MAINBINARYNAME}.exe,0"
    WriteRegStr HKCU "Software\Classes\magnet\shell\open\command" "" '"$INSTDIR\${MAINBINARYNAME}.exe" "%1"'
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro DriftStopApp
  ReadRegStr $0 HKCU "Software\Classes\magnet" "DriftOwned"
  ${If} $0 == "1"
    DeleteRegKey HKCU "Software\Classes\magnet"
  ${EndIf}
!macroend
