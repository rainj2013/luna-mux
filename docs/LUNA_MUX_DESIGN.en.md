# Luna Mux Design

Status: approved design baseline
Last updated: 2026-08-20

## 1. Product Definition

Luna Mux is a local and remote terminal workspace for coding agents, closer in form to a terminal multiplexer than an IDE. It provides no code editor, language service, graphical Git interface, or other traditional IDE features.

The first release targets Windows and macOS simultaneously and covers:

- PowerShell 7 and WSL on Windows, and zsh/bash on macOS;
- remote Linux terminals over SSH;
- a unified terminal UI for local and remote;
- project-centric, persistent, recursively splittable Mux Sessions;
- coding agent status, notifications, application control, and controlled cross-Pane/cross-Agent collaboration;
- Session-scoped managed Chrome resources and native `agent-browser` automation;
- merged connection management, SSH, SFTP, file transfer, port forwarding, credentials, and terminal capabilities, with a UI and usage experience close to Luna Remote.

The first release does not keep agents running after Luna Mux exits. The terminal backend keeps a boundary for a future daemon, but background Sessions are not implemented now.

## 2. Repository and Product Boundaries

Luna Mux and Luna Remote are independent applications and independent repositories. The two are not maintained through shared Git history, an upstream remote, branch merges, or commit sync.

- Luna Remote is one of the reference sources for functional behavior and interaction experience.
- Luna Mux merges connection management, SSH, SFTP, tunnels, transfers, credential handling, and terminal capabilities while keeping a similar UI and usage habits.
- When a Luna Remote feature needs to be referenced, AI reads its current code and behavior, then reimplements it against Luna Mux's domain model and security boundaries rather than porting commits directly.
- Application identifiers, databases, credentials, settings, browser profile directories, builds, and releases are fully isolated.
- Luna Remote data is not migrated automatically; the import wizard reads a stable snapshot only after explicit user confirmation.

Feature merging is done per actual need. Navigation, state ownership, data relationships, and interaction flows follow Luna Mux's Session, Pane, Runtime, Agent, and Browser Resource model; identical capabilities may keep a familiar interface and operation style.

Shared code is gradually moved down into core modules within the repository. Product features may depend on core modules, but core modules must not depend back on Mux Session, Agent, Browser, or brand code. On domain-model conflicts, the Luna Mux model wins.

## 3. Product Metadata

`product/product.json` is the single editable source of product identity. After changes, run:

```bash
npm run product:sync
npm run product:check
```

Metadata covers the display name, product key, executable and package names, bundle ID, database name, credential service, URL scheme, description, and icons. Runtime code reads the generated `ProductInfo` or Rust constants and does not repeat brand literals.

When renaming in the future, first add the old identity to `legacyIdentities`, then copy old data through an explicit, idempotent, verifiable migration.

## 4. Overall Architecture

```text
Luna Mux desktop app
|-- Mux Session / split manager
|-- TerminalRuntimeService
|   |-- InProcessLocalPtyTerminalBackend
|   |-- InProcessSshTerminalBackend
|   `-- future DaemonTerminalBackend
|-- AgentAdapter registry
|   |-- CodexAdapter
|   `-- ClaudeCodeAdapter
|-- Agent panel and shared Hook receiver
|-- LunaControlService
|   |-- trusted desktop adapter
|   |-- local Luna MCP adapter
|   `-- future CLI / local IPC adapter
|-- Session Browser Resource manager / local Chrome CDP controller
`-- SFTP / tunnel / transfer tools
```

### 4.1 Domain Model and Terminology

`Session` specifically means the project-level container:

- `muxSessionId`: a persisted project container;
- `paneId`: a stable leaf node in the Session's split tree;
- `runtimeId`: one running instance of a terminal Pane;
- `agentId`: the optional coding agent process in a terminal Runtime;
- `browserResourceId`: a persisted browser definition belonging to a Session;
- `targetId`: a PowerShell, WSL, macOS shell, or SSH target.

A Mux Session contains a name, a project root, a recursive terminal layout, and zero or more Browser Resources. A Pane can override its target and working directory. A terminal Pane owns at most one live Runtime; restarting the Runtime does not change `paneId`. A Browser Resource is not part of the split tree.

