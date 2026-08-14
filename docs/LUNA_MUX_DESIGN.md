# Luna Mux Design

Status: approved baseline
Last updated: 2026-08-14

## 1. Product definition

Luna Mux is a local and remote terminal workspace for coding agents. It is a terminal multiplexer-style desktop application, not an IDE: it does not provide a code editor, language server, Git GUI, or other traditional IDE surfaces.

The first release supports Windows and macOS together and provides:

- local terminals: PowerShell 7 and WSL on Windows; zsh or bash on macOS;
- remote Linux terminals through SSH;
- a shared terminal UI for every local and remote target;
- project-centered Mux Sessions with persistent horizontal and vertical split layouts;
- coding-agent status, notifications, and Agent control of Luna Mux, including
  controlled cross-pane and cross-Agent interaction;
- Session-level managed Chrome resources controlled through CDP;
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
|-- AgentAdapter registry
|   |-- CodexAdapter
|   `-- ClaudeCodeAdapter
|-- Agent dashboard and shared Hook event receiver
|-- LunaControlService
|   |-- trusted desktop adapter
|   |-- local Luna MCP adapter
|   `-- future CLI / local IPC adapters
|-- Session browser-resource manager / local Chrome CDP controller
`-- SFTP / tunnel / transfer tools
```

### 4.1 Domain model and terminology

The word session is reserved for the project-level container. The application distinguishes:

- `muxSessionId`: a stable, persistent project container;
- `paneId`: a stable leaf in that Mux Session's layout;
- `runtimeId`: one running terminal instance owned by a Terminal pane;
- `agentId`: an optional managed coding-agent process running in a terminal runtime;
- `browserResourceId`: a stable managed-browser definition owned by a Mux Session;
- `targetId`: a PowerShell, WSL, macOS shell, or SSH target.

A Mux Session has a name, one primary project root, a recursive terminal-pane layout, and zero or more managed browser resources. A pane may override the target and working directory, so one project can combine local terminals, different SSH hosts, Coding Agents, and ordinary shells. A Terminal pane owns at most one live Terminal Runtime. Restarting it creates a new `runtimeId` without changing its `paneId` or `muxSessionId`. A Browser Resource is not a pane and never participates in the split tree.

All terminal backends expose the same operations: create, write, resize, flow, interrupt, close, list, read output, and report capabilities. They emit normalized status, output-with-cursor, and exit events identified by `runtimeId`.

The initial implementation runs Terminal Runtimes in the Luna Mux process. A future `DaemonTerminalBackend` may implement attach/detach, IPC, persistent output, and remote bridges behind the same interface. No daemon wire protocol or runtime persistence is implemented now.

The daemon extension must preserve the same `TerminalBackend` operations and
runtime event contract. `runtimeId` remains stable while a caller is attached,
and attach/detach is an implementation concern rather than a new pane or
session type. A daemon may retain output after the desktop exits only after an
explicit product decision; the current release discards output on exit. The
future extension must define authentication, process ownership, shutdown,
backpressure, reconnect, and version negotiation before it is enabled. No
daemon-specific fields are allowed to leak into `TerminalPane`.

### 4.2 Unified terminal

Local and SSH Terminal Runtimes use the same React `TerminalPane` and xterm.js instance. Rendering, themes, fonts, backgrounds, search, copy/paste, shortcuts, resize, UTF-8 handling, WebGL fallback, and output flow control behave identically.

Capabilities control target-specific tools. Local runtimes do not show SFTP or SSH forwarding; SSH runtimes keep those tools. The terminal surface itself does not fork by platform or transport.

Local PTY support uses a mature cross-platform PTY library:

- Windows: ConPTY, PowerShell 7 (`pwsh.exe`), and selected WSL distributions;
- macOS: Unix PTY and the user's configured zsh or bash shell. Other shells are
  intentionally outside the first-release support matrix.

Windows child processes are assigned to a Job Object. macOS runtimes use a process group and signals. Closing a pane terminates its foreground process tree after confirmation; exiting Luna Mux confirms once for all active agents and then closes every PTY, SSH channel, and managed Chrome process.

### 4.3 Mux Sessions and panes

A Mux Session normally represents one project. Its primary project root supplies the default cwd for local terminals and project-aware tools. The root may be absent for an ad-hoc session, and individual panes may override cwd and target. The first pane type is Terminal; an Agent is a detected process running inside a Terminal Runtime, never a pane type.

