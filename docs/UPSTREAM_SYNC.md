# Luna Remote Upstream Sync

## Policy

Luna Mux has an independent `origin`. Luna Remote is configured only as the fetch remote named `upstream`. Never push to `upstream`, and do not merge its main branch wholesale.

Review and port updates that affect reusable foundations:

- SSH, SFTP, transfers, tunnels, credentials, and host-key security;
- terminal decoding, output flow control, rendering, and platform behavior;
- database correctness and broadly applicable migration fixes;
- build, dependency, Windows, and macOS fixes.

Do not automatically port Luna Remote-specific navigation, branding, release metadata, or product decisions.

## Port procedure

1. Run `git fetch upstream`.
2. Review commits after the last reviewed SHA.
3. Classify each commit as `ported`, `pending`, or `not-applicable`.
4. Use `git cherry-pick -x <sha>` when the change applies directly.
5. For adapted ports, implement the equivalent change and add these trailers:

```text
Upstream-Repo: luna-remote
Upstream-Commit: <sha>
Upstream-Module: <module>
Adaptation: modified
```

6. Run the affected core tests and Luna Mux regression checks.
7. Update the ledger below in the same change.

## Ledger

| Upstream commit | Status | Module | Luna Mux commit | Notes / verification |
| --- | --- | --- | --- | --- |
| `7ccc61f0bb1ea7ffb74556c1fe207e802bfbfd2f` | baseline | all | inherited | Initial Luna Mux repository ancestry |