### 4.2 Terminal Backend

All terminal backends implement the same set of operations — create, write, resize, flow control, interrupt, close, list, read output, and report capabilities — and emit standardized status, cursor output, and exit events keyed by `runtimeId`.

Current Runtimes run inside the Luna Mux process. A future `DaemonTerminalBackend` can implement attach/detach, IPC, persistent output, and remote bridging behind the same interface, but authentication, process ownership, shutdown, backpressure, reconnection, and version negotiation must be defined before enabling it. Daemon fields must not leak into `TerminalPane`.

Local and SSH Runtimes share the React `TerminalPane` and xterm.js. Themes, fonts, backgrounds, search, clipboard handling, shortcuts, resize, UTF-8, WebGL fallback, and output flow control stay consistent. SFTP and SSH forwarding are gated only by capability flags and do not fork the terminal UI.

- Windows uses ConPTY, PowerShell 7/5.1, and the selected WSL distribution, and cleans up the process tree through Job Objects.
- macOS uses Unix PTYs and the user-configured zsh/bash, and cleans up through process groups and signals.
- Closing an active Pane requires confirmation; exiting the app confirms once and cleans up all PTYs, SSH channels, and managed Chrome.

#### 4.2.1 Data Flow and Output Backpressure

```text
Terminal Runtime
  -> backend (SSH russh / local portable-pty)
  -> Rust incremental UTF-8 decoding and runtime event
  -> Tauri `terminal-runtime:event`
  -> React TerminalPane
  -> xterm.js
```

The app does not embed Terminal.app, Ghostty, or Windows Terminal. Each terminal Pane corresponds to a distinct Terminal Runtime; the current SSH implementation is carried by one Rust SSH connection and PTY channel, and disconnecting one does not affect other Panes.

The Rust backend requests an `xterm-256color` PTY through `russh` and forwards window size changes directly to the remote. Local PTYs use `portable-pty`; a Windows ConPTY may send an `ESC[6n` cursor position query on startup, which the real xterm.js `TerminalPane` answers — the PTY backend does not implement terminal emulation itself.

SSH and local PTY data chunks are processed in Rust with an incremental UTF-8 decoder, so multi-byte characters spanning chunks are never truncated. Runtime output uses a bounded ring buffer and a UTF-8 byte cursor; `TerminalPane` catches up incrementally from the cursor when it mounts. When the xterm write queue backs up, the frontend pauses backend reads through flow control and resumes once the backlog clears, avoiding large outputs occupying the WebView event loop for a long time.

A running Pane keeps the same xterm instance across cross-Session and within-Session view switches; non-active workspaces are only hidden with WebGL rendering disabled. When a component genuinely must unmount (for example, a layout reorder), the normal screen, alternate screen, and scrollback buffer are staged using xterm's official serialization format, and catch-up resumes from the last fully rendered UTF-8 byte cursor. You cannot just replay bounded raw PTY output, because full-screen TUI clear-screen and cursor-control sequences cannot reconstruct lost scroll history. Deleting a Pane or Session drops the corresponding snapshot at the same time.

Terminals enable the WebGL addon and fall back automatically to xterm's default renderer when initialization fails. Transparent backgrounds are composed from xterm's transparent canvas and the terminal container background image, without changing the whole system window's transparency.

#### 4.2.2 Security Boundaries

The WebView can only invoke explicitly registered Rust capabilities through Tauri commands. External links allow only HTTP/HTTPS, file selection uses native dialogs, and credentials are stored in the macOS keychain or Windows Credential Manager. Terminal and file transfer do not spawn Node.js child processes or helper processes.

### 4.3 Mux Sessions and Panes

A Mux Session usually corresponds to one project; the project root provides the default working directory for local terminals and project tools, and may be empty. Each Pane can override its working directory and target individually. An Agent is a detected process in a terminal, not a Pane type.

The layout is a recursive tree of Pane leaf nodes and horizontal/vertical split nodes, each split node carrying a ratio. Users can split, resize, focus, maximize, restore, rename, and close Panes, and can apply balanced row, column, or grid presets. Layout changes must not restart existing Runtimes.

