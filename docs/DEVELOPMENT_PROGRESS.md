# Luna Mux Development Progress

This is the durable development checkpoint. Update it in every development session that changes implementation status. `DEVELOPMENT_TASKS.md` remains the authoritative task inventory; this file records the current focus, decisions, evidence, and handoff.

## Current checkpoint

- Date: 2026-08-12
- Phase: M0 - Independent product foundation
- Active task: M0.6 - Complete the explicit Luna Remote import wizard
- Overall state: implementation started; connection import and session contracts are in place
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
- Added versioned session contract metadata with generated Rust/TypeScript types for targets, capabilities, lifecycle, UTF-8 byte cursors, truncation reads, and exit events.
- Added the thin `SessionBackend` trait and a lightweight Rust contract test crate; all three status/output/exit contract tests pass without requiring the native SSH dependency toolchain.

## In progress

- M0.6 still needs a direct Luna Remote data-source reader and selective Host Key/settings/forwarding-profile import. Credential import remains a separate, explicit authorization step.
- Existing SSH sessions are not yet routed through `SessionBackend`; that work is M1.2.
- Full native Rust compilation is waiting on the Windows native build prerequisite described below.

## Next actions

1. Complete M0.6 with direct read-only Luna Remote source discovery/selection and import of the remaining approved data classes.
2. Install or configure NASM, then run full `cargo check` and native Rust tests on Windows.
3. Wrap the existing SSH manager as `InProcessSshBackend` under M1.2 without changing current SSH behavior.
4. Design and replace the inherited SSH-centric application icon set under M0.8 before packaging a Luna Mux release.

## Verification evidence

| Check | Result | Date |
| --- | --- | --- |
| Source repository clean before clone | Passed | 2026-08-12 |
| New repository retains baseline history | Passed (`7ccc61f`) | 2026-08-12 |
| Upstream push disabled | Passed | 2026-08-12 |
| `npm run product:check` | Passed | 2026-08-12 |
| `npm run session:check` | Passed | 2026-08-12 |
| `npm run session:test` | Passed (3 tests) | 2026-08-12 |
| `npm run native-core:test` | Passed (6 tests, including transactional archive import) | 2026-08-12 |
| `npm run i18n:check` | Passed | 2026-08-12 |
| `npm run typecheck` | Passed | 2026-08-12 |
| `npm run web:build` | Passed | 2026-08-12 |
| `cargo metadata --manifest-path app/native/Cargo.toml --no-deps` | Passed | 2026-08-12 |
| `cargo fmt --manifest-path app/native/Cargo.toml -- --check` | Passed | 2026-08-12 |
| `cargo check --manifest-path app/native/Cargo.toml` | Blocked in third-party `aws-lc-sys`: NASM is unavailable; project Rust code was not reached | 2026-08-12 |
| `git diff --check` | Passed | 2026-08-12 |
| Browser visual verification | Not run: no in-app or Chrome browser instance was available; native Tauri launch is also blocked by NASM | 2026-08-12 |

## Decision log

| Date | Decision |
| --- | --- |
| 2026-08-12 | Luna Mux is an independent product and repository, not a Luna Remote feature branch. |
| 2026-08-12 | Windows and macOS are both first-release platforms. Distribution signing is out of scope. |
| 2026-08-12 | Local and SSH terminals share one TerminalPane and differ only through capabilities. |
| 2026-08-12 | Agents stop when Luna Mux exits; a thin SessionBackend preserves a future daemon extension point. |
| 2026-08-12 | Browser runs locally; remote development services use SSH local forwarding. |
| 2026-08-12 | Luna Remote updates are selectively ported by commit into in-repository core boundaries. |
| 2026-08-12 | Unreferenced Luna Remote screenshots and design images are removed; build-required icons remain temporarily until Luna Mux visual identity is designed. |
| 2026-08-12 | Session cursors count UTF-8 bytes and are capped at JavaScript's maximum safe integer so Rust and TypeScript retain exact cursor values. |
| 2026-08-12 | Connection backups use an in-memory preview snapshot and a single SQLite transaction; source files are never mutated and credentials are excluded. |

## Session update protocol

For every future implementation session:

1. Read this file and the active milestone in `DEVELOPMENT_TASKS.md` before editing.
2. Mark exactly one primary task `IN_PROGRESS` unless parallel work is explicitly recorded.
3. Record material decisions immediately in the decision log.
4. Add verification commands and outcomes before marking a task `DONE`.
5. Update Current checkpoint, Completed, In progress, and Next actions before ending the session.
6. Never mark work complete solely because code was written; acceptance evidence is required.
