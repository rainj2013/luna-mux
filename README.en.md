<div align="center">
  <h1>Luna Mux</h1>
  <p>A local and remote terminal workspace built for coding agents</p>
  <p><strong>Current status: the project is in its design and product foundation stage and is not yet ready for general use.</strong></p>
</div>

<div align="center"><a href="README.md">简体中文</a> · English</div>

Luna Mux is a new product and independent codebase spun out of Luna Remote. It builds on proven SSH, SFTP, port-forwarding, and terminal capabilities while adding cross-platform local terminals, flexible split layouts, coding-agent status management, cross-agent control, and browser-based verification workflows.

Luna Mux is not a traditional IDE. It does not plan to include a built-in code editor, language services, or a Git GUI.

## Initial release goals

- Windows: PowerShell, WSL, and remote SSH
- macOS: the local login shell and remote SSH
- One shared xterm.js terminal interface for both local and remote environments
- Persistent workspaces with recursive horizontal and vertical splits
- Codex status, notifications, and controlled cross-agent operations
- Local Chrome browser resources, with remote development services accessed through SSH forwarding
- Retained support for SFTP, file transfers, port forwarding, and system credential storage

For the initial release, agent lifecycles are tied to the Luna Mux application. Agents stop when the application exits, while the `SessionBackend` boundary remains available for a future background-daemon implementation.

## Development resources

- [Full design](docs/LUNA_MUX_DESIGN.md)
- [Step-by-step development tasks](docs/DEVELOPMENT_TASKS.md)
- [Current development progress](docs/DEVELOPMENT_PROGRESS.md)
- [Luna Remote upstream synchronization policy](docs/UPSTREAM_SYNC.md)
- [Development environment](docs/DEVELOPMENT.md)

The product name and technical identifiers are maintained centrally in `product/product.json`. After making changes, run:

```bash
npm run product:sync
npm run product:check
```

## Repository relationship

Luna Mux and Luna Remote are separate applications. This repository retains Luna Remote's Git history so the origin of code can be traced, but it has its own application identifiers, database, credential namespace, and release process. Future Luna Remote updates are synchronized through commit-level review and selective cherry-picking.

## License

Luna Mux is released under the [MIT License](LICENSE). Third-party components and fonts retain their respective licenses.
