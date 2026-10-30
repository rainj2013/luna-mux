# Luna Mux Design

## 1. Product Scope

Luna Mux is a local and remote terminal workspace for coding agents. Project-level Sessions organize Panes, Agents, and browser resources.

The desktop supports Windows and macOS, with local shells, WSL, and SSH as terminal targets. Connections, file transfers, port forwarding, and database panes support project workflows. Code editing, language services, and graphical Git tools are outside the product scope.

## 2. Core Model

```text
Application
└── Mux Session
    ├── Pane
    │   ├── Terminal Runtime
    │   │   └── Agent process (optional)
    │   └── Database Runtime
    └── Browser Resource
        └── Browser Runtime (started on demand)
```

| Object | Responsibility |
| --- | --- |
| Session | Project container organizing Panes and browser resources; also the collaboration and authorization boundary |
| Pane | Stable layout unit storing its type, configuration, and title |
| Runtime | One running terminal, database, or browser instance, owning connections, processes, and live state |
| Agent | A coding agent running in a terminal, integrated through an adapter |
| Browser Resource | Browser configuration within a Session, independent of the Pane layout |

Resource configuration is separate from running instances. Restoring a workspace preserves the user's arrangement; starting or restarting creates a new Runtime identity. Persisted configuration does not implicitly restore processes or execute commands.

## 3. Architecture

Functional modules:

```text
Luna Mux
├── Workspace: Sessions / Panes / layouts
├── Terminals: local shells / WSL / SSH
├── Agent integration: launch / Hook / MCP / status and collaboration
├── Remote work: connections / SFTP / transfers and deployment / tunnels
├── Browsers: resource management / page automation
├── Databases: connections / queries / data operations
├── AI command assistant
└── Shared services: approvals / auditing / credentials / settings / diagnostics
```

| Layer | Responsibility |
| --- | --- |
| Presentation | React / xterm.js displays the workspace and accepts user input |
| Services | Orchestrate resources, Agent integration, authorization, and collaboration |
| Backends | Own I/O and lifecycle for local PTYs, SSH, databases, file transfers, tunnels, and browsers |

Tauri connects the interface to native services. The database stores resource definitions and configuration; Runtime backends own live state. Views read state through APIs and events. UI and MCP share business rules rather than maintaining separate behavior.

### 3.1 Terminal Backend

Local PTYs and SSH use a shared contract, with platform differences contained in backends. Views and Agents share backend output and screen observation instead of maintaining separate terminal state. The interaction service coordinates input to prevent interleaved automation and allow user takeover.

### 3.2 Agents and Control

Adapters encapsulate provider-specific launch behavior, Hook/MCP integration, and status conversion. Shared services handle collaboration, authorization, and auditing. Agents run in ordinary terminals; runtime credentials are bound to a Runtime and expire with the instance.

Sessions define project collaboration scope. Control requests validate caller and resource permissions, with confirmation policies for side effects. Terminal output is an observation, not proof of command success.

### 3.3 Browsers and Databases

Browser resources are shared within a Session. The app owns browser processes while automation tools operate pages; browser windows remain outside the terminal layout. Browser data is isolated by resource and may be disposable or retained. Remote access uses authenticated communication bridges.

Databases are accessed through Panes. Each running instance owns its connection and query state, and writes require authorization.

## 4. Constraints and Tradeoffs

- Input is expressed as text, keys, or paste and encoded for the terminal's modes.
- Raw output supports incremental reads; screen snapshots expose current text, cursor position, and modes.
- Configurable prompt rules describe terminal program interaction states; waits accept only newly produced relevant state.
- Interaction records support deduplication, resumed reads, and cancellation of waits. Input coordination prevents interleaved writes and allows user takeover.

An interaction records its execution and captures the output cursor before sending input. Retries reuse that record, and timeouts resume observation without resending. Writes can partially complete, so failure cannot be treated automatically as “not executed.” Conflicting automated input returns immediately instead of queuing commands that may become stale; user takeover ends the previous observation.

Raw output preserves temporal order, while the screen model represents current state after overwrites and clearing. They serve incremental reading and state assessment respectively. The model is derived from backend output, making hidden Panes observable independently of display rendering without creating a second output history. Prompt rules remain program-independent and must exclude prompts predating the interaction.

Output, parsing, and waits have resource limits. Interaction records do not copy output history, and deduplication applies within the current app run. A PTY combines input echo, program output, and background output in one stream, so prompt matches and silence establish observation conditions rather than command success. Cancelling a wait does not interrupt a process.

### 3.5 Browser Resources

Browsers are shared at Session scope so users and Agents working on one project use the same page context. The tradeoff is shared browser state without isolated page permissions between participants. Each Session shares one active Browser Runtime, which can start on the first automation call.

Browsers run in independent desktop Chrome windows outside the terminal layout, keeping web tabs separate from terminal Pane layout, dimensions, and lifecycle.

Each Browser Resource owns a Keep browser data setting, off by default. When off, every Runtime uses a separate disposable profile that is deleted after Chrome closes. When on, that Browser Resource reuses one stable profile across Runtime starts in the same Session, retaining cookies, sign-in state, and site storage. The setting can change only while Chrome is stopped; turning it off immediately deletes the retained profile. Neither mode copies cookies from the user's local Chrome. Startup sweeps only disposable profiles left after a crash. Deleting a Browser Resource or Session removes its retained profile and Session downloads. Deletion must remain within the product data directory.

