# Luna Mux Development Tasks

This file is the ordered, durable work breakdown. Task IDs do not change after work begins. Status values are `TODO`, `IN_PROGRESS`, `BLOCKED`, and `DONE`.

## M0 - Independent product foundation

| ID | Status | Task | Acceptance |
| --- | --- | --- | --- |
| M0.1 | DONE | Create an independent repository with Luna Remote history | Repository exists at `D:\code\luna-mux`; source repository is unchanged |
| M0.2 | DONE | Configure Luna Remote as fetch-only `upstream` | No branch tracks upstream; push URL is disabled |
| M0.3 | DONE | Add product metadata single source | Metadata validates and generates frontend/Rust identity constants and manifests |
| M0.4 | DONE | Isolate runtime identity and storage | Bundle, package, database, credential, title, diagnostics, and backup identifiers use Luna Mux values |
| M0.5 | DONE | Replace remaining brand literals with generated product info | Runtime and build code contain no editable Luna Mux brand duplicates |
| M0.6 | DONE | Add explicit Luna Remote import wizard | Selective snapshot import; credentials require separate approval; source is never mutated |
| M0.7 | DONE | Document design, tasks, progress, and upstream policy | All four durable documents exist and cross-reference each other |
| M0.8 | DONE | Replace inherited SSH-centric product icons | Luna Mux has an original default icon and intentional selectable variants for Windows and macOS |

## M1 - Terminal runtime abstraction

| ID | Status | Task | Acceptance |
| --- | --- | --- | --- |
| M1.1 | DONE | Define runtime IDs, targets, capabilities, events, and cursor semantics | Rust and TypeScript contracts agree; contract tests cover status/output/exit without using project-level session terminology |
| M1.2 | IN_PROGRESS | Wrap existing SSH manager in `InProcessSshTerminalBackend` | Existing SSH, resize, flow, close, SFTP, and tunnels pass regression tests; new APIs use `runtimeId` |
| M1.3 | DONE | Make `TerminalPane` transport-independent | No SSH-specific decision exists in the terminal rendering component; it binds by `runtimeId` |
| M1.4 | DONE | Add a mock terminal backend contract suite | A backend can be substituted without changing Mux Session, pane, or TerminalPane behavior |
| M1.5 | DONE | Document future daemon terminal extension contract | Scope is limited to runtime backend behavior; no IPC protocol is implemented |

## M2 - Cross-platform local PTY

| ID | Status | Task | Acceptance |
| --- | --- | --- | --- |
| M2.1 | IN_PROGRESS | Select and spike the PTY library on Windows and macOS | `portable-pty` probe passes Unicode, resize, interrupt, and cleanup on Windows; the same probe must pass on macOS before completion |
| M2.2 | DONE | Implement Windows PowerShell 7 target | `pwsh.exe` is required and exposed as the PowerShell 7 target; same TerminalPane behavior as SSH; child tree is cleaned on close |
| M2.3 | IN_PROGRESS | Implement Windows WSL discovery and targets | Installed distributions are selectable; cwd and command launch are predictable |
| M2.4 | IN_PROGRESS | Implement macOS zsh/bash target | `$SHELL` or the account shell is accepted only when it is zsh/bash; process group cleanup works |
| M2.5 | DONE | Normalize terminal capabilities and target labels | Only remote tools differ; terminal controls and appearance remain shared |
| M2.6 | DONE | Add active-process close and application-exit confirmation | No managed PTY process remains after confirmed close or exit |

## M3 - Mux Sessions and split panes

