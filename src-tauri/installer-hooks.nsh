; Ends a running Drift before the installer's own check. That check (Windows Restart Manager) can fail with
; "Failed to kill Drift" when the app is hiding in the tray, and a forced end loses nothing: the library is saved
; every few seconds and downloads resume on the next start. Matches the exact file name, so the installer
; (Drift_x.y.z_x64-setup.exe) is never hit.
!macro DriftStopApp
  !define DriftUid ${__LINE__}
  nsExec::Exec '"$SYSDIR\taskkill.exe" /F /IM "${MAINBINARYNAME}.exe"'
  Pop $0
  ; A killed process lingers briefly and still counts as "running" to the installer's own check, which then
  ; aborts with "Failed to kill". Wait until the exe can be opened for writing (nothing has it) or ~10 s pass.
  StrCpy $1 0
  drift_wait_${DriftUid}:
    IfFileExists "$INSTDIR\${MAINBINARYNAME}.exe" 0 drift_done_${DriftUid}
    ClearErrors
    FileOpen $2 "$INSTDIR\${MAINBINARYNAME}.exe" a
    IfErrors 0 drift_close_${DriftUid}
    IntOp $1 $1 + 1
    ${If} $1 < 50
      Sleep 200
      Goto drift_wait_${DriftUid}
    ${EndIf}
    Goto drift_done_${DriftUid}
  drift_close_${DriftUid}:
    FileClose $2
  drift_done_${DriftUid}:
  Sleep 800
  !undef DriftUid
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
