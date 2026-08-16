<div align="center">
  <img src="assets/icons/luna.png" width="96" alt="Luna Mux icon">
  <h1>Luna Mux</h1>
  <p>A local and remote terminal workspace built for coding agents</p>
  <p>
    <a href="https://github.com/rainj2013/luna-mux/actions/workflows/release.yml"><img src="https://github.com/rainj2013/luna-mux/actions/workflows/release.yml/badge.svg" alt="Build status"></a>
    <img src="https://img.shields.io/badge/platform-macOS%20%7C%20Windows-5b8def?style=flat-square" alt="Supported platforms">
    <a href="LICENSE"><img src="https://img.shields.io/github/license/rainj2013/luna-mux?style=flat-square" alt="MIT License"></a>
  </p>
  <p>
    <a href="#quick-start">Quick start</a>
    ·
    <a href="docs/DEVELOPMENT.md">Development guide</a>
    ·
    <a href="docs/LUNA_MUX_DESIGN.md">Design</a>
  </p>
</div>

<div align="center">English · <a href="README.md">简体中文</a></div>

Luna Mux brings project terminals, coding agents, remote machines, and managed browsers into one workspace. A Session persists its project root, local or SSH Panes, and recursive split layout. Codex, Claude Code, and other tools still run in ordinary terminals, while supported agents gain status notifications, browser tooling, and narrowly scoped control over Luna Mux itself.

> [!IMPORTANT]
> Luna Mux is currently a **0.1.0 preview**. macOS is used continuously for development and has completed real agent, SSH, and browser tasks. The core Windows paths are implemented and have been tested on real hardware, while full WSL coverage, some Claude Code scenarios, and cross-platform release regression are still in progress. Do not treat it as unattended production infrastructure yet.

## Highlights

### Project Sessions and terminal Panes

- A Session represents one project context and persists its project root, Pane definitions, and recursive split tree.
- Layout controls include horizontal and vertical splits, draggable ratios, presets, rename, maximize, and restore.
- macOS uses a local zsh/bash PTY. Windows supports PowerShell 7 and WSL. Both platforms can create SSH terminals.
- Local and remote terminals share one xterm.js UI, including search, clipboard handling, themes, fonts, backgrounds, and output flow control.
- Restarting the application restores Session, Pane, and layout definitions without reconnecting hosts or launching processes behind the user's back.

### Coding agents in ordinary terminals

There is no special “Agent Pane.” Run `codex` or `claude` manually in any terminal, or use a built-in launch shortcut. Supported agents are configured at process scope, without rewriting the user's global Codex or Claude Code configuration.

- Unified working, waiting-for-input, waiting-for-permission, completed, and error states.
- Attention indicators in the sidebar and Pane border, plus desktop notifications that route back to the owning Session and Pane.
- An Agent Environment view for inspecting the real Adapter, Hook, Luna MCP, and Browser MCP health state.
- Managed terminals, agents, and Chrome processes close with Luna Mux.

### Let agents control Luna Mux

Luna MCP gives agents a constrained way to understand and operate Luna Mux resources instead of mistaking application operations for browser tasks.

Within their current Session, agents can:

- discover Sessions, Panes, Terminal Runtimes, and other managed agents;
- create Panes, update layouts, read bounded terminal output, and write terminal input;
- inspect agent state, send tasks, and interrupt managed agents;
- read safe connection summaries and update themes or terminal appearance;
- inspect transfers, tunnels, and control events.

The Session is both the collaboration and authorization boundary. Agents cannot access other Sessions by default. Important side effects such as closing Runtimes or starting transfers and tunnels may still require desktop approval. Credentials, private-key contents, and AI keys are never exposed through MCP.

### Session-scoped managed browsers

Each Session can own one managed Chrome resource. Chrome runs as a standard external window with an isolated persistent profile, so the user can take over at any time.

- Browsers are not terminal Panes and do not participate in split layouts.
- Agents use a pinned native `agent-browser` MCP for snapshots, interaction, waits, tabs, screenshots, console inspection, network requests, and HAR capture.
- Chrome starts lazily on the first real browser tool call, then the same Runtime and page are reused.
- Remote SSH agents reach the desktop's managed Chrome through an authenticated proxy; raw CDP is not exposed to the remote host.
- Luna Mux operations, Web content, and ordinary source/Shell/Git work use separate tool domains, reducing errors such as interpreting a Pane request as a browser-tab request.

### SSH, SFTP, and port forwarding

- SSH supports passwords, private keys, SSH Agent, host-key verification, keepalive, and one jump host.
- Connections can be grouped, reordered, backed up, and explicitly imported from OpenSSH Config or a Luna Remote database.
- SFTP supports local/remote browsing, upload, download, preview, drag and drop, conflict handling, transfer queues, and retry.
- Local, remote, and SOCKS5 dynamic forwarding are supported.
- Remote agent integration is disabled by default. When enabled, runtime files are isolated under `~/.luna-mux/runtime/<runtime-id>` on the remote host and removed on a normal disconnect.

### AI command assistant

The AI command assistant is independent of Codex and Claude agents. It uses a user-configured OpenAI-compatible service to generate Linux Shell, PowerShell, CMD, or macOS commands for the focused local or SSH terminal.

