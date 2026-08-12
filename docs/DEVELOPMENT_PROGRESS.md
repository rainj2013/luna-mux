# Luna Mux Development Progress

This is the durable development checkpoint. Update it in every development session that changes implementation status. `DEVELOPMENT_TASKS.md` remains the authoritative task inventory; this file records the current focus, decisions, evidence, and handoff.

## Current checkpoint

- Date: 2026-08-12
- Phase: M1 - Terminal runtime abstraction, with the M4.8 control boundary established
- Active task: M1.2 - Wrap existing SSH manager in `InProcessSshTerminalBackend`
- Task progress: 13 of 53 done (24.53%); 1 in progress
- Overall state: M0 foundation and M1 terminal-runtime contract are complete; SSH runtime adaptation is in progress
- Baseline Luna Remote commit: `7ccc61f0bb1ea7ffb74556c1fe207e802bfbfd2f`
- Luna Mux repository: `D:\code\luna-mux`
- Luna Remote source repository: `D:\code\ssh_tools`

## Completed

- Created an independent Git repository while preserving Luna Remote history.
- Configured Luna Remote as fetch-only `upstream`; disabled upstream push and branch tracking.
- Confirmed the Luna Remote source working tree was clean before cloning.
- Locked product name and identifiers to Luna Mux / `luna-mux` / `com.luna.mux`.
- Added `product/product.json` and generated identity entry points for frontend, Rust, manifests, HTML, and release workflow metadata.
- Isolated the database and credential namespace; startup performs no automatic Luna Remote data or credential migration.
- Preserved Luna Remote and older connection archive formats only as explicit file-import compatibility.
- Added the approved design, ordered development tasks, progress checkpoint, and upstream sync ledger.
- Removed four unreferenced Luna Remote design and product screenshots (about 5.7 MB); retained only image assets still required by the current build.
- Added a two-phase connection-backup import: read-only preview, source identification, selective connection/empty-group choice, dependency-aware jump-host selection, and one SQLite transaction. Credentials are explicitly excluded.
- Added versioned Terminal Runtime contract metadata with generated Rust/TypeScript types for targets, capabilities, lifecycle, UTF-8 byte cursors, truncation reads, and exit events.
- Added the thin `TerminalBackend` trait and a lightweight Rust contract test crate; all three status/output/exit contract tests pass without requiring the native SSH dependency toolchain.
- Installed NASM 3.02 from the official Windows archive, added it to the user PATH, and restored full native Rust compilation through `aws-lc-sys`.
- Completed the Luna Remote import wizard with automatic standard-path discovery, manual database selection, read-only SQLite snapshot validation, and selective connection/group/Host Key/portable-setting/forwarding-profile import.
- Added explicit, default-off credential copying through the system credential stores with a second confirmation, new connection IDs, dependency-aware jump-host/forwarding rewrites, transactional database writes, and credential cleanup on failed imports.
- Replaced inherited SSH, Classic, and Neon icon assets with an original Luna Mux family: Luna, Graphite, Signal, and Light. SVG masters feed a repeatable Tauri generation command; `npm run icons:check` compares deterministic PNG/ICO output and validates the packaged ICNS structure.
- Renamed the new transport boundary from the ambiguous session terminology to `TerminalBackend` and `TerminalRuntime`. The project-level concept is now explicitly `MuxSession`, with one Mux Session containing multiple persistent panes and each Terminal pane owning at most one live runtime.
- Added a transport-neutral `LunaControlService` boundary for future Agent, MCP, CLI, and local automation adapters. The initial implementation exposes a trusted UI adapter, a versioned operation catalog, scoped grants, structured errors, and a bounded runtime event cursor without implementing an external MCP server.
- Made the shared React `TerminalPane` transport-independent. It now binds by `runtimeId`, consumes normalized runtime output events, uses runtime flow-control/resize/write APIs, and catches up from the bounded output cursor when a pane mounts.
- Added a mock `TerminalBackend` contract suite covering target capabilities, runtime lifecycle, UTF-8 output cursors, writes, resize, flow control, interrupt, list, and close.
- Documented the future daemon backend extension point as a transport-neutral runtime contract; no daemon process, persistence, or IPC wire protocol is implemented.

## In progress

- Existing SSH connections are not yet fully regression-validated through `InProcessSshTerminalBackend`; M1.2 is now active.
- M4.8 is complete as a foundation task: only read-only terminal target/runtime discovery and event reads are implemented. No external Agent transport or mutating Agent operation is enabled yet.
- The existing Luna Remote-derived `SessionManager` and non-terminal UI `sessionId` fields remain compatibility-bound implementation names for SFTP, tunnels, transfers, and Host Key prompts; they do not define the new Mux Session domain model.

## Next actions

1. Wrap the existing SSH manager as `InProcessSshTerminalBackend` under M1.2 without changing current SSH behavior.
2. Select and spike the cross-platform PTY library under M2.1 after the SSH runtime boundary is stable.
3. Extend the control catalog and add an authenticated local Agent adapter before exposing write/control operations.

## Verification evidence