The layout is a recursive tree of pane leaves and horizontal/vertical branches with ratios. Users can split, resize, focus, maximize, restore, rename, and close panes. Existing panes can be rearranged without restarting their runtimes by applying balanced side-by-side, stacked, or two-columns-per-row presets. New panes may supply an optional display name; otherwise the selected terminal or SSH target label is used.

The database stores the Mux Session, project root, layout, pane target/cwd overrides, titles, legacy launch-profile fields, and Browser Resource definitions. A launch profile is now a compatibility field, not a pane kind: newly created panes are always ordinary Terminal panes. Restarting Luna Mux restores this configuration but never reconnects, reruns a shell, launches Chrome, or restores scrollback automatically. Runtime IDs and process IDs are not reused after an application restart.

The product information architecture follows the same ownership model. The
primary sidebar is a single project-navigation surface containing only the
Session list and each Session's Pane tree; it has no connection footer, second
resource tree, or inherited workspace/connection heading. Adding work always
begins from the selected Session. `Add Pane` creates one ordinary Terminal pane
and then chooses its local or SSH environment. The user may run `codex`, Claude,
or any other command in that terminal. Browser Resources are created and
managed from the Session's Browser view rather than `Add Pane`.
Saved SSH connections are named SSH targets in this context and live in a
secondary resource library used for creation, editing, import, and backup.
That library is reachable only from an SSH choice in `Add Pane` or resource
management commands. Targets never appear beside Sessions as another
first-class navigation tree, and opening a target always creates or reconnects
a Pane owned by the current Session.

### 4.4 Agent Adapter integration

There is no special Agent Pane. Every pane is a normal terminal runtime and
the user may start `codex`, `claude`, or another supported Agent manually. Luna Mux passes
stable Session/Pane/Runtime context and a narrowly scoped bootstrap credential
to every terminal runtime. A runtime-local Adapter shim sends an immediate
`AgentProcessStart` signal, so an Agent appears as soon as its process starts,
before the first turn. When the Agent emits `SessionStart`, the shared Hook receiver binds
the existing process identity to its `session_id`; `SessionEnd`, process exit,
or Runtime exit retires that identity. This makes manually started Agents
discoverable while keeping ordinary shell commands inert. Repeated Agent
launches in one terminal receive distinct live identities and stale dashboard
entries are removed.

`AgentAdapter` is the native extension boundary. Each Adapter owns its built-in
launch profile, automatic profile ID, runtime-local manual-command shim,
managed launch command, Hook/MCP configuration, remote Hook transport needs,
and compatibility with optional persistent user Hooks. The shared runtime,
dashboard, Hook receiver, Luna MCP, Browser Resource, authorization, and audit
services contain no provider-specific event model. Adding another Agent means
implementing and registering one Adapter, then supplying focused contract tests;
the frontend consumes the registry dynamically.

The managed launch profile is only an optional convenience path. `Start Agent`
lets the user select Codex or Claude Code and a terminal target, creates a normal
Terminal pane, persists the selected profile, and automatically starts the
Agent command after the shell is ready. It does not
introduce a second pane type. The ordinary `Add Pane` flow still starts only a
terminal, where the user may launch any Agent manually.

Codex and Claude Code both normalize their structured Hooks into:

- `working`;
- `waiting`, with `input`, `permission`, `external`, or `unknown` reason;
- `completed`;
- `error`.

Codex receives process-local TOML overrides and command Hooks. Claude Code
receives process-local `--settings` HTTP Hooks and `--mcp-config`; Luna Mux never
rewrites `~/.claude` for normal integration. Both use the same Luna MCP and
native `agent_browser` MCP wrapper. Local Hooks report to a loopback receiver.
Remote Hooks use an ephemeral reverse forward scoped by connection, runtime,
and random token: Codex uses the existing uploaded Python command forwarder,
while Claude Code posts directly through the forwarded loopback endpoint.
Optional preview/back-up/install support remains explicitly Codex-only.

Permission requests produce a notification and focus the associated terminal. Approval remains in the Agent TUI in the first release.

The Agent view is an observation and response center, not another launcher
library. It groups live identities into needs-response, running, and finished
sections; the needs-response queue is always first and exposes terminal input
inline. A compact overview shows each group count. Every Agent row retains pane
focus, optional input, interrupt, pane restart, read-state, and grant actions
without displaying raw Hook history as the primary content. System
notifications are suppressed for the currently focused pane but retained in
the in-app history.

### 4.5 Unified Luna Control API

