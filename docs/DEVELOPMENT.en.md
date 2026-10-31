# Luna Mux Development Guide

This guide covers environment setup, checks, packaging, and releases for Luna Mux. The project uses Tauri 2, React, xterm.js, and Rust, supports macOS and Windows, and builds installers natively on the target platform.

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

- Node.js 24 and npm, managed by [fnm](https://github.com/Schniz/fnm). Use the version specified in the repository root `.node-version`.
- The current Rust stable toolchain, installed and updated through [rustup](https://rustup.rs/). The project uses the Rust 2024 edition; the compiler must satisfy the locked dependencies.
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

`node --version` should match `.node-version`. Run `rustup update stable` if Rust is too old for the dependencies. The first dependency install requires access to the npm registry and crates.io.

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

Use the change levels in the [AI development guidelines](AI_DEVELOPMENT_GUIDELINES.en.md) to determine verification scope. The following are the common full-check commands; documentation, styling, and local behavior changes need only the checks required for their level.

```bash
npm run check
npm test
npm run web:build
```

`npm run check` validates product metadata, the terminal Runtime contract, icons, and i18n, then runs terminal-related script tests, TypeScript checking, and Rust `cargo check`. `npm test` runs the Rust tests; `npm run web:build` verifies the frontend production build.

## Packaging

Tauri installers must be built natively on the target platform; output lands in `app/native/target/release/bundle/`. Unsigned installers are produced when no certificate is configured. Configure an Apple Developer ID, notarization, and Windows code signing before official distribution.

Complete [Installing Project Dependencies](#installing-project-dependencies) before the first build. Run `npm ci` again after `package-lock.json` changes; routine repackaging does not require reinstalling dependencies. GitHub Actions uses crates.io by default; see the FAQ for local mirror configuration.

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

When building both Windows variants consecutively, compile the native application once and then create each installer:

```powershell
npm run build:win:prepare
npm run bundle:win
npm run bundle:win:webview2
```

`build:win:prepare` runs the frontend and Rust compilation only; the two bundle commands only create NSIS installers, which is useful for repeatedly testing installer configuration locally.

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

### Release Note Guidelines

Release notes explain user-visible features, fixes, and compatibility changes. Include the version heading, a short summary, “Changes and improvements”, installer notes, and a GitHub Compare link from the previous version.

- Group related changes by user task, covering all meaningful features, fixes, and compatibility effects without reproducing commits or implementation details. Do not impose a fixed item count.
- Use UI names and restrained, accurate language. Include technical details only when they affect installation, data, security, or user actions.
- State platform limitations, migration steps, and known constraints. Do not claim unverified benefits.
- Check the notes, version number, and Compare link against the version diff and test results before publishing.

The tag workflow generates notes automatically when no Release exists, but an official release should use reviewed notes. Create the matching draft Release immediately after pushing the tag, or edit the generated body after the workflow finishes. Existing release notes are preserved; a draft is published automatically after all four verified installers are uploaded.

macOS release installers are signed with a Developer ID Application certificate and notarized by Apple. The workflow verifies signatures, notarization tickets, and Gatekeeper before uploading a DMG. Missing credentials or failed verification stop the macOS release; there is no fallback to ad-hoc signing. Windows installers are unsigned, so the system may show an unknown publisher or SmartScreen prompt.

### macOS signing credentials

Create a `macos-release` environment under **Settings → Environments**, restricting deployment sources to the `master` branch and `v*` tags (add separate Branch and Tag rules). Select `master` for manual builds. Add the following environment Secrets; never put them in repository files, ordinary Variables, issues, or logs:

| Secret | Value |
| --- | --- |
| `APPLE_CERTIFICATE` | Base64-encoded encrypted `.p12` containing the Developer ID Application certificate and its private key |
| `APPLE_CERTIFICATE_PASSWORD` | Export password for that `.p12` |
| `APPLE_SIGNING_IDENTITY` | Full certificate name, such as `Developer ID Application: Your Name (TEAMID)` |
| `APPLE_API_KEY` | Key ID of an App Store Connect **individual API key** |
| `APPLE_API_PRIVATE_KEY` | Complete text of that individual key's `.p8` file |

Export the certificate and private key from Keychain Access as a password-protected `.p12`. Generate the Base64 file outside the repository and set Secrets through standard input, keeping their contents out of command arguments and terminal output. For example, with files under `~/keys`:

```bash
openssl base64 -A -in ~/keys/DeveloperIDApplication.p12 -out ~/keys/certificate-base64.txt
gh secret set APPLE_CERTIFICATE --env macos-release < ~/keys/certificate-base64.txt
gh secret set APPLE_CERTIFICATE_PASSWORD --env macos-release
gh secret set APPLE_SIGNING_IDENTITY --env macos-release
gh secret set APPLE_API_KEY --env macos-release
gh secret set APPLE_API_PRIVATE_KEY --env macos-release < ~/keys/ApiKey_EXAMPLE.p8
gh secret list --env macos-release
```

Commands without redirection prompt interactively. `gh secret list` shows names and update times, never values. Base64 is an encoding, not encryption; local files should also be readable only by their owner.

The workflow installs dependencies and compiles before `scripts/release-macos.sh` restores credentials under `$RUNNER_TEMP`, imports a dedicated temporary keychain, and bundles the app. This flow uses an individual API key and explicitly selects Xcode 26.3 (individual keys require Xcode 26 or newer), omitting the Issuer ID and bypassing Tauri's built-in team-key notarizer. Immediately after submission, it saves a `notarization-pending-macos-<arch>` Actions artifact for 14 days: the original signed DMG, a permission-preserving application archive, and a manifest with the submission ID, source commit, and SHA-256 checksums. No private keys are included. Waiting starts only after that checkpoint is saved. After Apple accepts the submission, working copies are stapled and verified, and only the final DMG becomes a Release installer. Exit, failure, and cancellation clean up the temporary keychain, `.p12`, and `.p8`; credentials are excluded from caches and artifacts.

### Resume after a notarization timeout

Apple continues processing after the client's 30-minute wait expires. Do not select Re-run jobs on the original build: the workflow blocks recompilation and resubmission if a signed checkpoint already exists.

In Actions, select **Resume macOS notarization → Run workflow**, select the `master` workflow branch, and supply the original `Build and Release` run ID and release tag (for example, `v0.6.3`). Recovery verifies that the source is a tag-push run in this repository, that the tag still identifies its original commit, and that checkpoint hashes match. It restores the original signed files and only queries/waits for the original submission. It requires no signing certificate or `.p12` and never rebuilds, re-signs, or resubmits. Once both macOS packages pass, it reuses the original Windows artifacts, uploads all four installers, and publishes an existing draft without changing its notes.

If Apple is still processing, dispatch recovery again later with the same original run ID; the checkpoint remains unchanged. Recovery requires a successfully saved checkpoint artifact. If Apple received a submission but saving its checkpoint failed, investigate that original submission instead of automatically resubmitting. Recovery stops if the tag moved or the checkpoint was modified, expired, or missing. Older workflows without saved checkpoints cannot recover their original signed files after the runner is gone; a submission ID alone cannot staple a different build.

GitHub encrypts Secrets at rest, but cannot prevent someone authorized to modify a trusted release workflow from stealing its credentials. Limit repository write access and permission to modify release branches and tags, and review changes to the workflow and its scripts. Third-party Actions in the macOS job are pinned to commit SHAs. Never enable shell tracing or dump environment variables in credential-bearing steps. Developer names, Team IDs, and public certificates in signed artifacts are public information; signing private keys and API private keys must not appear in artifacts.

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
