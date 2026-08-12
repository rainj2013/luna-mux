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
| M0.6 | TODO | Add explicit Luna Remote import wizard | Selective snapshot import; credentials require separate approval; source is never mutated |
| M0.7 | DONE | Document design, tasks, progress, and upstream policy | All four durable documents exist and cross-reference each other |
| M0.8 | TODO | Replace inherited SSH-centric product icons | Luna Mux has an original default icon and intentional selectable variants for Windows and macOS |

## M1 - Session abstraction

| ID | Status | Task | Acceptance |
| --- | --- | --- | --- |
| M1.1 | TODO | Define session IDs, targets, capabilities, events, and cursor semantics | Rust and TypeScript contracts agree; contract tests cover status/output/exit |
| M1.2 | TODO | Wrap existing SSH manager in `InProcessSshBackend` | Existing SSH, resize, flow, close, SFTP, and tunnels pass regression tests |
| M1.3 | TODO | Make `TerminalPane` transport-independent | No SSH-specific decision exists in the terminal rendering component |
| M1.4 | TODO | Add a mock backend contract suite | A backend can be substituted without changing workspace or TerminalPane |
| M1.5 | TODO | Document future daemon extension contract | Scope is limited to backend behavior; no IPC protocol is implemented |

## M2 - Cross-platform local PTY

| ID | Status | Task | Acceptance |
| --- | --- | --- | --- |
| M2.1 | TODO | Select and spike the PTY library on Windows and macOS | Unicode, resize, interrupt, exit code, and sustained output work on both platforms |
| M2.2 | TODO | Implement Windows PowerShell target | Same TerminalPane behavior as SSH; child tree is cleaned on close |
| M2.3 | TODO | Implement Windows WSL discovery and targets | Installed distributions are selectable; cwd and command launch are predictable |
| M2.4 | TODO | Implement macOS login-shell target | Shell is resolved without hardcoding zsh; process group cleanup works |
| M2.5 | TODO | Normalize terminal capabilities and target labels | Only remote tools differ; terminal controls and appearance remain shared |
| M2.6 | TODO | Add active-process close and application-exit confirmation | No managed PTY process remains after confirmed close or exit |

## M3 - Workspace and split panes

| ID | Status | Task | Acceptance |
| --- | --- | --- | --- |
| M3.1 | TODO | Define Workspace, Pane, SplitNode, and launch-profile schema | Schema supports Terminal/Browser leaves and recursive branches |
| M3.2 | TODO | Add SQLite migration and repository APIs | Existing Luna Mux data migrates transactionally; layouts round-trip |
| M3.3 | TODO | Implement horizontal/vertical splitting and resize | Ratios remain stable and panes never overlap at supported window sizes |
| M3.4 | TODO | Implement focus, move, maximize, restart, and close | Keyboard and pointer workflows work on Windows and macOS |
| M3.5 | TODO | Restore layout without launching processes | Restart restores configuration with every runtime pane stopped |
| M3.6 | TODO | Reframe SFTP/tunnels/transfers as selected-target tools | Existing features remain available without becoming pane types |

## M4 - Codex adapter and dashboard

| ID | Status | Task | Acceptance |
| --- | --- | --- | --- |
| M4.1 | TODO | Implement Luna-managed Codex launch profiles | Every managed process has stable workspace/pane/session/agent IDs |
| M4.2 | TODO | Implement local structured hook receiver | Working, waiting, completed, and error transitions are deterministic |
| M4.3 | TODO | Implement previewable adapter install/uninstall | Existing user hooks are merged and backed up, never overwritten wholesale |
| M4.4 | TODO | Implement remote hook upload and reverse forwarding | Events are scoped by SSH connection, session, and random token |
| M4.5 | TODO | Build Agent dashboard and unread history | Filter, focus, input, interrupt, restart, and mark-read work |
| M4.6 | TODO | Add Windows/macOS system notifications | Focused pane suppresses duplicate OS notification; in-app history remains |
| M4.7 | TODO | Add terminal-only fallback signals | OSC/BEL/activity are labeled heuristic and never presented as authoritative state |

## M5 - Cross-agent Luna MCP

| ID | Status | Task | Acceptance |
| --- | --- | --- | --- |
| M5.1 | TODO | Implement scoped in-process MCP transport | Local and SSH-managed Codex can connect only with a session token |
| M5.2 | TODO | Add agent list/status/task/interrupt tools | Only authorized managed agents are discoverable |
| M5.3 | TODO | Add bounded output ring and cursor reads | Incremental reads and truncation semantics pass contract tests |
| M5.4 | TODO | Add directional read/write/control grants | Grants are explicit, visible, persistent, and immediately revocable |
| M5.5 | TODO | Add raw PTY write | Authorized input is delivered exactly once to the selected live session |
| M5.6 | TODO | Add 30-day audit retention and cleanup | Caller, target, operation, input, result, and timestamp are queryable and removable |

## M6 - Browser pane

| ID | Status | Task | Acceptance |
| --- | --- | --- | --- |
| M6.1 | TODO | Discover and launch Chrome on Windows/macOS | CDP binds to loopback on a random port; managed processes close with Luna Mux |
| M6.2 | TODO | Implement isolated workspace profiles | Login state persists per workspace; temporary profiles are removable |
| M6.3 | TODO | Implement CDP screencast pane | Frames, viewport, mouse, keyboard, wheel, and navigation are interactive |
| M6.4 | TODO | Add local forwarding for remote dev services | Browser uses loopback URL; SSH disconnect is visible and recoverable |
| M6.5 | TODO | Implement browser MCP tools | Navigate, snapshot, click, type, press, scroll, evaluate, screenshot, and wait work |
| M6.6 | TODO | Add external Chrome takeover | User can focus the real Chrome window for unsupported embedded interactions |

## M7 - Continuous compatibility

| ID | Status | Task | Acceptance |
| --- | --- | --- | --- |
| M7.1 | TODO | Establish Windows/macOS CI checks | Typecheck, Rust tests, build checks, product metadata, and i18n run on both platforms |
| M7.2 | TODO | Add terminal parity scenarios | PowerShell, WSL, macOS shell, and SSH share the same behavioral suite |
| M7.3 | TODO | Review Luna Remote upstream periodically | Sync ledger records ported, pending, and rejected commits with evidence |
| M7.4 | TODO | Run milestone regression gates | A milestone is complete only after applicable Windows and macOS acceptance passes |
