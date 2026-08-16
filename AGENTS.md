# Luna Mux Agent Rules

## Cross-platform terminal compatibility

- When changing terminal, PTY, agent launch/injection, hook/MCP, browser runtime, WSL, or SSH remote features, keep behavior compatible across macOS, Windows PowerShell 5.1, PowerShell 7, WSL local terminals, and SSH remote terminals.
- Do not assume stdin/stdout/EOF, signals, PTY dimensions, PATH resolution, or command quoting behave the same across shells and platforms. In particular, Windows hook child processes may keep stdin open; parse a complete JSON value or line instead of blocking with `read_to_end`.
- When capturing subprocess output, prefer files or bounded reads over `read_to_string` on inherited pipes: on Windows a grandchild can keep the pipe handle open after the direct child exits, making the read wait forever.
- Reuse existing target-specific helpers such as `local_pty_backend::is_powershell_target`, WSL `/mnt/<drive>/...` path conversion, and SSH/remote branches instead of adding one-shell-only special cases.
- After such changes, run the relevant native tests and `npm run typecheck`, and verify the affected local terminals (PowerShell 5.1, PowerShell 7, WSL) when available. For macOS or SSH targets that cannot be verified locally, state the remaining risk explicitly.
