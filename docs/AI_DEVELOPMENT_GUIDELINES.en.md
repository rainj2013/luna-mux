# Luna Mux AI-Assisted Development Guidelines

## Purpose and Scope

This is the full workflow for high-risk changes: cross-module features, shared contracts, persistence, process lifecycles, security boundaries, and platform integration. Documentation, styling, and localized changes that do not affect shared state follow the fast path in `AGENTS.md` and do not require this document.

The principle is simple: establish checkable architecture assumptions before implementation, use tests and diffs as evidence, and have an independent context review risk. AI-generated code and human-written code meet the same merge bar.

## 1. Change Levels

Classify the change first. L0/L1 use the fast path; L2/L3 follow the remaining sections:

| Level | Typical scope | Minimum verification |
| --- | --- | --- |
| L0 Documentation/style | docs, copy, pure CSS, no behavior change | Inspect diff; run `npm run web:build` when relevant |
| L1 Local behavior | one component, pure function, non-shared UI state | Narrowest relevant check or test, usually `npm run typecheck` |
| L2 Shared contract | Tauri command, Rust module, Runtime/Control contract, database model, MCP/Hook | `npm run check`, `npm test`, and relevant contract/native-core tests |
| L3 Runtime/platform | PTY, SSH/SFTP, Agent injection, browser, lifecycle, permissions, installers | All L2 checks + end-to-end scenario + affected native platform |

When uncertain, use the higher level. Cross-module calls, persistence formats, public JSON, process lifecycles, and security boundaries are at least L2.

## 2. L2/L3 Before Coding: Define Architecture Acceptance

Do not ask AI to simply “implement the issue”. Write a short plan answering the items relevant to the change; write “none” for the rest:

1. **Boundary:** Which layer owns the change and which modules may be modified?
2. **Data ownership:** Which manager/database/contract owns session, pane, Runtime, connection, Agent, transfer, or tunnel state? What is the single source of truth?
3. **Call chain:** How does the request travel through React -> Tauri command -> service/backend -> external process and return through events or values? Where are errors translated and recorded?
4. **Compatibility:** Must old APIs, JSON, database records, configuration, Hook/MCP clients, or extensions continue to work? What is the migration or fallback strategy?
5. **Platform differences:** Do macOS, Windows PowerShell 5.1/7, WSL, or SSH differ in stdin/stdout, signals, paths, PTY sizes, or quoting?
6. **Non-goals:** What is explicitly out of scope?

Acceptance criteria must be verifiable, such as “closing a Runtime requires desktop confirmation”, “missing legacy fields use defaults”, or “remote commands are not reparsed by a local shell”.

## 3. AI Work Protocol

For L2/L3 changes:

1. Read `AGENTS.md`, relevant design sections, and relevant code. Do not read every document by default. First report a short impact, ownership, call-chain, and test plan.
2. Modify code only after the plan is clear. Reuse existing managers, backends, contracts, i18n, and platform helpers; do not create parallel state or implementations.
3. Run the smallest relevant test after each logical block.
4. Do not treat compilation as completion. Prove behavior, boundaries, failures, compatibility, and side effects against the acceptance criteria.
5. AI must not expand scope, delete existing tests, bypass confirmation, expose credentials, or change test expectations merely to make tests pass.

## 4. Implementation Constraints

### Modules and Dependencies

- The frontend communicates with native code through stable Tauri APIs/events; components must not copy database or Runtime state.
- Keep Rust dependencies one-way: commands translate boundaries, services/managers orchestrate, backends perform I/O, and contracts define cross-process shapes.
- Put new concepts in the module that owns their lifecycle and data. Do not bypass managers or create a second cache for the same entity.
- Changes to public JSON, Runtime/Control contracts, database fields, or MCP schemas must update sync/generation scripts, consumers, and compatibility tests together.

### Cross-Platform Runtime

- Terminal, PTY, Agent, Hook, MCP, browser, WSL, and SSH changes must follow the cross-platform requirements in `AGENTS.md` and reuse target-specific helpers.
- Do not assume stdin closes, pipes reach EOF when a child exits, or signals and paths behave identically. Use bounded reads, files, or complete JSON/line parsing.
- Lifecycle changes must describe startup, shutdown, crashes, app exit, and idempotency. Keep user confirmation for important side effects.
- Credentials, private keys, API keys, and raw CDP ports must not enter logs, MCP responses, event payloads, or test fixtures.

### Frontend and Localization

- Use stable i18n keys instead of inline copy; update every locale and run `npm run i18n:check`.
- Keep one source for shared UI state. Clean up event subscriptions on unmount and handle stale async results and errors.

## 5. Tests and Evidence

Choose applicable scenarios based on risk; do not mechanically cover unrelated cases:

| Scenario | Evidence |
| --- | --- |
| Happy path | Unit/component test or repeatable end-to-end steps |
| Boundaries and empty values | Explicit input, expected output, and error type |
| Failure and cancellation | Process, network, permission, timeout, repeat-call, and app-exit cases |
| Compatibility | Read/migration tests for old JSON, config, API, or database records |
| Architecture | Dependency direction, single ownership, contract fields, and schema checks |
| Platform | Affected macOS, Windows, PowerShell, WSL, or SSH target, or a recorded coverage risk |

```text
L0: npm run web:build when relevant
L1: npm run typecheck + relevant tests
L2: npm run check + npm test + relevant runtime/native-core tests
L3: L2 + end-to-end flow + affected native platform verification
```

## 6. Risk-Based Independent Review

L3 requires a fresh AI context or human reviewer. L2 requires one only when it changes more than three modules, a public contract/migration, a security boundary, or concurrency behavior. Other L2 changes need a focused diff review by the implementer. L0/L1 do not require independent review.

The independent reviewer receives only the requirement, acceptance criteria, diff, and test results, and checks the applicable dimensions:

- Module boundaries and dependency direction (25%)
- Data ownership, cohesion, and coupling (20%)
- Compatibility of old APIs/JSON/configuration/extensions (15%)
- Locality and maintainability of the change (15%)
- Failure, concurrency, remote/cluster behavior, and operability (15%)
- Coverage of happy path, boundaries, failures, and compatibility (10%)

Turn every “possible issue” into a reproduction step, code location, or missing test. Do not mark it passed without evidence. After fixing findings, recheck the diff scope.

## 7. PR and Merge Gate

L2/L3 PRs must state the change boundary, data ownership/call chain, compatibility and platform impact, test results, uncovered risks, and the independent review conclusion when triggered. L0/L1 PRs only need the change and verification result. Do not merge when acceptance criteria lack implementation/test mapping, required checks fail, contracts/migrations/locales are unsynchronized, platform risk is unverified and undocumented, security confirmation is bypassed, sensitive data is exposed, or unrelated refactoring is unexplained.

## 8. Maintain the Guidelines

When a regression or review finds a recurring pattern, add an acceptance criterion, test template, or rule while fixing the code. Keep the guidelines aligned with the architecture; when documentation conflicts with executable contracts or tests, fix the documentation in the same change.
