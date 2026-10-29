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

- Participants within a Session share collaboration capabilities without fine-grained isolation. Cross-Session access requires separate authorization.
- Configuration is persisted while live state belongs to its instance. Closing one Runtime should not affect others.
- Credentials remain in the system credential store and are excluded from tool results and logs. Cleanup is restricted to app-owned data.
- Output, interactions, and background tasks have resource limits to support stable long-running operation.