The database persists Sessions, project roots, layouts, Pane target/working-directory/title, and Browser Resources. Restarting the app restores definitions only; it does not reconnect, run a shell, start Chrome, or restore scrollback. Runtime IDs and process IDs are not reused across app restarts.

The main sidebar contains only the Session and Pane tree. New work always starts from the selected Session: `New Pane` creates a normal terminal and chooses a local or SSH environment; the user can run `codex`, `claude`, or other commands there. Browser Resources are managed from the Session's browser view. SSH connections are kept in a secondary resource library and are not promoted to primary navigation alongside Sessions.

### 4.4 Agent Adapter Integration

There is no special Agent Pane. Users manually launch a supported agent in any normal terminal. Luna Mux injects stable Session/Pane/Runtime context and narrow-scope launch credentials into the Runtime; the Runtime's local shim reports `AgentProcessStart` immediately, and a structured `SessionStart` then binds the provider session. `SessionEnd`, process exit, or Runtime exit clears the identity.

`AgentAdapter` is the native extension boundary. Each adapter is responsible for:

- the manual command shim;
- Hook/MCP configuration;
- remote Hook transport requirements;
- an optional compatibility strategy for persisting user Hooks.

Runtime, panel, Hook receiver, Luna MCP, Browser Resource, authorization, and audit stay provider-agnostic. To add an agent provider, implement and register an adapter, then add contract tests.

Codex and Claude Code structured Hooks are unified as:

- `working`;
- `waiting`, with reason `input`, `permission`, `external`, or `unknown`;
- `completed`;
- `error`.

Codex uses process-level TOML overrides and command Hooks; Claude Code uses process-level `--settings` HTTP Hooks and `--mcp-config`. Normal integration does not rewrite `~/.claude`. Both share the Luna MCP and the native `agent_browser` MCP.

Remote agent integration is off by default. When off, a normal SSH Pane does not probe for agent commands, open SFTP, upload files, establish reverse forwarding, or modify the interactive shell. Remote file changes and audit/EDR risks must be shown before enabling.

When enabled, support files all go under `~/.luna-mux/runtime/<runtime-id>`; only that directory is added to the current Pane's `PATH`. Luna Mux does not modify remote shell startup files or the user's agent config, and deletes that exact directory over SFTP before a normal disconnect. Remote cleanup cannot be guaranteed on network interruption or process crash.

Permission requests are still approved by the agent TUI; Luna Mux notifies and focuses the owning terminal. The Agent view shows provider, owning Pane, target, Hook, Luna MCP, and Browser MCP health; unread and attention states continue to appear in the sidebar and terminal border.

### 4.5 Unified Luna Control API

`LunaControlService` is the single core boundary through which humans and local AI agents operate the app. The trusted desktop UI, the local Luna MCP, and future CLI/IPC all call the same operation catalog; adapters authenticate the caller before invoking the service and must not access SQLite, `SessionManager`, terminal backends, or Browser Runtime directly.

The resource graph:

```text
Application
`-- Mux Session
    |-- Pane
    |   `-- Terminal Runtime
    |       `-- optional Agent process
    `-- Browser Resource
        `-- optional Browser Runtime
