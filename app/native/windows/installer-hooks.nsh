!macro StopInstalledLunaMuxProcesses
  ; Tauri's CheckIfAppIsRunning macro terminates only the main executable. Capture
  ; matching main-process PIDs first so we can clean up descendants (PowerShell,
  ; ConPTY and Codex) after the normal installer prompt/termination has completed.
  StrCpy $R4 "$PLUGINSDIR\luna-mux-process-tree.txt"
  ${If} ${RunningX64}
    ; NSIS is 32-bit, so Sysnative is required to inspect a 64-bit process path.
    StrCpy $R3 "$WINDIR\Sysnative\WindowsPowerShell\v1.0\powershell.exe"
  ${Else}
    StrCpy $R3 "$SYSDIR\WindowsPowerShell\v1.0\powershell.exe"
  ${EndIf}
  ; Keep the PowerShell argument free of nested double quotes.  nsExec passes
  ; this whole command through Windows' command-line parser before PowerShell
  ; sees it; [Environment]::NewLine has the same result as "`n" without
  ; prematurely ending the quoted -Command argument.
  nsExec::ExecToStack `"$R3" -NoLogo -NoProfile -NonInteractive -WindowStyle Hidden -Command "& { param([string]$$target,[string]$$pidFile); $$target = [IO.Path]::GetFullPath($$target); $$roots = @(Get-CimInstance Win32_Process -ErrorAction SilentlyContinue | Where-Object { try { $$_.ExecutablePath -and [String]::Equals([string]$$_.ExecutablePath,$$target,[StringComparison]::OrdinalIgnoreCase) } catch { $$false } } | Select-Object -ExpandProperty ProcessId); if ($$roots.Count -gt 0) { Set-Content -LiteralPath $$pidFile -Value ($$roots -join [Environment]::NewLine) -Encoding ASCII } else { Remove-Item -LiteralPath $$pidFile -Force -ErrorAction SilentlyContinue } }" "$INSTDIR\${MAINBINARYNAME}.exe" "$R4"`
  Pop $R0
  Pop $R1

  ; Give the user a chance to close the old app cleanly before any forced
  ; termination.  The prompt is only shown when a matching process was found.
  IfFileExists "$R4" luna_mux_running_app luna_mux_no_running_app
    MessageBox MB_ICONEXCLAMATION|MB_OKCANCEL "Luna Mux is still running. Please completely close the old version before installing the new version. Click OK to continue; if it is still running, the installer will terminate it and its child processes. Click Cancel to stop the installation." IDOK luna_mux_continue IDCANCEL luna_mux_cancel
  luna_mux_cancel:
    Abort
  luna_mux_continue:
  luna_mux_no_running_app:

  !insertmacro CheckIfAppIsRunning "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"

  ; CheckIfAppIsRunning uses the same NSIS scratch registers, including $R3.
  ; Restore the PowerShell path before running the descendant cleanup.
  ${If} ${RunningX64}
    StrCpy $R3 "$WINDIR\Sysnative\WindowsPowerShell\v1.0\powershell.exe"
  ${Else}
    StrCpy $R3 "$SYSDIR\WindowsPowerShell\v1.0\powershell.exe"
  ${EndIf}

  ; The main process may already be gone by now, while its children can remain
  ; alive. Resolve descendants from the captured root PID(s) and terminate only
  ; that process tree. Re-scan several times because a shell can briefly spawn a
  ; child while it is being torn down.
  nsExec::ExecToStack `"$R3" -NoLogo -NoProfile -NonInteractive -WindowStyle Hidden -Command "& { param([string]$$pidFile); if (!(Test-Path -LiteralPath $$pidFile)) { exit 0 }; for ($$attempt = 0; $$attempt -lt 8; $$attempt++) { $$records = @(Get-CimInstance Win32_Process -ErrorAction SilentlyContinue); $$queue = @(); $$roots = @(Get-Content -LiteralPath $$pidFile -ErrorAction SilentlyContinue | ForEach-Object { $$line = $$_.Trim(); if ($$line -match '^[1-9][0-9]*$$') { [uint32]$$line } }); $$queue += $$roots; $$descendants = @(); while ($$queue.Count -gt 0) { $$parent = [uint32]$$queue[0]; if ($$queue.Count -eq 1) { $$queue = @() } else { $$queue = $$queue[1..($$queue.Count - 1)] }; foreach ($$child in ($$records | Where-Object { [uint32]$$_.ParentProcessId -eq $$parent })) { $$childId = [uint32]$$child.ProcessId; if (!($$descendants -contains $$childId) -and !($$roots -contains $$childId)) { $$descendants += $$childId; $$queue += $$childId } } }; if ($$descendants.Count -eq 0) { break }; foreach ($$processId in $$descendants) { Stop-Process -Id $$processId -Force -ErrorAction SilentlyContinue }; Start-Sleep -Milliseconds 250 }; Remove-Item -LiteralPath $$pidFile -Force -ErrorAction SilentlyContinue }" "$R4"`
  Pop $R0
  Pop $R1

  IfFileExists "$INSTDIR\agent-browser.exe" 0 luna_mux_sidecar_done
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
  luna_mux_delete_sidecar:
    ClearErrors
    Delete "$INSTDIR\agent-browser.exe"
    ${If} ${Errors}
      IntOp $R2 $R2 + 1
      ${If} $R2 < 8
        Sleep 250
        Goto luna_mux_delete_sidecar
      ${EndIf}
      MessageBox MB_ICONSTOP|MB_OK "Luna Mux could not stop its browser automation process. Close Luna Mux and retry the installer." /SD IDOK
      Abort
    ${EndIf}
  luna_mux_sidecar_done:
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro StopInstalledLunaMuxProcesses
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro StopInstalledLunaMuxProcesses
!macroend
