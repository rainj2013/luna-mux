# Luna Mux 开发文档

Luna Mux 基于 Tauri 2、React、xterm.js 和 Rust 开发，首版同步支持 macOS 和 Windows。安装包需要在对应目标平台原生构建。

本文中的命令均在仓库根目录执行，也就是包含 `package.json` 的目录。

## 项目结构

```text
app/
  frontend/   React、xterm.js 和 Tauri API 桥接层
  native/     Rust 后端、Tauri 配置和平台能力
assets/icons/ 应用及界面图标
docs/         架构、开发和设计资料
```

终端的数据流、PTY、输出背压和安全边界见 [设计方案](LUNA_MUX_DESIGN.md) 的「终端后端」一节。

## 开发环境

- Node.js 24 和 npm，推荐使用 [fnm](https://github.com/Schniz/fnm) 管理。仓库根目录的 `.node-version` 已固定主版本。
- Rust stable 1.85 或更高版本。项目使用 Rust 2024 edition，推荐通过 [rustup](https://rustup.rs/) 安装。
- macOS 需要 Xcode Command Line Tools。
- Windows 需要 Microsoft C++ Build Tools、Windows SDK、WebView2 Runtime 和 NASM（`aws-lc-sys` 的 native crypto 构建依赖）。

### macOS

安装 Xcode Command Line Tools。

```bash
xcode-select --install
```

如果尚未安装 `fnm`，可以通过 Homebrew 安装。

```bash
brew install fnm
```

将下面一行加入 `~/.zshrc`，然后重新打开终端。也可以直接执行它，为当前终端初始化环境。

```bash
eval "$(fnm env --use-on-cd --shell zsh)"
```

安装 Rust。

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

只开发 macOS 桌面应用不需要安装完整 Xcode。`tauri info` 可能提示缺少 Xcode，该提示主要影响 iOS 开发。

### Windows

在 PowerShell 中安装 `fnm`、Rust、Visual Studio 2022 Build Tools 和 WebView2 Runtime。

```powershell
winget install --exact --id Schniz.fnm
winget install --exact --id Rustlang.Rustup
winget install --exact --id Microsoft.VisualStudio.2022.BuildTools --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
winget install --exact --id Microsoft.EdgeWebView2Runtime
winget install --exact --id NASM.NASM
```

安装完成后重新打开 PowerShell。将下面一行加入 PowerShell 的 `$PROFILE`，并在当前终端执行一次。

```powershell
fnm env --use-on-cd --shell powershell | Out-String | Invoke-Expression
```

确保 Rust 使用默认的 MSVC 工具链。Visual Studio 安装器中应包含“使用 C++ 的桌面开发”、MSVC v143 和 Windows 10/11 SDK。

## 安装项目依赖

获取或解压源码后，进入仓库根目录并执行。

```console
fnm install 24
fnm use
rustup default stable
npm ci
```

确认工具链可用。

```bash
node --version
npm --version
rustc --version
cargo --version
```

`node --version` 应为 `v24.x`，`rustc` 应为 1.85 或更高版本。首次安装依赖需要访问 npm registry 和 crates.io。

## 启动开发环境

```bash
npm run dev
```

该命令会在 `127.0.0.1:1420` 启动 Vite，并编译、启动 Tauri 桌面应用。首次 Rust 编译耗时会明显长于后续增量编译。出现 Luna Mux 窗口即表示环境搭建完成。

只启动或构建 Web 前端可以使用下面的命令。

```bash
npm run web:dev
npm run web:build
```

Web 页面离开 Tauri Runtime 后无法完整使用本地数据库、SSH、SFTP、系统凭据和文件对话框等桌面能力。

开发启动不需要环境变量或 API Key。AI 命令助手为可选功能，可以在应用的“设置 → AI”中配置 OpenAI 兼容服务。

> Luna Mux 使用独立应用标识 `com.luna.mux`、数据库和系统凭据命名空间，不会自动读取或修改 Luna Remote 数据。

## 多语言

界面文案集中在 `app/frontend/src/locales/`，组件只引用稳定的词条 key，不直接保存中英文文案。

- `zh-CN.messages.ts` 是完整的基准词条表，并定义可用 key 的 TypeScript 类型。
- `en.messages.ts` 是英文词条表。变量使用 `{{name}}` 形式的占位符。
- `*.locale.ts` 是自动发现的语言注册文件，包含语言代码、显示名称、排序、词条表和该语言的帮助内容。
- `help-content.tsx` 存放需要列表、表格和图标的富文本帮助内容；普通界面文案应放在词条表中。

新增语言时，复制 `en.messages.ts` 和 `en.locale.ts`，分别改为新的语言代码，再翻译词条和帮助内容即可。设置页会自动显示新的语言，原生菜单也会从同一份词条表生成，无需修改 React 组件或 Rust 语言枚举。

```bash
npm run i18n:check
```

该检查会验证所有语言的 key、重复项和插值占位符完全一致，并阻止旧式内联 `tr()` 调用重新进入组件。

## 检查

```bash
npm run check
npm test
npm run web:build
```

`npm run check` 会依次校验产品元数据、终端 Runtime 契约、图标、多语言、TypeScript 类型并执行 Rust `cargo check`；`npm test` 运行 Rust 单元测试，`npm run web:build` 验证前端生产构建。提交代码前应保证三条命令全部通过。

## 打包

Tauri 安装包必须在目标平台原生构建，产物位于 `app/native/target/release/bundle/`。未配置证书时会生成未签名安装包。正式分发前应配置 Apple Developer ID、公证和 Windows 代码签名。

GitHub Actions 和默认开发环境使用 crates.io。首次构建或 `package-lock.json` 变化后执行一次下面的命令。

```bash
fnm use
npm ci
rustup default stable
```

日常重新打包不需要重复安装依赖，也不需要临时修改 Cargo 配置。

### macOS

构建 DMG。

```bash
npm run build:mac
```

校验 DMG。

```bash
hdiutil verify "app/native/target/release/bundle/dmg/Luna Mux_<version>_<arch>.dmg"
```

`<arch>` 在 Apple Silicon 构建中通常为 `aarch64`，在 Intel 构建中通常为 `x64`。请以 `bundle/dmg/` 中的实际文件名为准。

### Windows

构建 Windows x64 标准 NSIS 安装包（推荐）。

```powershell
npm run build:win
```

标准版不内置 WebView2 Runtime。Windows 10/11 通常已经具备 WebView2；目标机器缺少时，安装程序会联网下载 bootstrapper。

构建内置 WebView2 Runtime 的兼容版。

```powershell
npm run build:win:webview2
```

兼容版体积更大，只适合系统缺少 WebView2 且安装时无法联网的场景。构建机首次打包仍需联网下载 Microsoft WebView2 离线安装器。`npm run build:win:offline` 作为旧命令的兼容别名继续保留。发布前应在 Windows 真机验证标准版和兼容版的安装、覆盖安装、卸载及重启恢复。

## GitHub Actions

仓库中的 `Build and Release` 工作流会构建四个安装包。

- macOS Intel x64 DMG
- macOS Apple Silicon ARM64 DMG
- Windows x64 标准安装包（推荐，文件名不带额外版本后缀）
- Windows x64 内置 WebView2 兼容安装包（`with-webview2`）

在 GitHub 仓库的 Actions 页面中手动运行工作流时，构建结果会作为临时产物保存 14 天。推送以 `v` 开头的版本标签时，工作流还会创建同名 GitHub 发布版本，并上传全部安装包。

```bash
git tag v1.0.0
git push origin v1.0.0
```

macOS 安装包使用 ad-hoc 签名，不需要 Apple 开发者证书。它没有经过 Apple 公证，用户首次运行时仍需在“隐私与安全性”设置中确认。Windows 安装包没有代码签名，系统可能显示未知发布者或 SmartScreen 提示。

## 常见问题

### 配置 Cargo 国内镜像

macOS 和 Linux 使用下面的命令。

```bash
cp .cargo/config.rsproxy.toml.example .cargo/config.toml
cargo fetch --manifest-path app/native/Cargo.toml
```

Windows PowerShell 使用下面的命令。

```powershell
Copy-Item .cargo/config.rsproxy.toml.example .cargo/config.toml
cargo fetch --manifest-path app/native/Cargo.toml
```

恢复 crates.io。

```bash
rm .cargo/config.toml
```

Windows PowerShell：

```powershell
Remove-Item .cargo/config.toml
```

### 端口 1420 被占用

Vite 使用固定端口 `1420`。关闭占用该端口的进程后重新运行 `npm run dev`。

```bash
lsof -nP -iTCP:1420 -sTCP:LISTEN
```

Windows PowerShell 可以使用下面的命令。

```powershell
Get-NetTCPConnection -LocalPort 1420 -ErrorAction SilentlyContinue
```

### Windows 无法编译或启动

重新打开 Visual Studio Installer，确认已安装“使用 C++ 的桌面开发”、MSVC v143 和 Windows SDK，再确认 WebView2 Runtime 和 NASM（`nasm -v` 可执行）已安装。修改工具链后应重新打开 PowerShell。

### macOS 出现签名或 Keychain 提示

本地 `npm run dev` 和未签名构建不需要分发证书。首次保存密码或 API Key 时，macOS 可能要求授权访问 Keychain，这是系统凭据存储的正常行为。