Luna Mux is an operable application for both people and local AI Agents. Its
single core control boundary is the versioned, transport-neutral
`LunaControlService`. The trusted desktop UI, the local Luna MCP adapter, and
future CLI or local IPC adapters all converge on the same operation catalog.
An adapter authenticates its caller and invokes the service; it never accesses
SQLite, `SessionManager`, a terminal backend, or a Browser Runtime directly.
Product behavior must not be implemented only in an Agent-specific transport.

The control API follows the product's resource graph:

```text
Application
`-- Mux Session
    |-- Pane
    |   `-- Terminal Runtime
    |       `-- optional detected Agent process
    `-- Browser Resource
        `-- optional Browser Runtime
```

Stable resource IDs, rather than display names or process IDs, are used in
requests, grants, events, and audit records. A Pane remains the persistent
layout object while a Terminal Runtime and Agent identity may be replaced
without changing that Pane. Browser Resources remain outside the pane layout
but participate in the same Session ownership and authorization model.

The operation catalog is expanded incrementally, but its intended scope is:

- discovery and status for Sessions, Panes, terminal targets, Terminal
  Runtimes, Agents, Browser Resources, Browser Runtimes, transfers, and
  tunnels;
- Session lifecycle and metadata, including create, rename, project-root
  update, and delete;
- Pane creation, split, rename, move, focus, maximize, layout-preset,
  restart, and close operations;
- bounded terminal-output reads plus write, resize, flow, interrupt, and close
  operations;
- Agent status, task delivery, response, and interrupt convenience operations;
- Browser Resource lifecycle operations such as create, start, focus, restart,
  stop, and delete;
- allowlisted application settings reads and updates, including theme;
- a bounded event stream for resource, runtime, Agent, approval, and
  attention-state changes.

This list defines the target control surface, not a claim that every operation
is already implemented. Capability discovery must report only operations
available in the running version, allowing Agents to degrade gracefully rather
than guess UI behavior or configuration files. Page navigation and DOM
interaction remain the responsibility of the native `agent-browser` harness;
the Luna Control API owns the Browser Resource lifecycle and authorization, not
a second browser-automation implementation.

The control envelope carries a contract version, request ID, optional resource
scope, arguments, and an idempotency key. Caller identity is injected by the
authenticated adapter and is intentionally absent from the request body.
Unauthorized, stale-version, invalid-argument, unavailable, and internal
failures use structured error codes rather than transport-specific strings.
Mutating operations define idempotency behavior so retries cannot accidentally
create duplicate Sessions, Panes, tasks, or side effects.

### 4.5.1 Cross-pane and cross-Agent control

Cross-pane cooperation is an application of the unified Luna Control API, not
an independent subsystem. A Pane is a resource, not a caller. A detected Agent
process running in a Terminal Runtime receives an authenticated caller identity
and may be granted access to resources in its Mux Session. An ordinary terminal
Pane can therefore be a control target even when it does not contain a detected
Agent. Agent-specific operations are semantic conveniences layered over the
same terminal and runtime primitives.

For example:

```text
terminal.runtime.output.read  read bounded output from any granted live Pane
terminal.runtime.write        write raw PTY input to any granted live Pane
terminal.runtime.interrupt    interrupt a granted live Terminal Runtime
agents.get_status             read structured state for a detected Agent
agents.send_task              deliver a task using the target Agent Adapter
agents.interrupt              interrupt a detected Agent
```

The UI grant editor lists every other live terminal Pane in the current Mux
Session and visually distinguishes ordinary terminals from Panes containing a
detected Agent. Grants are directional: granting Agent A access to Pane B does
not grant B access to A. Mutual access requires two explicit grants. The first
release limits Agent-issued resource control to the current Mux Session;
cross-Session access is not inferred.

Grants are scoped by caller, resource kind, optional resource ID, and `read`,
`write`, or `control` access. `write` includes `read`; `control` includes both
and permits lifecycle actions allowed by policy. A grant never silently grants
access to unrelated resources. Raw PTY input, interrupt, close, transfer,
tunnel, Browser Resource, and other side effects are subject to the operation's
approval policy. Active access remains visible and immediately revocable.

Terminal output uses a bounded in-memory ring and monotonically increasing
cursors. If old output is overwritten, the read result includes a truncation
marker and the new earliest cursor. Output disappears when Luna Mux exits.
Control actions record authenticated caller, target, timestamp, operation,
input summary, approval decision, and result for 30 days by default.

### 4.6 Browser resources

Chrome always runs on the desktop hosting Luna Mux. Each Browser Resource belongs to one Mux Session and receives an isolated persistent profile; a resource can instead request a removable temporary profile.

Browser is not a Pane and does not participate in splitting, resizing, maximizing, or the terminal layout. The user interacts with the page in a standard external Chrome window with its normal address bar, tabs, and browser controls. Luna Mux's Browser view manages the stable resource name, lifecycle status, start, focus, restart, stop, and deletion; it does not render a live browser surface. Starting a local Browser Resource opens exactly one controllable blank tab and never visits a network URL automatically. The explicit `about:blank` launch target avoids Chrome's internal `chrome://newtab` target, which the automation harness would otherwise replace with a second controllable tab. Navigation belongs to the user or the Agent after startup. The existing database `url` column remains only for backward compatibility with legacy remote-bound records.

