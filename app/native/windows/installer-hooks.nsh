!macro StopInstalledLunaMuxProcesses
  !insertmacro CheckIfAppIsRunning "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"

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
