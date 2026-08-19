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
    <a href="docs/DEVELOPMENT.md">Development guide</a>
    ·
    <a href="docs/LUNA_MUX_DESIGN.md">Design</a>
  </p>
</div>

<div align="center">English · <a href="README.md">简体中文</a></div>

Luna Mux brings terminals, coding agents, remote machines, and browser automation into one workspace. A Session persists its project root, local or SSH Panes, and recursive split layout. Codex, Claude Code, and other agents run in terminals, with status notifications and browser tooling.

## Project Sessions and Terminal Panes

- A Session represents one project context and persists its project root, local or SSH Panes, and recursive split layout.
- Layout controls include horizontal and vertical splits, draggable ratios, presets, rename, maximize, and restore.
- macOS uses a local zsh/bash PTY. Windows supports PowerShell and WSL. Both platforms can create SSH terminals.
- Local and remote terminals share one xterm.js UI, including search, clipboard handling, themes, fonts, backgrounds, and output flow control.
- Restarting the application restores Session and layout definitions without reconnecting hosts or launching processes behind the user's back.

## Coding Agents

Codex, Claude Code, and other agents run in terminals. Launch `codex` or `claude` manually, or pick a saved launch profile when creating a Pane to start the agent automatically once the shell is ready.

- Unified working, waiting-for-input, waiting-for-permission, completed, and error states.
- Attention indicators in the sidebar and Pane border, plus desktop notifications that route back to the owning Session and Pane.
- An Agent Environment view for inspecting the Adapter, Hook, Luna MCP, and Browser MCP health state.
- Agent lifetime follows the application; managed terminals, agents, and Chrome close with Luna Mux.

## Agent Control of Luna Mux

Luna MCP exposes Luna Mux's control capabilities to agents in the terminal, covering Sessions, Panes, terminals, agents, connections, settings, diagnostics, transfers, and tunnels. Agents can:

- discover Sessions, Panes, Terminal Runtimes, and other managed agents;
- create Panes, update layouts, read bounded terminal output, and write terminal input;
- inspect agent state, send tasks, and interrupt managed agents;
- read safe connection summaries, update themes or terminal appearance, and run built-in diagnostics;
- close Runtimes or start transfers and tunnels, important side effects that may still require desktop approval.

Credentials, private-key contents, and API keys are never exposed through MCP.

## Browser Automation

Each Session can own one isolated Chrome that agents use to automate web tasks. The browser runs as a full, standalone window that the user can take over at any time.

- Agents use a native `agent-browser` MCP for snapshots, interaction, waits, tabs, screenshots, console inspection, network requests, and HAR capture.
- Chrome starts lazily on the first browser tool call, then the same Runtime and page are reused.
- Remote SSH agents reach the desktop's Chrome through an authenticated proxy; raw CDP is not exposed to the remote host.

## SSH, SFTP, and Port Forwarding

- SSH supports passwords, private keys, SSH Agent, host-key verification, keepalive, and one jump host.
- Connections can be grouped, reordered, backed up, and imported from OpenSSH Config or a Luna Remote database.
- SFTP supports local/remote browsing, upload, download, preview, drag and drop, transfer queues, and retry.
- Local, remote, and SOCKS5 dynamic forwarding are supported.

## AI Command Assistant

The AI command assistant is independent of Codex and Claude agents. It uses a user-configured OpenAI-compatible service to generate Linux Shell, PowerShell, CMD, or macOS commands for the focused local or SSH terminal.

- Suggestions include explanations, assumptions, warnings, and a risk level, and can be copied, inserted without Enter, or executed after risk confirmation.
- Optional terminal context is processed by common personal-data redaction before the request is sent.
- Leaving AI unconfigured does not affect any other feature.

## Development

Environment setup, checks, and packaging instructions are in the [development guide](docs/DEVELOPMENT.md).

## License

Luna Mux is released under the [MIT License](LICENSE). Third-party components, fonts, and bundled tools retain their own licenses and distribution terms.
