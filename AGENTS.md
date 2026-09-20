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

## Desktop UI design language

Luna Mux is a quiet desktop workbench. New UI should look like it already belongs: reuse the tokens, radii, and component patterns in `app/frontend/src/styles.css` instead of inventing a parallel look.

### Surfaces

The shell sits on `--app-bg`. Workspace interiors sit on `--canvas`. Distinct functional modules — settings groups, file browsers, transfer queue, diagnostic tiles, and similar blocks — are rounded `--surface` cards with `--border` and `--radius-panel`. Dialogs use `--radius-shell`. Buttons, inputs, and small tiles use `--radius-control`.

A page should read as a set of cards on a canvas, with visible gaps between modules. Related content belongs in the same card; different concerns belong in separate cards.

### Type and color

Use the shared text tokens: `--text-strong` for titles, `--text` for body, `--text-muted` for supporting copy. Keep type size consistent within a view so headings, body, and helper copy feel like one scale.

Accent color is for primary actions, focus, and light emphasis. Hover and selection stay quiet: `--surface-hover` and `--selection-bg`. Terminal chrome stays on the `--terminal-*` palette in both app themes.

### Layout

Group a heading with its supporting copy and control. Prefer existing buttons, tabs, list rows, and cards over new chrome. Nearby controls should share size and radius.

### Copy

Keep interface copy short and say what the control does.
