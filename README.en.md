<div align="center">
  <h1>Luna Mux</h1>
  <p>A local and remote terminal workspace for coding agents</p>
  <p><strong>Status: product foundation and design; no usable release yet.</strong></p>
</div>

<div align="center">English · <a href="README.md">简体中文</a></div>

Luna Mux is an independent product and repository derived from Luna Remote. It reuses mature SSH, SFTP, port-forwarding, and terminal foundations while adding cross-platform local terminals, free-form split workspaces, coding-agent status, cross-agent control, and browser validation workflows.

Luna Mux is not a traditional IDE and does not plan to embed a code editor, language server, or Git GUI.

## First-release goals

- Windows PowerShell, WSL, and remote SSH
- macOS login shell and remote SSH
- One xterm.js terminal experience for local and remote sessions
- Persisted workspaces with recursive horizontal and vertical splits
- Codex status, notifications, and controlled cross-agent operations
- A local Chrome browser pane with SSH forwarding for remote development services
- Existing SFTP, transfers, tunnels, and system credential storage

Agents initially share the Luna Mux process lifetime. A thin SessionBackend boundary preserves a future daemon-backed implementation without implementing background sessions now.

## Development references

- [Design](docs/LUNA_MUX_DESIGN.md)
- [Development tasks](docs/DEVELOPMENT_TASKS.md)
- [Development progress](docs/DEVELOPMENT_PROGRESS.md)
- [Luna Remote upstream sync](docs/UPSTREAM_SYNC.md)
- [Development environment](docs/DEVELOPMENT.md)

Product identity is maintained in `product/product.json`. After changing it, run:

```bash
npm run product:sync
npm run product:check
```

## Repository relationship

Luna Mux and Luna Remote are separate applications. This repository retains Luna Remote history for source traceability, but uses independent application identifiers, storage, credentials, and release workflows. Upstream changes are reviewed and ported selectively by commit.

## License

This project retains the [MIT License](LICENSE). Third-party components and fonts keep their respective licenses.
