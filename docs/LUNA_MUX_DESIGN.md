# Luna Mux Design

Status: approved baseline
Last updated: 2026-08-12

## 1. Product definition

Luna Mux is a local and remote terminal workspace for coding agents. It is a terminal multiplexer-style desktop application, not an IDE: it does not provide a code editor, language server, Git GUI, or other traditional IDE surfaces.

The first release supports Windows and macOS together and provides:

- local terminals: PowerShell and WSL on Windows; the login shell on macOS;
- remote Linux terminals through SSH;
- a shared terminal UI for every local and remote target;
- project-centered Mux Sessions with persistent horizontal and vertical split layouts;
- coding-agent status, notifications, and controlled cross-agent interaction;
- a local Chrome browser pane controlled through CDP;
- the reusable SSH, SFTP, transfer, tunnel, credential, and terminal capabilities inherited from Luna Remote.

The first release does not keep agents running after Luna Mux exits. It preserves a terminal backend boundary so daemon-backed runtimes can be added later without replacing Mux Sessions, panes, or the terminal UI.

## 2. Repository and product boundary

Luna Mux and Luna Remote are independent applications and repositories.

- Luna Remote is a source of reusable implementation and proven capabilities, not a product-design template for Luna Mux.
- Luna Mux keeps the Luna Remote Git history as its initial ancestry.
- `origin` will point to the independent Luna Mux repository when it is created.
- `upstream` is a fetch-only reference to Luna Remote.
- Updates are reviewed and ported by commit; the complete upstream branch is never merged blindly.
- Runtime identifiers, databases, credentials, settings, browser profiles, builds, and releases are isolated.
- Luna Remote data is never migrated automatically. A later import wizard reads a snapshot only after explicit user approval.

Reuse is decided capability by capability. SSH transport, SFTP, tunnels, transfers, credential handling, terminal rendering, and platform fixes may be retained when they fit Luna Mux. Luna Remote navigation, tab semantics, state ownership, naming, data relationships, and UX flows carry no compatibility requirement and may be replaced completely.

Reusable code is gradually separated into in-repository core modules. Product features may depend on core; core must not depend on Mux Session, agent, browser, or brand-specific code. When inherited code conflicts with the Luna Mux domain model, the Luna Mux model takes precedence and the inherited implementation must be adapted, rewritten, or omitted.

## 3. Product metadata

`product/product.json` is the single editable source for product identity. `npm run product:sync` updates generated runtime constants and build manifests. `npm run product:check` fails when generated files are stale.

The metadata owns the display name, stable product key, executable and package names, bundle ID, database name, credential service, URL scheme, description, and icon set. Runtime code consumes generated `ProductInfo` or Rust constants rather than embedding product names.

If the product is renamed later, the previous identity is added to `legacyIdentities`; an explicit, idempotent migration then copies and verifies old data before switching identities.

## 4. Architecture

```text
Luna Mux Desktop
|-- Mux Session / split-pane manager
|-- TerminalRuntimeService
|   |-- InProcessLocalPtyTerminalBackend
|   |-- InProcessSshTerminalBackend
|   `-- future DaemonTerminalBackend
|-- Agent dashboard and event receiver
|-- Luna MCP server
|-- Local Chrome CDP controller
`-- SFTP / tunnel / transfer tools
```

### 4.1 Domain model and terminology

The word session is reserved for the project-level container. The application distinguishes:

- `muxSessionId`: a stable, persistent project container;
- `paneId`: a stable leaf in that Mux Session's layout;
- `runtimeId`: one running terminal instance owned by a Terminal pane;
- `agentId`: an optional managed coding-agent process running in a terminal runtime;
- `targetId`: a PowerShell, WSL, macOS shell, or SSH target.

A Mux Session has a name, one primary project root, a default target, a recursive pane layout, and browser profile settings. A pane may override the target and working directory, so one project can combine local terminals, different SSH hosts, coding agents, ordinary shells, and browser panes. A Terminal pane owns at most one live Terminal Runtime. Restarting it creates a new `runtimeId` without changing its `paneId` or `muxSessionId`.

All terminal backends expose the same operations: create, write, resize, flow, interrupt, close, list, read output, and report capabilities. They emit normalized status, output-with-cursor, and exit events identified by `runtimeId`.

The initial implementation runs Terminal Runtimes in the Luna Mux process. A future `DaemonTerminalBackend` may implement attach/detach, IPC, persistent output, and remote bridges behind the same interface. No daemon wire protocol or runtime persistence is implemented now.

### 4.2 Unified terminal

Local and SSH Terminal Runtimes use the same React `TerminalPane` and xterm.js instance. Rendering, themes, fonts, backgrounds, search, copy/paste, shortcuts, resize, UTF-8 handling, WebGL fallback, and output flow control behave identically.

Capabilities control target-specific tools. Local runtimes do not show SFTP or SSH forwarding; SSH runtimes keep those tools. The terminal surface itself does not fork by platform or transport.

Local PTY support uses a mature cross-platform PTY library:

- Windows: ConPTY, PowerShell, and selected WSL distributions;
- macOS: Unix PTY and the user's configured login shell.

Windows child processes are assigned to a Job Object. macOS runtimes use a process group and signals. Closing a pane terminates its foreground process tree after confirmation; exiting Luna Mux confirms once for all active agents and then closes every PTY, SSH channel, and managed Chrome process.

### 4.3 Mux Sessions and panes

A Mux Session normally represents one project. Its primary project root supplies the default cwd for local terminals and project-aware tools. The root may be absent for an ad-hoc session, and individual panes may override cwd and target. The first pane types are Terminal and Browser.

