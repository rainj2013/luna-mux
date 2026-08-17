!macro StopInstalledLunaMuxProcesses
  !insertmacro CheckIfAppIsRunning "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"

  IfFileExists "$INSTDIR\agent-browser.exe" 0 luna_mux_sidecar_done
    nsExec::ExecToStack `"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoLogo -NoProfile -NonInteractive -WindowStyle Hidden -Command "& { param([string]$$target); $$target = ((@($$target) + @($$args)) -join ' '); $$target = [IO.Path]::GetFullPath($$target); Get-Process -Name agent-browser -ErrorAction SilentlyContinue | Where-Object { try { [IO.Path]::GetFullPath($$_.Path) -eq $$target } catch { $$false } } | Stop-Process -Force -ErrorAction Stop }" "$INSTDIR\agent-browser.exe"`
    Pop $R0
    Pop $R1
    Sleep 750

    ClearErrors
    Delete "$INSTDIR\agent-browser.exe"
    ${If} ${Errors}
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
