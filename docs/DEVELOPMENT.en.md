# Luna Mux Development Guide

Luna Mux is built on Tauri 2, React, xterm.js, and Rust. The first release targets macOS and Windows simultaneously. Installers must be built natively on the corresponding target platform.

All commands in this document are run from the repository root, the directory that contains `package.json`.

## Project Structure

```text
app/
  frontend/   React, xterm.js, and Tauri API bridge layer
  native/     Rust backend, Tauri configuration, and platform capabilities
assets/icons/ app and UI icons
docs/         architecture, development, and design materials
```

See the "Terminal Backend" section of the [design document](LUNA_MUX_DESIGN.en.md) for terminal data flow, PTY, output backpressure, and security boundaries.

## Development Environment

- Node.js 24 and npm, with [fnm](https://github.com/Schniz/fnm) recommended for version management. The `.node-version` in the repo root pins the major version.
- Rust stable 1.85 or later. The project uses the Rust 2024 edition; install via [rustup](https://rustup.rs/).
- macOS requires Xcode Command Line Tools.
- Windows requires Microsoft C++ Build Tools, the Windows SDK, the WebView2 Runtime, and NASM (a native crypto build dependency of `aws-lc-sys`).

### macOS

Install Xcode Command Line Tools.

```bash
xcode-select --install
```

If `fnm` is not installed yet, install it via Homebrew.

```bash
brew install fnm
```

Add the following line to `~/.zshrc` and reopen the terminal. You can also run it directly to initialize the environment for the current terminal.

```bash
eval "$(fnm env --use-on-cd --shell zsh)"
```

Install Rust.

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

You do not need a full Xcode install just to develop the macOS desktop app. `tauri info` may report a missing Xcode; that notice mainly affects iOS development.

### Windows

In PowerShell, install `fnm`, Rust, Visual Studio 2022 Build Tools, and the WebView2 Runtime.

```powershell
winget install --exact --id Schniz.fnm
winget install --exact --id Rustlang.Rustup
winget install --exact --id Microsoft.VisualStudio.2022.BuildTools --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
winget install --exact --id Microsoft.EdgeWebView2Runtime
winget install --exact --id NASM.NASM
```

Reopen PowerShell after installation. Add the following line to your PowerShell `$PROFILE` and run it once in the current terminal.

```powershell
fnm env --use-on-cd --shell powershell | Out-String | Invoke-Expression
```

Make sure Rust uses the default MSVC toolchain. The Visual Studio installer should include "Desktop development with C++", MSVC v143, and the Windows 10/11 SDK.

## Installing Project Dependencies

After obtaining or extracting the source, enter the repository root and run.

```console
fnm install 24
fnm use
rustup default stable
npm ci
```

Verify the toolchain is available.

```bash
node --version
npm --version
rustc --version
cargo --version
```

`node --version` should be `v24.x` and `rustc` should be 1.85 or later. The first dependency install requires access to the npm registry and crates.io.

## Starting the Development Environment

```bash
npm run dev
```

This command starts Vite at `127.0.0.1:1420`, then compiles and launches the Tauri desktop app. The first Rust compile takes noticeably longer than subsequent incremental builds. The environment is ready once the Luna Mux window appears.

To start or build only the web frontend, use the following commands.

```bash
npm run web:dev
npm run web:build
```

Web pages cannot fully use the local database, SSH, SFTP, system credentials, and file dialogs once they leave the Tauri Runtime.

Development startup requires no environment variables or API keys. The AI command assistant is optional and can be configured with an OpenAI-compatible service in "Settings → AI".

> Luna Mux uses its own application identifier `com.luna.mux`, database, and system credential namespace, and never reads or modifies Luna Remote data automatically.

## Internationalization

UI copy is centralized in `app/frontend/src/locales/`; components reference only stable keys and do not store Chinese or English text directly.

- `zh-CN.messages.ts` is the complete baseline message table and defines the TypeScript type of the available keys.
- `en.messages.ts` is the English message table. Variables use `{{name}}`-style placeholders.
- `*.locale.ts` files are auto-discovered language registration files containing the language code, display name, sort order, message table, and that language's help content.
- `help-content.tsx` holds rich-text help content that needs lists, tables, and icons; ordinary UI copy belongs in the message table.

To add a language, copy `en.messages.ts` and `en.locale.ts`, rename them to the new language code, then translate the messages and help content. The settings page shows the new language automatically, and native menus are generated from the same message table, so no React component or Rust language enum changes are needed.

```bash
npm run i18n:check
```

This check verifies that keys, duplicates, and interpolation placeholders match exactly across all languages, and blocks legacy inline `tr()` calls from re-entering components.

## Checks

```bash
npm run check
npm test
npm run web:build
```

`npm run check` validates product metadata, the terminal Runtime contract, icons, i18n, and TypeScript types in sequence, then runs Rust `cargo check`; `npm test` runs the Rust unit tests; `npm run web:build` verifies the frontend production build. All three commands should pass before committing.

## Packaging

Tauri installers must be built natively on the target platform; output lands in `app/native/target/release/bundle/`. Unsigned installers are produced when no certificate is configured. Configure an Apple Developer ID, notarization, and Windows code signing before official distribution.

GitHub Actions and the default development environment use crates.io. Run the following once after the first build or after `package-lock.json` changes.

```bash
fnm use
npm ci
rustup default stable
```

Everyday re-packaging does not require reinstalling dependencies or temporarily editing Cargo configuration.

### macOS

Build the DMG.

```bash
npm run build:mac
```

Verify the DMG.

```bash
hdiutil verify "app/native/target/release/bundle/dmg/Luna Mux_<version>_<arch>.dmg"
```

`<arch>` is usually `aarch64` for Apple Silicon builds and `x64` for Intel builds. Use the actual filename in `bundle/dmg/`.

### Windows

Build the standard Windows x64 NSIS installer (recommended).

```powershell
npm run build:win
```

The standard version does not bundle the WebView2 Runtime. Windows 10/11 usually has WebView2 already; when the target machine lacks it, the installer downloads a bootstrapper online.

Build the compatibility version with the WebView2 Runtime bundled.

```powershell
npm run build:win:webview2
```

The compatibility version is larger and only suits machines that lack WebView2 and cannot go online during installation. The first build on a build machine still needs to download the Microsoft WebView2 offline installer online. `npm run build:win:offline` remains as a compatibility alias for the old command. Verify install, over-install, uninstall, and restart-recovery of both the standard and compatibility versions on real Windows hardware before release.

## GitHub Actions

The `Build and Release` workflow in the repository builds four installers.

- macOS Intel x64 DMG
- macOS Apple Silicon ARM64 DMG
- Windows x64 standard installer (recommended, no extra version suffix in the filename)
- Windows x64 bundled-WebView2 compatibility installer (`with-webview2`)

When the workflow is run manually from the repository's Actions page, build results are kept as temporary artifacts for 14 days. Pushing a version tag starting with `v` also creates a GitHub release of the same name and uploads all installers.

```bash
git tag v1.0.0
git push origin v1.0.0
```

macOS installers use ad-hoc signing and do not require an Apple developer certificate. They are not notarized, so first-time users still need to confirm in "Privacy & Security" settings. Windows installers are unsigned, so the system may show an unknown publisher or SmartScreen prompt.

## FAQ

### Configuring a Cargo Mirror

macOS and Linux use the following commands.

```bash
cp .cargo/config.rsproxy.toml.example .cargo/config.toml
cargo fetch --manifest-path app/native/Cargo.toml
```

Windows PowerShell uses the following commands.

```powershell
Copy-Item .cargo/config.rsproxy.toml.example .cargo/config.toml
cargo fetch --manifest-path app/native/Cargo.toml
```

Restore crates.io.

```bash
rm .cargo/config.toml
```

Windows PowerShell:

```powershell
Remove-Item .cargo/config.toml
```

### Port 1420 Is Occupied

Vite uses the fixed port `1420`. Close the process occupying that port and rerun `npm run dev`.

```bash
lsof -nP -iTCP:1420 -sTCP:LISTEN
```

Windows PowerShell can use the following command.

```powershell
Get-NetTCPConnection -LocalPort 1420 -ErrorAction SilentlyContinue
```

### Windows Fails to Compile or Launch

Reopen the Visual Studio Installer, confirm "Desktop development with C++", MSVC v143, and the Windows SDK are installed, then confirm the WebView2 Runtime and NASM (`nasm -v` runs) are installed. Reopen PowerShell after changing the toolchain.

### macOS Signing or Keychain Prompts

Local `npm run dev` and unsigned builds do not require a distribution certificate. The first time you save a password or API key, macOS may ask to authorize Keychain access; this is normal system credential-store behavior.
