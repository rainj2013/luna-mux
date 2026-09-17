# Luna Mux Agent Rules

## AI-assisted development workflow

- Use the fast path for documentation, styling, or a localized change that does not alter shared state, contracts, persistence, security, process lifecycle, or platform behavior: inspect the relevant code, make the smallest change, run the narrowest relevant check, and review the diff. The full workflow document is not required.
- For cross-module changes, or changes to Tauri commands, Rust services, shared state, contracts, persistence, MCP/Hook, security, process lifecycle, terminal/PTY, browser runtime, WSL, or SSH, read and follow `docs/AI_DEVELOPMENT_GUIDELINES.md` before editing.
- If a fast-path task expands into one of those areas, stop and switch to the full workflow.

## Local Node.js toolchain

- On all development platforms, including Windows and macOS, this repository's Node.js installation is managed by `fnm` and the version is selected by `.node-version`.
- Non-interactive shells may not have the fnm environment on `PATH`. Before running `node`, `npm`, or project scripts, initialize it in the same shell command. Use `fnm env --use-on-cd --shell powershell | Out-String | Invoke-Expression` in PowerShell, or `eval "$(fnm env --use-on-cd --shell zsh)"` in macOS zsh.
- Do not conclude that Node.js is unavailable, install another Node.js runtime, or switch package managers before attempting the fnm initialization above. Use `npm` and the checked-in `package-lock.json` for dependencies and scripts unless the user explicitly requests otherwise.

## Cross-platform terminal compatibility

- When changing terminal, PTY, agent launch/injection, hook/MCP, browser runtime, WSL, or SSH remote features, keep behavior compatible across macOS, Windows PowerShell 5.1, PowerShell 7, WSL local terminals, and SSH remote terminals.
- Do not assume stdin/stdout/EOF, signals, PTY dimensions, PATH resolution, or command quoting behave the same across shells and platforms. In particular, Windows hook child processes may keep stdin open; parse a complete JSON value or line instead of blocking with `read_to_end`.
- When capturing subprocess output, prefer files or bounded reads over `read_to_string` on inherited pipes: on Windows a grandchild can keep the pipe handle open after the direct child exits, making the read wait forever.
- Reuse existing target-specific helpers such as `local_pty_backend::is_powershell_target`, WSL `/mnt/<drive>/...` path conversion, and SSH/remote branches instead of adding one-shell-only special cases.
- After such changes, run the relevant native tests and `npm run typecheck`, and verify the affected local terminals (PowerShell 5.1, PowerShell 7, WSL) when available. For macOS or SSH targets that cannot be verified locally, state the remaining risk explicitly.
