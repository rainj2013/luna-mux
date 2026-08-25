!macro RejectRunningLunaMux
  ; Refuse to start an install or uninstall while any Luna Mux process is
  ; running. The generated Tauri installer hook below normally offers to kill
  ; the process; doing that can also tear down the user's terminal sessions.
  !if "${INSTALLMODE}" == "currentUser"
    nsis_tauri_utils::FindProcessCurrentUser "${MAINBINARYNAME}.exe"
  !else
    nsis_tauri_utils::FindProcess "${MAINBINARYNAME}.exe"
  !endif
  Pop $R0
  ${If} $R0 = 0
    MessageBox MB_ICONSTOP|MB_OK "Luna Mux is currently running. Please completely close it, then run the installer again." /SD IDOK
    Quit
  ${EndIf}
!macroend

!macro StopOrphanedAgentBrowser
  ${If} ${FileExists} "$INSTDIR\agent-browser.exe"
    ${If} ${RunningX64}
      ; NSIS is 32-bit, so Sysnative is required to inspect a 64-bit process path.
      StrCpy $R3 "$WINDIR\Sysnative\WindowsPowerShell\v1.0\powershell.exe"
    ${Else}
      StrCpy $R3 "$SYSDIR\WindowsPowerShell\v1.0\powershell.exe"
    ${EndIf}
    nsExec::ExecToStack `"$R3" -NoLogo -NoProfile -NonInteractive -WindowStyle Hidden -Command "& { param([string]$$target); $$target = ((@($$target) + @($$args)) -join ' '); $$target = [IO.Path]::GetFullPath($$target); for ($$attempt = 0; $$attempt -lt 8; $$attempt++) { $$running = @(Get-Process -Name agent-browser -ErrorAction SilentlyContinue | Where-Object { try { [IO.Path]::GetFullPath($$_.Path) -eq $$target } catch { $$false } }); if ($$running.Count -eq 0) { exit 0 }; foreach ($$process in $$running) { Stop-Process -Id $$process.Id -Force -ErrorAction SilentlyContinue }; Start-Sleep -Milliseconds 250 }; exit 1 }" "$INSTDIR\agent-browser.exe"`
    Pop $R0
    Pop $R1

    StrCpy $R2 0
    ${Do}
      ClearErrors
      Delete "$INSTDIR\agent-browser.exe"
      ${IfNot} ${Errors}
        ${ExitDo}
      ${EndIf}
      IntOp $R2 $R2 + 1
      ${If} $R2 >= 8
        MessageBox MB_ICONSTOP|MB_OK "Luna Mux could not stop its browser automation process. Close Luna Mux and retry the installer." /SD IDOK
        Quit
      ${EndIf}
      Sleep 250
    ${Loop}
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro RejectRunningLunaMux
  !insertmacro StopOrphanedAgentBrowser
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro RejectRunningLunaMux
  !insertmacro StopOrphanedAgentBrowser
!macroend