The implementation binds Chrome CDP to a random loopback port and persists one Chrome user-data directory per Browser Resource, or creates a disposable directory for a temporary profile. A Browser Runtime is ephemeral: its process ID, CDP port, websocket, and local forwarded URL never enter SQLite. There is no continuous screencast. Agent browser tools use CDP directly and request accessibility snapshots or screenshots only when needed.

The first release exposes one running managed Chrome per Mux Session to Agents
running in that Session. This Session scope is enforced by the wrapper's
inherited `muxSessionId`; if multiple Browser Runtimes are active for one
Session the wrapper refuses to choose implicitly. The old per-Agent Browser
grant rows remain migration data but are not presented as effective security,
because `agent-browser` connects directly to the loopback CDP endpoint.
Fine-grained per-Agent Browser authorization requires a future authenticated
CDP proxy and must not be represented in UI before it is enforceable.

Luna Mux does not reimplement browser automation. It launches an isolated
external Chrome and attaches a pinned native `agent-browser` executable to its
stable loopback CDP endpoint. `agent-browser` provides a mature Agent-oriented
harness: accessibility snapshots with stable refs, interaction and waits,
screenshots, tab selection, console and network inspection, and a typed MCP
surface. The Agent sees this as an ordinary `agent_browser` MCP server configured
by the terminal's Luna Mux context. The old `browser.*` Luna MCP methods are
retained only for migration/diagnostics and are not the Agent-facing browser
contract.

Codex's bundled Browser Plugin is supplied by the Codex desktop host and is not
a generic connector for arbitrary local CDP ports. Luna Mux therefore does not
pretend to embed or discover that private host backend. A small
`luna-mux mcp browser` wrapper resolves the current Session's reserved CDP
endpoint and launches the bundled `agent-browser` sidecar, allowing manual
`codex` and `claude` commands in ordinary Luna Mux terminals to use the same browser without exposed
ports or a separate Agent pane type. `luna-mux mcp chrome` remains only as a
backward-compatible command alias. Each terminal Adapter explicitly forwards the
Session ID and reserved CDP port through `mcp_servers.agent_browser.env`; it does
not assume an Agent will preserve arbitrary terminal environment variables when it
starts an MCP subprocess.

The Browser runtime path has no Node.js or `npx` dependency. Luna Mux, its MCP
wrapper, browser lifecycle manager, and the pinned `agent-browser` sidecar are
native executables. The repository still uses its existing Node-based Vite/Tauri
frontend build tooling; a small build-preparation script downloads the correct
checksummed sidecar for Windows x64, macOS x64, or macOS ARM64 before development
or packaging. Node is not installed or invoked by the distributed Browser MCP
runtime.

Within a Luna Mux terminal, the shim disables Codex's bundled Browser Plugin,
its `node_repl` browser channel, and the exact cached
`control-in-app-browser/SKILL.md` path because their private host backend is
unavailable here and otherwise wins Browser prompts despite returning no instances. Plugin
disablement alone is insufficient: Codex can still discover the cached Skill.
The process-local `skills.config` override therefore merges the user's existing
Skill choices before disabling that one conflicting path; it does not rewrite
the user's global configuration. Browser requests then have one supported Agent
route: the Session-aware `agent_browser` MCP server.

Basic MCP connectivity is not a sufficient browser integration contract. A
small process-local routing harness constrains Luna Mux-managed Agent processes.
Codex receives merged `developer_instructions`; Claude Code receives an appended
system prompt. Both require
`agent_browser_get_url` plus `agent_browser_snapshot` before interaction,
reuse of the current page, and no shell-based browser launch or
recovery. Provider configuration and shared `PreToolUse` enforcement remove or reject
Browser Resource connect/close, install/upgrade, dashboard, plugin, and chat
lifecycle calls plus browser-launching shell commands. Normal tab and window
operations remain available when the Agent judges them useful for the task.
Chrome process and connection lifecycle therefore remain owned by the Browser
Resource. The bound tab is the default workspace for ordinary single-page
navigation. Agents can autonomously create, switch, operate, and close tabs or
windows when the task benefits from preserving context, comparing pages, or a
multi-page workflow; users do not need to prescribe those tool-level steps.
Agents must not create a new tab merely for routine navigation setup or error
recovery.

