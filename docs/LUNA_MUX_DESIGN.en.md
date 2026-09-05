# Luna Mux Design

Last updated: 2026-09-05

## 1. Product Scope

Luna Mux is a local and remote terminal workspace for coding agents. Project-level Sessions organize terminal Panes, Agents, and browser resources, providing unified terminal operation and controlled collaboration.

The desktop supports Windows and macOS, with local shells, WSL, and SSH as terminal targets. Connection management, file transfers, and port forwarding complement the terminal. Code editing, language services, and graphical Git tools are outside the product scope.

## 2. Domain Model

```text
Application
└── Mux Session
    ├── Pane
    │   └── Terminal Runtime
    │       └── Agent process (optional)
    └── Browser Resource
        └── Browser Runtime (started on demand)
```

| Object | Responsibility and ownership |
| --- | --- |
| Mux Session | Project container owning the root directory, terminal layout, and browser resources; also the collaboration and authorization boundary |
| Pane | Stable layout leaf storing the terminal target, working directory, and title |
| Terminal Runtime | One running instance of a Pane, owning its terminal connection, I/O, and process lifecycle |
| Agent | A detected coding agent process within a Runtime, integrated through an adapter |
| Browser Resource | Persistent browser definition within a Session, separate from the terminal layout |
| Browser Runtime | One running browser instance, owning its process and automation connection |
| Terminal Target | A local, WSL, or saved SSH launch target |

Stable resource IDs link objects. A Pane owns at most one live Runtime; restarting it preserves the Pane identity. Process IDs and display names do not determine authorization.

## 3. Architecture and State Ownership

React and xterm.js present the terminal, while Tauri connects the interface to Rust native services. Responsibilities have three layers:

- **Presentation**: Sessions, terminals, Agent status, and resource management views.
- **Services**: Layout, control authorization, terminal interactions, Agent adapters, and browser resources.
- **Backends**: I/O and lifecycle for local PTYs, SSH, file transfers, tunnels, and browser processes.

The database stores resource definitions and configuration; Runtime backends own live state and output. Interfaces and tools access state through services and contracts without creating parallel sources of truth. Shared core modules do not depend on product UI or branding.

### 3.1 Sessions and Layout

Primary navigation organizes work through the Session and Pane tree, with saved connections serving as reusable launch resources. The Session root supplies the default working directory; Panes can override the target and directory. Agents run in ordinary terminals without a dedicated Pane type.

Layouts are recursive trees of Pane leaves and horizontal or vertical splits with ratios, supporting splitting, resizing, reordering, minimization, and maximization. Updates validate Pane uniqueness and tree structure and serialize concurrent changes. View changes do not restart Runtimes.

App restart restores Sessions, layouts, Pane configuration, and browser definitions; running instances start on demand. Runtime and process identities and terminal scrollback do not survive app restart. Closing active terminals requires confirmation, and app shutdown cleans up managed running instances.

### 3.2 Terminal Backends and Data Flow

Local PTY and SSH backends implement a shared `TerminalBackend` contract for input, output, screen observation, resizing, flow control, interruption, and closing. Backend capabilities determine whether file transfer and forwarding are available; platform differences stay within backends.

```text
Local PTY / SSH
  → Incremental UTF-8 decoding
  → Runtime output buffer and text screen model
  → Events with output cursors
  → TerminalPane / xterm.js
```

Output uses bounded memory and monotonic byte cursors, explicitly reporting gaps when older data is overwritten. The interface resumes from its consumed cursor and applies backpressure by pausing and resuming backend reads. Blocking PTY writes are isolated from asynchronous tasks.

Workspace switches retain terminal instances. Component reconstruction uses display snapshots and output cursors to preserve continuity. xterm.js owns display and terminal query responses; the backend text screen model serves observation without duplicate responses. Rendering falls back to the default renderer if acceleration fails.

Each backend independently owns its connections and process trees, so closing one Runtime does not affect other Panes. Terminal protocols, signals, and process cleanup adapt to the target environment without assuming identical shell behavior across platforms.

### 3.3 Agent Integration

`AgentAdapter` encapsulates provider launch adaptation, Hook/MCP configuration, and state conversion. Runtimes inject Session, Pane, and Runtime context with scoped credentials. Agent identity follows the process lifecycle and is cleared when the process or Runtime exits.

Provider events map to working, waiting, completed, and error states. Luna Mux presents status, sends notifications, and focuses terminals; Agent permission requests remain in the Agent's own interface.

Configuration is scoped to running instances without rewriting global user settings. Remote integration requires explicit enablement. Support files and communication bridges are isolated by Runtime and cleaned up with its lifecycle; ordinary SSH connections do not implicitly install Agent integration.

### 3.4 Control and Terminal Collaboration

`LunaControlService` centralizes application operations, authorization, idempotency, and auditing. The desktop and Luna MCP invoke it through authenticated adapters. Adapters supply caller identity, and operations use versioned contracts and structured errors.

The Session is the default collaboration scope. Agents may observe and operate terminals and Agents within that Session; shared targets or process relationships do not imply cross-Session access. Closing Runtimes and starting transfers or tunnels follow separate approval policies. Resource discovery excludes credentials.

Terminal interaction follows an observe → input → wait for new state → decide loop:

- Input is expressed as text, keys, or paste and encoded for the terminal's modes.
- Raw output supports incremental reads; screen snapshots expose current text, cursor position, and modes.
- Configurable prompt rules describe terminal program interaction states; waits accept only newly produced relevant state.
- Interaction records support deduplication, resumed reads, and cancellation of waits. Input coordination prevents interleaved writes and allows user takeover.

Output, parsing, and waits have resource limits. Interaction records do not copy output history, and deduplication applies within the current app run. Prompt matches and silence only indicate that observation conditions were met; cancelling a wait does not interrupt a process.

### 3.5 Browser Resources

Luna Mux manages browsers in independent desktop Chrome windows, outside the terminal layout. Profiles are isolated by resource. Each Session shares one active Browser Runtime, which can start on the first automation call.

The app owns browser lifecycle; `agent_browser` owns page automation. Automation binds to the Session's existing page and reuses it for ordinary navigation. Page operations cannot independently launch or replace the browser process.

CDP binds only to local loopback, with ports and connection details kept as temporary runtime state. Remote Agents access the Session browser through authenticated Runtime communication bridges; raw CDP is not forwarded remotely. Remote development services use separate SSH tunnels, which browser resources do not own.

Tool routing follows resource ownership: Luna MCP controls application resources, `agent_browser` operates web content, and native tools handle development and host work. Terminal Panes and browser tabs are distinct resources.

## 4. Data and Security Boundaries

`product/product.json` defines product identity centrally, with generated metadata consumed at runtime. The database, system credential service, application settings, and browser directories are isolated from other products.

Persistent resource definitions are separate from temporary runtime state. Configuration migrations are explicit, idempotent transactions. Users select external data imports, with credential imports separately authorized and handled through the system credential store.

The WebView accesses only explicitly registered native capabilities. Credentials remain in the system credential store and are excluded from tool results and logs. Control auditing records caller, target, operation, and result summaries without retaining sensitive input bodies.