BrowserRuntimeManager uses CDP to make the bound page follow the application theme. With the system theme selected, Chrome follows the OS. Screenshots enable element annotations by default.

Before reusing a retained profile, BrowserRuntimeManager reads native process metadata to check for an unassociated Chrome main process using that exact directory. Matching excludes other resources and Chrome child processes. An occupied profile produces a request to close the old window before preferences are changed or a process is launched, preserving browser data. Cleaning up an exited Runtime only removes disposable directories and does not write retained Preferences first. The probe only reads metadata; it does not adopt or close unassociated windows. Chrome exiting before CDP becomes ready returns an exit error immediately. Readiness probes, target discovery, and bootstrap-tab cleanup share a loopback client with proxies and redirects disabled and a request timeout. HTTP and WebSocket initialization errors do not expose raw endpoints or response contents; WebSocket connections also have a timeout.

The Browser tab uses a sidebar with Overview, Browser settings and Agent tools. Overview shows runtime status, addresses and configuration summaries with shortcuts to their settings. Settings use consistent switches and compact rows; tool descriptions stay brief. Page changes keep the proxy editor mounted and save on blur. Navigation supports arrow keys, Home/End and visible focus. Proxy and profile cards keep equal widths and stack in narrow workspaces.

Browser proxy and profile cards use equal-width grid columns and share the same row height. Browser Resources persist the proxy address and a separate enabled switch. Turning the switch off retains the address and uses system proxy defaults at launch. Switch changes save immediately; addresses save on blur or Enter without a save button. Failed saves show an error and restore the actual switch state. Changes apply on the next browser launch. Existing databases and resource JSON without the enabled field retain their previous configured-proxy behavior.

The app owns browser lifecycle; `agent_browser` owns page automation. Automation binds to the Session's existing page and reuses it for ordinary navigation. Page operations cannot independently launch or replace the browser process.

Each Browser Resource persists an Agent tool-group selection using upstream profiles. Core is always enabled; Network, Debug, Tabs, State/Auth Vault, React, Mobile, and WebMCP are off by default and can be selected by category on the Browser page. Local and remote Agents share the same selection. Saving updates the current Runtime registry, so new MCP connections use it immediately; an already-running Agent must restart to rediscover tools because current clients do not respond to live tool-list changes. Browser Runtime always enables Chrome WebMCP interfaces, but an Agent can discover page-registered tools only when the WebMCP group is enabled. React inspection requires enabling `react-devtools` when opening the page. Luna Mux does not bundle upstream skills because their CLI and named-session guidance conflicts with the managed browser contract.

Injected Agent instructions describe the optional groups and, when Debug is enabled, batch calls, accessibility audits, and screenshot/snapshot comparisons. Core incremental observation and output filtering remain available.

The bundled agent-browser is pinned to 0.39.1, with release SHA-256 checks for Windows x64 and macOS x64/ARM64. Browser MCP config enables `useSystemCa` by default so the CLI's own HTTPS requests trust the operating system certificate store while retaining certificate verification; managed Chrome's certificate settings are unaffected. A tool's `caCert` is a PEM path on the Luna Mux host, including for WSL/SSH Agents. The MCP bridge preserves upstream `structuredContent.response.code`; `outcome_unknown` means an operation may already have executed, so Agents must inspect read-only state before deciding whether to retry. Scoped snapshots can read the selected custom element's Shadow DOM. The experimental Obscura engine is not integrated into the existing Chrome/CDP Runtime.

CDP binds only to local loopback, with ports and connection details kept as temporary runtime state. Remote Agents access the Session browser through authenticated Runtime communication bridges; raw CDP is not forwarded remotely. Remote development services use separate SSH tunnels, which browser resources do not own.

Tool routing follows resource ownership: Luna MCP controls application resources, `agent_browser` operates web content, and native tools handle development and host work. Terminal Panes and browser tabs are distinct resources.

### 3.6 Database Panes

Database panes are a Pane type within a Session. Each pane owns its own Database Runtime and connection Profile. They support SQLite, MySQL/MariaDB, and PostgreSQL, with table and view browsing, column/index/foreign-key inspection, paginated SQL queries, and CSV, JSON, and SQL import/export; SQLite also supports selected schema edits.

Profiles store connection settings and a system credential reference; passwords remain in the system credential store. Panes connect read-only by default, with DML and DDL writes authorized separately per Session. The Runtime holds only the active connection and query state and is cleaned up when the pane closes or the app exits.

## 4. Data and Security Boundaries

`product/product.json` defines product identity centrally, with generated metadata consumed at runtime. The database, system credential service, application settings, and browser directories are isolated from other products.

Persistent resource definitions are separate from temporary runtime state. Configuration migrations are explicit, idempotent transactions. Users select external data imports, with credential imports separately authorized and handled through the system credential store.

The WebView accesses only explicitly registered native capabilities. Credentials remain in the system credential store and are excluded from tool results and logs. Control auditing records caller, target, operation, and result summaries without retaining sensitive input bodies.
- Participants within a Session share collaboration capabilities without fine-grained isolation. Cross-Session access requires separate authorization.
- Configuration is persisted while live state belongs to its instance. Closing one Runtime should not affect others.
- Credentials remain in the system credential store and are excluded from tool results and logs. Cleanup is restricted to app-owned data.
- Output, interactions, and background tasks have resource limits to support stable long-running operation.