Browser startup is demand-driven, not tied to launch order. Each supported Adapter starts the
native STDIO MCP server with a Session-stable, loopback CDP port even while no
Chrome process exists; `agent-browser` can initialize and publish its typed tool
catalog without connecting to Chrome. Immediately before the first real
`agent_browser` tool call, `PreToolUse` asks the running Luna Mux application to
ensure the Session browser. Luna Mux starts the one eligible local Browser
Resource on the reserved port, waits for CDP readiness, then allows the original
tool call to continue. All Agents in the Session share the port and the same
Runtime. The sidecar uses an isolated Session-derived namespace, and Luna Mux
closes that attached daemon before it stops the Browser Runtime. This is
triggered by the actual tool call rather than prompt keyword matching, so
ordinary Codex work does not open Chrome. Automatic selection refuses zero or
multiple local candidates with an explicit error. Browser Resources are always
local and never select or own an SSH connection.

On Windows, the first tool call also prewarms the Session-scoped agent-browser
daemon after Chrome reaches CDP readiness and before the Hook allows the MCP
call to continue. This avoids an upstream 0.34.0 stdio handle-inheritance case
where the MCP server can wait indefinitely when its captured CLI child is also
the process that creates the detached daemon. Warmup uses null stdio, the same
checksummed binary, config, Session namespace, and pinned CDP page; subsequent
MCP tools reuse that daemon. The same warmup is harmless and retained on macOS
so the browser routing contract remains platform-independent. If Chrome was
restarted or the previously pinned tab was closed, the persisted CDP target ID
is stale. Warmup then temporarily relaxes the pin, adopts one existing managed
Chrome page, immediately restores strict pinning, and verifies the binding
before the original Agent tool is allowed. Recovery never creates a tab or
exposes page-lifecycle tools to the Agent.

cmux is a useful reference but uses a different browser architecture. It owns
WKWebView surfaces and exposes a large `cmux browser` CLI/socket RPC contract,
plus an installable `cmux-browser` Skill that teaches Agents its snapshot/ref,
wait, and interaction workflow. It does not solve Codex-to-browser routing by
pointing the bundled Browser Skill at CDP. Luna Mux uses external managed Chrome
rather than an embedded webview, but now adopts the same practical lesson: the
CDP connection needs an Agent-oriented harness. The pinned native
`agent-browser` sidecar supplies that harness without requiring Luna Mux to own
a second browser automation implementation.

Remote development services remain reachable through Luna Mux's independent
SSH tunnel capability: the user creates a local forward and gives the Browser
Resource that loopback URL. Browser creation never asks for an SSH Pane and
does not persist tunnel ownership. Existing database records with a legacy
`sourcePaneId` remain readable during migration, but the current UI cannot
create new remote-bound Browser Resources.

## 5. Data and compatibility

Luna Mux uses `com.luna.mux`, `luna-mux.db`, and `com.luna.mux.credentials`. It must never auto-open or modify Luna Remote storage.

The planned Luna Remote import wizard can copy selected connections, groups, host keys, settings, and forwarding profiles. Credential copying requires a separate explicit choice and uses the system credential store. Import reads a stable source snapshot and writes Luna Mux data transactionally.

Both Windows and macOS are release-one platforms. Every milestone is complete only after its applicable Windows and macOS acceptance cases pass. Signing, notarization, auto-update, and distribution governance are outside the current scope; installation follows the same user-approved risk flow as Luna Remote.

## 6. Delivery sequence

1. Establish the independent repository, product metadata, isolated identity, import boundary, and upstream workflow.
2. Extract `TerminalBackend` and remove SSH assumptions from `TerminalPane`.
3. Add Windows PowerShell 7/WSL and macOS zsh/bash PTYs with identical terminal behavior.
4. Replace the single-terminal tab model with project-centered Mux Sessions and persisted recursive split panes.
5. Add the Codex adapter, dashboard, event forwarding, and notifications.
6. Reserve and implement the Luna Mux control service, then add Luna MCP for
   cross-agent operations using that boundary.
7. Add Session-level managed Chrome resources, CDP tools, and remote-service forwarding.
8. Continue selective upstream ports and cross-platform regression work throughout development.

The detailed executable work breakdown and durable status are maintained in `DEVELOPMENT_TASKS.md` and `DEVELOPMENT_PROGRESS.md`.