The layout is a recursive tree of pane leaves and horizontal/vertical branches with ratios. Users can split, resize, focus, move, maximize, restore, restart, and close panes.

The database stores the Mux Session, project root, layout, pane target/cwd overrides, titles, launch profiles, and browser profile references. Restarting Luna Mux restores this configuration but never reconnects, reruns a shell, or restores scrollback automatically. Runtime IDs and agent process IDs are not reused after an application restart.

### 4.4 Codex integration

Only coding agents launched through Luna Mux are fully managed. Launching associates stable Mux Session and pane identifiers with a fresh terminal runtime and agent identifier, then injects a scoped event endpoint and MCP configuration.

The Codex adapter uses structured hooks and notifications to report:

- `working`;
- `waiting`, with `input`, `permission`, `external`, or `unknown` reason;
- `completed`;
- `error`.

Local hooks report to a loopback receiver. Remote hooks use an ephemeral reverse forward on the current SSH connection, scoped by connection, runtime, and random token. The adapter installation flow previews and backs up configuration changes and can uninstall them.

Permission requests produce a notification and focus the associated terminal. Approval remains in the Codex TUI in the first release.

The dashboard filters agents, shows last activity and unread state, focuses a pane, sends input, interrupts, restarts, and marks notifications read. System notifications are suppressed for the currently focused pane but retained in the in-app history.

### 4.5 Cross-agent control

An in-process Luna MCP server exposes:

```text
agents.list
agents.get_status
agents.send_task
agents.interrupt
terminal.read_output
terminal.write
browser.*
```

Only Luna Mux-managed agents are discoverable. Access is granted directionally from a source agent to a target agent with read, write, and control capabilities. Raw PTY access is allowed after one explicit grant and remains visibly indicated until revoked.

Terminal output uses a bounded in-memory ring and monotonically increasing cursors. If old output is overwritten, `read_output` returns a truncation marker and the new earliest cursor. Output disappears when Luna Mux exits.

Cross-agent actions record caller, target, timestamp, operation, input, and result for 30 days by default.

### 4.5.1 Agent control of Luna Mux

Luna Mux itself is an operable application for local AI Agents. The core
control boundary is the versioned, transport-neutral `LunaControlService`.
Future MCP, CLI, or local IPC adapters authenticate a caller and then invoke
this service; they never access SQLite, `SessionManager`, or a terminal backend
directly. The same operation catalog is used for capability discovery across
transports.

The first reserved operations are read-only `terminal.targets.list` and
`terminal.runtimes.list`, plus a bounded event stream with monotonic cursors.
The control envelope carries a contract version, request ID, optional resource
scope, arguments, and an idempotency key. Caller identity is injected by the
authenticated adapter and is intentionally absent from the request body.

Grants are scoped by resource kind and optional resource ID, with `read`,
`write`, and `control` levels. A control grant includes read and write access;
it does not silently grant access to unrelated resources. Destructive
operations such as interrupt, close, raw terminal input, transfer, tunnel, or
browser side effects will be gated by an approval policy before they are
exposed. Unauthorized, stale-version, invalid-argument, unavailable, and
internal failures use structured error codes rather than transport-specific
strings.

This control plane is deliberately independent of the cross-agent MCP work:
M4.8-M4.12 operate Luna Mux resources, while M5 handles directional discovery
and cooperation between managed coding agents. Both layers share the same
runtime IDs, event cursors, grants, and audit model.

### 4.6 Browser pane

Chrome always runs on the desktop hosting Luna Mux. Each Mux Session receives an isolated, persistent profile, with an optional temporary profile.

The Browser pane uses CDP screencast frames and forwards mouse, keyboard, wheel, navigation, viewport, and screenshot operations. It also supports opening the page in the external Chrome window. Native WebView parity for IME, drag/drop, accessibility, DRM, and all browser extensions is not promised.

Remote development services are exposed to local Chrome through the existing SSH local-forwarding capability. Luna Mux does not install or control remote Chrome. Managed Chrome processes and temporary forwards close with the application.

## 5. Data and compatibility

Luna Mux uses `com.luna.mux`, `luna-mux.db`, and `com.luna.mux.credentials`. It must never auto-open or modify Luna Remote storage.

The planned Luna Remote import wizard can copy selected connections, groups, host keys, settings, and forwarding profiles. Credential copying requires a separate explicit choice and uses the system credential store. Import reads a stable source snapshot and writes Luna Mux data transactionally.

Both Windows and macOS are release-one platforms. Every milestone is complete only after its applicable Windows and macOS acceptance cases pass. Signing, notarization, auto-update, and distribution governance are outside the current scope; installation follows the same user-approved risk flow as Luna Remote.

## 6. Delivery sequence

1. Establish the independent repository, product metadata, isolated identity, import boundary, and upstream workflow.
2. Extract `TerminalBackend` and remove SSH assumptions from `TerminalPane`.
3. Add Windows PowerShell/WSL and macOS login-shell PTYs with identical terminal behavior.
4. Replace the single-terminal tab model with project-centered Mux Sessions and persisted recursive split panes.
5. Add the Codex adapter, dashboard, event forwarding, and notifications.
6. Reserve and implement the Luna Mux control service, then add Luna MCP for
   cross-agent operations using that boundary.
7. Add local Chrome lifecycle, browser pane, CDP tools, and remote-service forwarding.
8. Continue selective upstream ports and cross-platform regression work throughout development.

The detailed executable work breakdown and durable status are maintained in `DEVELOPMENT_TASKS.md` and `DEVELOPMENT_PROGRESS.md`.