Suggestions include explanations, assumptions, warnings, and a risk level. A command can be copied, inserted without Enter, or executed after risk confirmation. Optional terminal context can be processed by common personal-data redaction before the request is sent. Leaving AI unconfigured does not affect any other feature.

## Runtime model

```text
Luna Mux
`-- Mux Session (project and authorization boundary)
    |-- Pane
    |   `-- Terminal Runtime (local PTY or SSH)
    |       `-- optional Codex / Claude Code agent
    `-- Browser Resource
        `-- managed external Chrome Runtime
```

Sessions, Panes, and Browser Resources are persistent. Terminal Runtimes, agent processes, Chrome processes, output buffers, and temporary grants exist only for the current application lifetime. This keeps layout restoration predictable and avoids leaving unmanaged processes behind after exit.

## Platform status

| Capability | macOS | Windows |
| --- | --- | --- |
| Local terminal | zsh/bash validated | PowerShell 7 validated; full WSL validation in progress |
| SSH, SFTP, transfers, and tunnels | Implemented and continuously exercised | Implemented and tested on real hardware |
| Codex Hook, Luna MCP, Browser MCP | Local and real SSH paths validated | PowerShell path validated; WSL pending |
| Claude Code Adapter | Basic launch and injection validated | Equivalent end-to-end scenarios pending |
| Agent notifications | Theme-aware in-app notification with Pane routing | Native system notification; continued real-device regression pending |
| Packages | Unsigned DMG | Unsigned standard/bundled-WebView2 NSIS installers |

See [development progress](docs/DEVELOPMENT_PROGRESS.md) and the [long-term task list](docs/DEVELOPMENT_TASKS.md) for validation evidence and unfinished work.

## Quick start

There is no stable binary release yet, so developers should currently run Luna Mux from source. Requirements:

- Node.js 24 and npm;
- Rust stable 1.85 or newer;
- macOS: Xcode Command Line Tools;
- Windows: MSVC Build Tools, Windows SDK, WebView2 Runtime, and NASM.

```bash
git clone https://github.com/rainj2013/luna-mux.git
cd luna-mux
npm ci
npm run dev
```

`npm run dev` synchronizes the platform-specific `agent-browser` sidecar, starts Vite on `127.0.0.1:1420`, and builds and launches the Tauri desktop application. The first Rust build takes noticeably longer than incremental builds.

See the [development guide](docs/DEVELOPMENT.md) for full environment setup, Cargo mirror configuration, packaging, and troubleshooting.

## Checks and builds

```bash
npm run check
npm test
npm run web:build
```

Build desktop packages on their target platform:

```bash
npm run build:mac
# or on Windows
npm run build:win
npm run build:win:webview2
```

GitHub Actions can build macOS Intel, macOS Apple Silicon, a standard Windows package, and a compatibility package with WebView2 bundled. Windows 10/11 normally already includes WebView2, so most users should download the smaller standard package. The bundled-WebView2 package is intended for machines that lack WebView2 and cannot download it during installation. Pushing a `v*` tag publishes a GitHub Release. Current builds use ad-hoc or no signing, so no paid developer identity is required, but first launch may trigger a macOS Privacy & Security confirmation or Windows SmartScreen.

## Data and security

- The database, settings, browser profiles, and credential namespace are fully isolated from Luna Remote.
- Passwords and AI keys use macOS Keychain or Windows Credential Manager rather than the project database.
- Agent events retain lifecycle metadata only, not prompts, tool input, or tool output.
- Luna MCP uses authenticated loopback-only transport. Tokens are Runtime-scoped and revoked when the Runtime exits.
- Remote agent support files do not modify shell startup files or user-level agent configuration on the remote host.
- Browser automation connects only to isolated Chrome instances launched by Luna Mux. Remote agents do not receive raw CDP addresses.

## Documentation

- [Development setup, checks, and packaging](docs/DEVELOPMENT.md)
- [Product and architecture design](docs/LUNA_MUX_DESIGN.md)
- [Terminal Runtime architecture](docs/TERMINAL_ARCHITECTURE.md)
- [Current development progress](docs/DEVELOPMENT_PROGRESS.md)
- [Long-term task list](docs/DEVELOPMENT_TASKS.md)

Product naming, application identity, and storage namespaces are maintained in `product/product.json`. Run `npm run product:sync` after editing it and verify generated output with `npm run product:check`.

## Feature continuity with Luna Remote

[Luna Remote](https://github.com/rainj2013/luna-remote) is a separate desktop application focused on everyday SSH and SFTP use. Luna Mux focuses on project terminals, coding-agent collaboration, and managed-browser workflows. Luna Mux combines familiar connection management, SSH, SFTP, transfer, and port-forwarding capabilities while preserving a similar UI and interaction model for layouts, themes, terminal appearance, and routine remote work.

The repositories are completely independent at the Git level: they do not share managed history, an upstream remote, or commit synchronization. When a Luna Remote capability is useful, AI reviews its current code and behavior and reimplements it against Luna Mux's Session, Pane, Runtime, and permission model. Luna Mux never reads or modifies Luna Remote data automatically; it reads a selected snapshot only when the user explicitly runs the import flow.

## License

Luna Mux is released under the [MIT License](LICENSE). Third-party components, fonts, and bundled tools retain their own licenses and distribution terms.
