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
  ReadRegStr $0 HKCU "Software\Classes\magnet" "DriftOwned"
  ${If} $0 == "1"
    DeleteRegKey HKCU "Software\Classes\magnet"
  ${EndIf}
!macroend