| Check | Result | Date |
| --- | --- | --- |
| Source repository clean before clone | Passed | 2026-08-12 |
| New repository retains baseline history | Passed (`7ccc61f`) | 2026-08-12 |
| Upstream push disabled | Passed | 2026-08-12 |
| `npm run product:check` | Passed | 2026-08-12 |
| `npm run runtime:check` | Passed; terminal runtime contract v3 generated outputs match | 2026-08-12 |
| `npm run runtime:test` | Passed (3 tests; runtime status/output/exit) | 2026-08-12 |
| `npm run icons:check` | Passed; four SVG-derived PNG variants and Luna ICO/ICNS validated | 2026-08-12 |
| `npm run native-core:test` | Passed (7 tests, including read-only Luna Remote snapshot and transactional data-class import) | 2026-08-12 |
| `npm run i18n:check` | Passed | 2026-08-12 |
| `npm run typecheck` | Passed | 2026-08-12 |
| `npm run web:build` | Passed | 2026-08-12 |
| `cargo metadata --manifest-path app/native/Cargo.toml --no-deps` | Passed | 2026-08-12 |
| `cargo fmt --manifest-path app/native/Cargo.toml -- --check` | Passed | 2026-08-12 |
| `cargo check --manifest-path app/native/Cargo.toml` | Passed with NASM 3.02; `aws-lc-sys`, `russh`, and Luna Mux compiled successfully | 2026-08-12 |
| `cargo test --manifest-path app/native/Cargo.toml` | Passed (31 tests, including icon compatibility and image validation) | 2026-08-12 |
| `cargo test --manifest-path tests/native-core/Cargo.toml` | Passed (7 tests) | 2026-08-12 |
| `git diff --check` | Passed | 2026-08-12 |
| Luna Mux icon visual inspection | Passed at 512, 64, 32, and 16 px for Luna, Graphite, Signal, and Light variants | 2026-08-12 |
| Browser visual verification | Not run: browser runtime reported no available in-app or Chrome instance | 2026-08-12 |
| `cargo check --manifest-path app/native/Cargo.toml` after control boundary | Passed with structured control service and event buffer | 2026-08-12 |
| `cargo test --manifest-path app/native/Cargo.toml control_service` | Passed (7 control service tests) | 2026-08-12 |
| `npm run typecheck` after TerminalPane runtime migration | Passed | 2026-08-12 |
| `npm run i18n:check` after runtime terminology update | Passed; 420 keys | 2026-08-12 |
| `cargo test --manifest-path app/native/Cargo.toml terminal_backend` | Passed; mock backend lifecycle and UTF-8 cursor suite | 2026-08-12 |
| `cargo test --manifest-path app/native/Cargo.toml` after TerminalPane/backend contract work | Passed (45 tests) | 2026-08-12 |

## Decision log

| Date | Decision |
| --- | --- |
| 2026-08-12 | Luna Mux is an independent product and repository, not a Luna Remote feature branch. |
| 2026-08-12 | Luna Remote is only an implementation and proven-capability donor. Luna Mux does not preserve its architecture, navigation, terminology, data model, or UX unless each part independently fits the Luna Mux design. |
| 2026-08-12 | Windows and macOS are both first-release platforms. Distribution signing is out of scope. |
| 2026-08-12 | Local and SSH terminals share one TerminalPane and differ only through capabilities. |
| 2026-08-12 | Agents stop when Luna Mux exits; a thin `TerminalBackend` preserves a future daemon extension point for live runtimes. |
| 2026-08-12 | Browser runs locally; remote development services use SSH local forwarding. |
| 2026-08-12 | Luna Remote updates are selectively ported by commit into in-repository core boundaries. |
| 2026-08-12 | Unreferenced Luna Remote screenshots and design images are removed; build-required icons remain temporarily until Luna Mux visual identity is designed. |
| 2026-08-12 | Terminal Runtime output cursors count UTF-8 bytes and are capped at JavaScript's maximum safe integer so Rust and TypeScript retain exact cursor values. |
| 2026-08-12 | Connection backups use an in-memory preview snapshot and a single SQLite transaction; source files are never mutated and credentials are excluded. |
| 2026-08-12 | Direct Luna Remote import opens SQLite with read-only/query-only flags, allowlists portable settings, clears source-private background paths, and requires a second confirmation before reading system credentials. |
| 2026-08-12 | `MuxSession` is the project-level persistent container; it owns multiple panes. A Terminal pane owns one replaceable `TerminalRuntime`, and an optional Coding Agent runs inside that runtime. `runtimeId` is used for transport events; `sessionId` is not the new domain term. |
| 2026-08-12 | Luna Mux application icons use editable SVG masters and one Tauri generation command; PNG/ICO are byte-checked, ICNS structure is validated, and old SSH-centric files are removed from the independent repository. |
| 2026-08-12 | Agent operation of Luna Mux is a separate control-plane concern from agent-to-agent MCP. `LunaControlService` is the sole core entry point; future transports authenticate callers and inject scoped grants. |
| 2026-08-12 | A mock terminal backend is the conformance fixture for local PTY and future daemon implementations; TerminalPane and Mux Session code must depend only on `TerminalBackend`, not on SSH internals. |

## Development update protocol

For every future implementation session:

1. Read this file and the active milestone in `DEVELOPMENT_TASKS.md` before editing.
2. Mark exactly one primary task `IN_PROGRESS` unless parallel work is explicitly recorded. Current primary task: M1.2.
3. Record material decisions immediately in the decision log.
4. Add verification commands and outcomes before marking a task `DONE`.
5. Update Current checkpoint, Completed, In progress, and Next actions before ending the session.
6. Never mark work complete solely because code was written; acceptance evidence is required.