| ID | Status | Task | Acceptance |
| --- | --- | --- | --- |
| M3.1 | DONE | Define MuxSession, Pane, SplitNode, and launch-profile schema | One project-centered Mux Session owns recursive Terminal panes; panes may override cwd and target |
| M3.2 | DONE | Add SQLite migration and repository APIs | Existing Luna Mux data migrates transactionally; layouts round-trip |
| M3.3 | IN_PROGRESS | Implement horizontal/vertical splitting and resize | Ratios remain stable and panes never overlap at supported window sizes |
| M3.4 | IN_PROGRESS | Implement focus, layout presets, rename, maximize, and close | Existing runtimes survive layout changes; names and layouts persist; keyboard and pointer workflows work on Windows and macOS |
| M3.5 | DONE | Restore Mux Session layout without launching processes | Restart restores panes and launch configuration with every Terminal Runtime stopped |
| M3.6 | DONE | Reframe SFTP/tunnels/transfers as selected-target tools | Existing features remain available without becoming pane types |
| M3.7 | DONE | Make Session/Pane the exclusive primary navigation | The sidebar contains only the Session/Pane tree; SSH targets live in a secondary resource library |
| M3.8 | DONE | Unify pane creation around ordinary terminals | Add Pane chooses only a local/SSH target; new persisted panes have no Agent-specific type or launch profile; legacy panes still open as terminals |

## M4 - Agent adapters and dashboard

| ID | Status | Task | Acceptance |
| --- | --- | --- | --- |
| M4.1 | DONE | Implement Luna-managed Agent launch profiles | Every managed process is scoped by muxSessionId, paneId, runtimeId, and agentId |
| M4.2 | DONE | Implement local structured hook receiver | Working, waiting, completed, and error transitions are deterministic |
| M4.3 | DONE | Implement previewable adapter install/uninstall | Existing user hooks are merged and backed up, never overwritten wholesale |
| M4.4 | IN_PROGRESS | Implement remote hook upload and reverse forwarding | Events are scoped by SSH connection, terminal runtime, and random token |
| M4.5 | IN_PROGRESS | Build the Agent observation and response center | Needs-response, running, and finished groups plus focus, input, interrupt, restart, grants, and read-state work in a real multi-Agent run |
| M4.6 | IN_PROGRESS | Add Windows/macOS system notifications | Focused pane suppresses duplicate OS notification; in-app history remains |
| M4.7 | DONE | Add terminal-only fallback signals | OSC/BEL/activity are labeled heuristic and never presented as authoritative state |
| M4.13 | IN_PROGRESS | Give every Terminal Runtime scoped Luna context | Local PowerShell/macOS runtimes inherit Session/Pane/Runtime identity plus narrow Hook/MCP credentials; WSL/SSH bridges remain |
| M4.14 | DONE | Register manually started Agents from runtime-local shims and structured Hooks | Process start is visible before the first turn; `SessionStart` binds the provider session; process/Session/Runtime exit retires it; repeated launches leave no duplicate live Agent |
| M4.15 | IN_PROGRESS | Migrate dashboard and grants from Agent panes to live Agents | Dashboard/focus/notifications now follow detected Agent identities; grant persistence still uses source Pane compatibility keys |
| M4.16 | IN_PROGRESS | Extract the AgentAdapter registry and add Claude Code | Codex remains compatible; manual and quick-launch Claude Code share Hook/MCP/Browser infrastructure; Windows, macOS, WSL, and SSH acceptance is recorded |

## M4.8-M4.12 - Agent control of Luna Mux

These tasks let an external AI Agent operate Luna Mux itself. They are
separate from cross-agent collaboration: the caller is an Agent/CLI/automation
client, while the resources are Luna Mux application objects. All transports
must call `LunaControlService`; none may access SQLite, `SessionManager`, or a
terminal backend directly.

| ID | Status | Task | Acceptance |
| --- | --- | --- | --- |
| M4.8 | DONE | Define `LunaControlService` operation catalog and versioned request/response envelopes | Catalog, caller kinds, scoped grants, structured errors, idempotency field, and event cursor semantics are tested |
| M4.9 | DONE | Add authenticated Agent control adapter | A local adapter authenticates the caller and injects `ControlCaller`; caller identity cannot be supplied by request JSON |
| M4.10 | DONE | Expose Mux Session/Pane/Runtime/target discovery and safe control tools | Read-only discovery works first; mutating operations use explicit write/control grants and never expose raw database access |
| M4.11 | DONE | Add approval policy for destructive operations | Close, interrupt, raw input, transfer, tunnel, and browser side effects can require user approval by policy |
| M4.12 | DONE | Add control audit events and event cursor reads | Authorized callers receive bounded events with truncation markers; audit records include caller, resource, operation, result, and timestamp |

