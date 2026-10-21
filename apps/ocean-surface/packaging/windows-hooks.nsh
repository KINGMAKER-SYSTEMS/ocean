; Keep the installer's force-termination path away from Ocean's owned daemon.
; Replace the pinned Tauri template macro before either install/uninstall expands it.
; Never kill a process from an installer: explicit app Quit owns that decision.

!macro OceanRequireWritableBinary name
  ${If} ${FileExists} "$INSTDIR\${name}"
    Push $0
    ; OPEN_EXISTING, GENERIC_WRITE, share read/write/delete; no bytes are written.
    ; Windows refuses write access to a mapped executable or a locked file.
    System::Call 'kernel32::CreateFileW(w "$INSTDIR\${name}", i 0x40000000, i 7, p 0, i 3, i 128, p 0) p.r0'
    ${If} $0 = -1
      Pop $0
      IfSilent +2
        MessageBox MB_OK|MB_ICONEXCLAMATION "Ocean's files are in use or unavailable. Close Ocean and connected agent tools, then retry."
      SetErrorLevel 1618
      Abort
    ${EndIf}
    System::Call 'kernel32::CloseHandle(p r0)'
    Pop $0
  ${EndIf}
!macroend

!ifmacrondef CheckIfAppIsRunning
  !error "Unsupported Tauri NSIS process-check template"
!endif
!macroundef CheckIfAppIsRunning
!macro CheckIfAppIsRunning executableName productName
  Push $R0
  nsis_tauri_utils::FindProcessCurrentUser "${executableName}"
  Pop $R0
  ${If} $R0 = 0
    Pop $R0
    IfSilent +2
      MessageBox MB_OK|MB_ICONEXCLAMATION "Quit Ocean before updating or removing it."
    SetErrorLevel 1618
    Abort
  ${EndIf}
  Pop $R0
  !insertmacro OceanRequireWritableBinary "${MAINBINARYNAME}.exe"
  !insertmacro OceanRequireWritableBinary "ocean-daemon.exe"
  !insertmacro OceanRequireWritableBinary "ocean-mcp.exe"
!macroend