```

Requests, authorization, events, and audit use stable resource IDs, not display names or process IDs. The operation catalog expands gradually through capability discovery; the current scope includes:

- discovery and status of Sessions, Panes, terminal targets, Runtimes, agents, transfers, and tunnels;
- Session/Pane metadata, Pane creation, and complete layout updates;
- bounded terminal output read, write, resize, flow control, interrupt, and close;
- agent status, task delivery, and interrupt;
- whitelisted application settings read and update;
- a bounded control event stream.

Agent-visible MCP follows these rules:

- Secure global settings use a separate `Settings` authorization; the `Application` super-resource is never granted to agents;
- Connection discovery returns only operational metadata, never credential values, private key contents, or AI keys;
- Session, Pane, and layout writes are limited to the caller's current Session;
- Creating a Pane persists the Pane and split tree first, then notifies the live desktop to start the Runtime through the normal Tauri path; concurrent creations run serially to avoid overwriting layouts;
- A complete layout must and can only contain every Pane of the current Session, each exactly once, with bounded ratios and nesting depth;
- Theme and terminal appearance update SQLite, the native window, and the WebView together;
- Transfer and tunnel observation is limited by Runtime; starting transfers/tunnels and closing Runtimes keep desktop approval;
- Browser automation does not enter the Luna MCP tool catalog and is provided only by the Session-aware `agent_browser` MCP.

The control envelope contains a contract version, a request number, an optional resource scope, parameters, and an idempotency key. Caller identity is injected by the authentication adapter and does not appear in the request body. Unauthorized, expired version, invalid parameters, unavailable, and internal errors use structured error codes. Mutating operations define idempotent behavior so retries cannot duplicate resources or side effects.

#### 4.5.1 Cross-Pane and Cross-Agent Control

A Pane is a resource, not a caller. An agent in a terminal receives an authenticated identity; the Mux Session is the collaboration and security boundary. By default it can discover Panes, live Runtimes, and agents in the same Session but cannot access other Sessions. A normal shell Pane is a first-class control target just like an Agent Pane.

Common operation examples:

```text
terminal.runtime.output.read  read the bounded output of a live Pane in the same Session
terminal.runtime.write        write PTY input to a live Pane in the same Session
terminal.runtime.interrupt    interrupt a Terminal Runtime in the same Session
agents.get_status             read the structured status of a detected agent
agents.send_task              deliver a task through the target adapter
agents.interrupt              interrupt a detected agent
mux.pane.create               create a Pane, insert it into the layout, and start the Runtime on demand
mux.layout.set                write a fully validated Session split tree
```

Same-Session membership is itself the trust decision for output reads, PTY writes, and agent task delivery; no second layer of Luna Mux approval is added. Close, transfer, tunnel, and other destructive lifecycle operations are still approved per their own policies. Cross-Session permission cannot be inferred from display names, process parent/child relationships, or the same SSH target.

Terminal output uses a bounded in-memory ring and a monotonic cursor; when old output is overwritten, a truncation marker and the new earliest cursor are returned. Output disappears after the app exits. Control operations keep a 30-day audit by default, recording caller, target, time, operation, input summary, approval, and result.

### 4.6 Browser Resources

Chrome always runs on the desktop where Luna Mux runs. Each Browser Resource belongs to one Mux Session, uses an isolated persistent profile directory, and can also request a one-shot temporary directory.

A Browser is not a Pane and does not participate in terminal splitting, resizing, or maximizing. Users interact in a standard external Chrome window; the Luna Mux browser view only manages resource names, lifecycle, launch, focus, restart, stop, and delete. Launching a local resource opens only one controlled `about:blank` page and does not navigate to a network address on its own; the legacy database `url` field is kept only to be compatible with historical remote records.

Chrome CDP binds to a random loopback port. The Browser Runtime's process ID, CDP port, WebSocket, and temporary forwarding address do not enter SQLite and are not continuously screenshotted. Agents request an accessibility snapshot or screenshot only when needed.

The first release allows one running managed Chrome per Session. The wrapper inherits `muxSessionId` from the environment; implicit selection is refused when a Session has multiple Browser Runtimes. Direct CDP cannot enforce per-agent authorization, so no fine-grained permissions are claimed until an authenticated CDP proxy is added later.

Luna Mux does not reimplement browser automation. It launches an isolated external Chrome and connects a pinned-version, verified native `agent-browser` to a stable loopback CDP. That tool provides stable references, interaction, waits, screenshots, tabs, console, network inspection, and typed MCP. Legacy `browser.*` Luna MCP methods are kept only for internal migration/diagnostics.

`luna-mux mcp browser` resolves the endpoint for the current Session and starts the built-in sidecar; `luna-mux mcp chrome` is kept only as a compatibility alias. Distribution does not depend on Node.js or `npx`; Node is used only for the repo's Vite/Tauri build.

An SSH Pane uses a single POSIX shell helper uploaded to the Runtime directory, and connects to the desktop through a remote loopback port allocated by the SSH server plus Runtime-random credentials. Hook forwarding prefers curl/wget; the Browser MCP byte bridge uses socat, nc/ncat, or bash `/dev/tcp`; Python is not required on the remote. After desktop verification the same Session-level sidecar starts. Raw CDP is not forwarded to the remote, the helper embeds no credentials, and the bridge expires with the Runtime.

In Luna Mux terminals, process-level config disables the unavailable, request-stealing Codex Browser Plugin, the `node_repl` browser path, and their cache skills, while merging the user's existing skill config without modifying global files. Browser requests keep only the `agent_browser` supported route.

Tool routing first distinguishes three resource domains: Luna Mux's own application settings, connection summaries, Sessions, Panes, Terminal Runtimes, managed Agents, SFTP transfers, and SSH tunnels go to the Luna MCP; URLs, web pages, DOM, links, forms, page screenshots, browser tabs/windows, browser console, and page network traffic go to `agent_browser`; source code, files, Git, builds, ordinary shell commands, OS settings, and external services go to the corresponding native tools. An agent must not route ordinary development operations to the Luna MCP merely because it runs inside Luna Mux, and must not use `agents.*` as a substitute for its own subagent/delegation mechanism.

Every Luna MCP tool description states the resource owner, typical user intent, and adjacent-domain counterexamples. An unqualified "pane", Pane, split, or layout always means Luna Mux application resources and maps to `mux.pane.create`, `mux.panes.list`, and `mux.layout.set` respectively, never browser tabs or windows. Browser routing constraints also include: get the current URL and a snapshot before interacting, reuse the current page, never start or restore a browser through the shell, and block resource lifecycle tools such as install/upgrade/connect/close. Agents may use tabs and windows on their own when there is a real context-isolation or multi-page need, but routine navigation and error recovery must not create new tabs for no reason.

Startup is triggered by the first real tool call. Each Session reserves a stable CDP port so the MCP can initialize and publish its catalog even before Chrome is running. `PreToolUse` asks Luna Mux, before the first browser call, to start the single candidate resource, wait for CDP, warm up the Session-level daemon, then release the original call. Multiple agents reuse the same port and Runtime.

The warm-up solves the `agent-browser` 0.34.0 issue on Windows where creating a background daemon may inherit STDIO handles and hang waiting for EOF; macOS keeps the same path. If Chrome restarts or the pinned tab disappears, the warm-up temporarily unpins, adopts the existing page, restores the strict pin, and verifies, without creating a new tab.

Remote development services keep using a separate SSH local forward, then the browser accesses the loopback URL. Creating a new Browser Resource does not select an SSH Pane and does not persist tunnel ownership; the historical `sourcePaneId` is read only for migration compatibility.

## 5. Data and Compatibility

Luna Mux uses `com.luna.mux`, `luna-mux.db`, and `com.luna.mux.credentials`, and must not open or modify Luna Remote storage automatically.

The import wizard can copy selected connections, groups, host keys, settings, and forwarding config. Copying credentials requires a separate explicit choice and goes through the system credential store. Import reads a stable snapshot and writes into Luna Mux transactionally.

Windows and macOS are both first-release platforms. Milestones complete only after the applicable dual-platform acceptance passes. Signing, notarization, auto-update, and distribution governance are out of scope for now.

## 6. Delivery Order

1. Establish the standalone repository, product metadata, isolated identity, data-import boundary, and feature-reference boundary.
2. Extract `TerminalBackend` and remove SSH assumptions from `TerminalPane`.
3. Add Windows PowerShell/WSL and macOS zsh/bash PTYs and unify terminal behavior.
4. Replace the single-terminal tab model with project-level Mux Sessions and persistent recursive splits.
5. Add Agent Adapters, the status panel, event forwarding, and notifications.
6. Implement the Luna Control Service, then provide the Luna MCP and cross-agent operations through that boundary.
7. Add Session-level managed Chrome, the native browser MCP, and remote service forwarding.
8. Reference relevant product capabilities as needed, reimplement them with AI, and run cross-platform regression.