## M5 - Cross-agent Luna MCP

| ID | Status | Task | Acceptance |
| --- | --- | --- | --- |
| M5.1 | IN_PROGRESS | Implement scoped in-process MCP transport for agent-to-agent operations | Local and SSH-managed Codex can connect only with a runtime-scoped token; application control reuses M4.8-M4.12 |
| M5.2 | DONE | Add agent list/status/task/interrupt tools | Only authorized managed agents are discoverable |
| M5.3 | DONE | Add bounded output ring and cursor reads | Incremental reads and truncation semantics pass contract tests |
| M5.4 | DONE | Add directional read/write/control grants | Grants are explicit, visible, persistent, and immediately revocable |
| M5.5 | DONE | Add raw PTY write | Authorized input is delivered exactly once to the selected live session |
| M5.6 | DONE | Add 30-day audit retention and cleanup | Caller, target, operation, input, result, and timestamp are queryable and removable |
| M5.7 | IN_PROGRESS | Make Luna MCP available to manually started Agents | PowerShell 7 and macOS local terminals receive a runtime-scoped MCP identity without a generated visible launch command; WSL and SSH delivery remain |

## M6 - Browser resources

| ID | Status | Task | Acceptance |
| --- | --- | --- | --- |
| M6.1 | IN_PROGRESS | Discover and launch Chrome on Windows/macOS | CDP binds to loopback on a random port; managed processes close with Luna Mux |
| M6.2 | IN_PROGRESS | Implement isolated Browser Resource profiles | Login state persists per resource; temporary profiles are removable |
| M6.3 | DONE | Implement Session-level Browser Resource manager | Browser stays outside the split layout; users can create, start on a clean New Tab, focus external Chrome, restart, stop, and delete resources |
| M6.4 | DONE | Keep Browser Resources local and separate remote-service forwarding | New Browser creation has no SSH source; remote services use the independent SSH tunnel workflow; legacy source records remain readable |
| M6.5 | IN_PROGRESS | Replace the custom browser-tool prototype with native `agent-browser` | A real Agent can discover and reuse pages, use stable snapshot refs, click/type/wait, switch existing tabs/popups, inspect console/network, and capture MCP images without launching another browser |
| M6.6 | IN_PROGRESS | Add external Chrome interaction | User can focus and use the real managed Chrome window; Windows is validated and macOS remains pending |
| M6.7 | DONE | Add the Session-aware native Browser MCP wrapper | `luna-mux mcp browser` resolves the current Session endpoint and launches the checksummed `agent-browser` sidecar without Node, `npx`, or an exposed CDP port; `mcp chrome` remains a compatibility alias |
| M6.8 | IN_PROGRESS | Configure ordinary Agent processes for `agent-browser` MCP | Local Codex shim adds the wrapper, preserves user config, initializes without Chrome, starts the unique local Browser Resource and prewarms its pinned daemon on first tool use; WSL/SSH delivery and real Codex browser acceptance remain |
| M6.9 | DONE | Retire custom Agent-facing `browser.*` methods | Luna control APIs remain internally available; selector-based Browser methods are filtered from Agent Luna MCP and duplicate routing instructions are removed |
| M6.10 | IN_PROGRESS | Enforce the one-running-Chrome-per-Session v1 rule | Browser start/UI prevents ambiguous simultaneous runtimes; the MCP wrapper rejects ambiguity and never presents unenforceable per-Agent CDP grants |

## M7 - Continuous compatibility

| ID | Status | Task | Acceptance |
| --- | --- | --- | --- |
| M7.1 | TODO | Establish Windows/macOS CI checks | Typecheck, Rust tests, build checks, product metadata, and i18n run on both platforms |
| M7.2 | TODO | Add terminal parity scenarios | PowerShell 7, WSL, macOS zsh/bash, and SSH share the same behavioral suite |
| M7.3 | TODO | Review Luna Remote upstream periodically | Sync ledger records ported, pending, and rejected commits with evidence |
| M7.4 | TODO | Run milestone regression gates | A milestone is complete only after applicable Windows and macOS acceptance passes |
