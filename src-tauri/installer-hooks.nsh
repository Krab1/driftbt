; Ends a running Drift before the installer's own check. That check (Windows Restart Manager) aborts with
; "Failed to kill Drift" for as long as Windows still lists a drift.exe, and a killed Drift stays listed for a few
; seconds (its WebView2 helper processes are still shutting down and hold on to it). So after ending it we keep asking
; for the process list until drift.exe is really gone. A forced end loses nothing: the library is saved every few
; seconds and downloads resume on the next start. Matches the exact file name, so the installer is never hit, and
; never uses /T because an update installer may itself be a child of the app that is exiting.
!macro DriftStopApp
  !define DriftUid ${__LINE__}
  nsExec::Exec '"$SYSDIR\taskkill.exe" /F /IM "${MAINBINARYNAME}.exe"'
  Pop $0
  StrCpy $1 0
  drift_wait_${DriftUid}:
    ; find exits with 0 when it saw the name, i.e. the process is still listed.
    nsExec::Exec '"$SYSDIR\cmd.exe" /C ""$SYSDIR\tasklist.exe" /FI "IMAGENAME eq ${MAINBINARYNAME}.exe" /NH | "$SYSDIR\find.exe" /I "${MAINBINARYNAME}.exe""'
    Pop $0
    ${If} $0 == 0
      IntOp $1 $1 + 1
      ${If} $1 < 150
        Sleep 200
        Goto drift_wait_${DriftUid}
      ${EndIf}
    ${EndIf}
  ; Then until the exe itself can be opened for writing (an antivirus scan or the shell may still have it for a moment).
  StrCpy $1 0
  drift_file_${DriftUid}:
    IfFileExists "$INSTDIR\${MAINBINARYNAME}.exe" 0 drift_done_${DriftUid}
    ClearErrors
    FileOpen $2 "$INSTDIR\${MAINBINARYNAME}.exe" a
    IfErrors 0 drift_close_${DriftUid}
    IntOp $1 $1 + 1
    ${If} $1 < 100
      Sleep 200
      Goto drift_file_${DriftUid}
    ${EndIf}
    Goto drift_done_${DriftUid}
  drift_close_${DriftUid}:
    FileClose $2
  drift_done_${DriftUid}:
  Sleep 500
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
