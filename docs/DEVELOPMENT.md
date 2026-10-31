# Luna Mux 开发文档

本文说明 Luna Mux 的开发环境、检查命令、打包和发布流程。项目基于 Tauri 2、React、xterm.js 和 Rust，支持 macOS 和 Windows；安装包在对应目标平台原生构建。

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

- Node.js 24 和 npm，使用 [fnm](https://github.com/Schniz/fnm) 管理，版本以仓库根目录的 `.node-version` 为准。
- 当前 Rust stable 工具链，通过 [rustup](https://rustup.rs/) 安装和更新。项目使用 Rust 2024 edition，编译器版本须满足锁定依赖的要求。
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

`node --version` 应与 `.node-version` 匹配。如果 Rust 版本不满足依赖要求，运行 `rustup update stable`。首次安装依赖需要访问 npm registry 和 crates.io。

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

验证范围以 [AI 协作开发规范](AI_DEVELOPMENT_GUIDELINES.md) 中的变更分级为准。以下是常用的完整检查入口；文档、样式和局部行为修改按对应级别执行相关检查即可。

```bash
npm run check
npm test
npm run web:build
```

`npm run check` 校验产品元数据、终端 Runtime 契约、图标和多语言，运行终端相关脚本测试、TypeScript 类型检查及 Rust `cargo check`；`npm test` 运行 Rust 测试，`npm run web:build` 验证前端生产构建。

## 打包

Tauri 安装包必须在目标平台原生构建，产物位于 `app/native/target/release/bundle/`。未配置证书时会生成未签名安装包。正式分发前应配置 Apple Developer ID、公证和 Windows 代码签名。

首次构建前先完成[安装项目依赖](#安装项目依赖)。`package-lock.json` 变化后重新运行 `npm ci`；日常重新打包不需要重复安装依赖。GitHub Actions 默认使用 crates.io，本地镜像配置见常见问题。

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

如果需要连续构建两个 Windows 变体，可以只编译一次原生程序，再分别生成安装器：

```powershell
npm run build:win:prepare
npm run bundle:win
npm run bundle:win:webview2
```

`build:win:prepare` 只执行前端和 Rust 编译，后两个命令只执行 NSIS 打包，适合本地反复验证安装器配置。

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

### Release Note 规范

Release Note 面向用户，说明升级带来的功能、修复和兼容性变化。推荐包含版本标题、简短摘要、“新增与改进”、“安装包说明”及上一版本到当前版本的 GitHub Compare 链接。

- 按用户任务合并相关变化，完整覆盖有意义的功能、修复和兼容性影响，不逐条复述提交或内部实现，也不限定条数。
- 使用界面名称和克制、准确的语言；技术细节仅在影响安装、数据、安全或用户操作时说明。
- 明确平台限制、迁移步骤和已知限制，不夸大未经验证的效果。
- 发布前依据版本 diff 和测试结果核对说明、版本号及 Compare 链接。

标签工作流在 Release 尚不存在时会自动生成说明，但正式版本应使用人工审阅的 Release Note：可以在推送标签后立即创建同名草稿 Release，或在工作流完成后编辑自动生成的正文。工作流发现已有 Release 时保留正文，四个安装包验证并上传完成后自动发布草稿。

macOS 发布包使用 Developer ID Application 证书签名，并通过 Apple 公证；工作流验证签名、公证票据和 Gatekeeper 后才上传 DMG。凭据缺失或验证失败会终止 macOS 发布，不会回退到 ad-hoc 签名。Windows 安装包没有代码签名，系统可能显示未知发布者或 SmartScreen 提示。

### macOS 签名凭据

在仓库 **Settings → Environments** 创建 `macos-release` 环境，将部署来源限制为 `master` 分支和 `v*` 标签（分别添加 Branch 和 Tag 规则）。手动构建请选择 `master`。在该环境添加以下 Secrets，勿放进仓库文件、普通 Variables、Issue 或日志：

| Secret | 内容 |
| --- | --- |
| `APPLE_CERTIFICATE` | 包含 Developer ID Application 证书及其私钥的加密 `.p12` 文件，以 Base64 编码 |
| `APPLE_CERTIFICATE_PASSWORD` | 上述 `.p12` 的导出密码 |
| `APPLE_SIGNING_IDENTITY` | 完整证书名称，例如 `Developer ID Application: Your Name (TEAMID)` |
| `APPLE_API_KEY` | App Store Connect **个人密钥**的 Key ID |
| `APPLE_API_PRIVATE_KEY` | 该个人密钥 `.p8` 文件的完整文本 |

从钥匙串导出证书和私钥为带密码的 `.p12`，在仓库之外生成 Base64 文件，并通过标准输入设置 Secrets，避免把内容放进命令参数或终端输出。例如，假设文件都位于 `~/keys`：

```bash
openssl base64 -A -in ~/keys/DeveloperIDApplication.p12 -out ~/keys/certificate-base64.txt
gh secret set APPLE_CERTIFICATE --env macos-release < ~/keys/certificate-base64.txt
gh secret set APPLE_CERTIFICATE_PASSWORD --env macos-release
gh secret set APPLE_SIGNING_IDENTITY --env macos-release
gh secret set APPLE_API_KEY --env macos-release
gh secret set APPLE_API_PRIVATE_KEY --env macos-release < ~/keys/ApiKey_EXAMPLE.p8
gh secret list --env macos-release
```

无重定向的 `gh secret set` 命令会交互式读取值。`gh secret list` 只显示名称和更新时间，不显示内容。Base64 只是编码，不是加密；本地文件也应仅允许当前用户读取。

工作流先安装依赖、完成编译，再由 `scripts/release-macos.sh` 在 `$RUNNER_TEMP` 中临时恢复凭据，导入独立钥匙串并打包。当前流程针对个人 API 密钥，显式选择 Xcode 26.3（个人密钥需要 Xcode 26 或更新版本），不传 Issuer ID；Tauri 内置的团队密钥公证路径不参与。提交后立即保存 `notarization-pending-macos-<arch>` Actions 产物，包含原始签名 DMG、保留权限的应用归档和带原提交编号、源码版本、文件 SHA-256 的清单，保留 14 天，不含任何私钥。保存成功后才开始等待公证。Apple 接受后，在工作副本中装订票据并验证，只将最终 DMG 用作 Release 安装包。退出、失败或取消时清理临时钥匙串、`.p12` 和 `.p8`；凭据不进入缓存或上传产物。

### 公证超时后恢复

Apple 在客户端 30 分钟等待超时后仍会继续处理。不要点击原构建的 Re-run jobs：如果已有签名检查点，工作流会阻止重新编译和提交。

在 Actions 中选择 **Resume macOS notarization → Run workflow**，工作流分支选 `master`，填入原始 `Build and Release` 的运行 ID 与发布标签（例如 `v0.6.3`）。恢复流程会校验原任务来自本仓库的标签推送、标签仍指向原提交，并校验检查点文件哈希；随后下载原包，只查询和等待原公证编号。无需签名证书或 `.p12`，不会重新编译、重签或再次提交。两种 macOS 包均通过后复用原 Windows 产物，上传四个安装包并发布已有草稿，保留发布说明。

如果 Apple 仍未处理完成，可以稍后再次运行恢复工作流，继续使用同一原始运行 ID；原检查点不会被修改。恢复能力以检查点产物已保存成功为前提；若 Apple 已接收提交但检查点保存失败，应先核对原提交，不要自动重新投递。标签变动、文件被篡改、检查点过期或缺失时会停止。未保存检查点的旧版工作流无法事后恢复原始签名文件，不能仅靠提交编号为另一次构建的文件装订票据。

Secrets 会由 GitHub 加密存储，但不能防御有权修改受信任发布工作流的人恶意读取凭据。应限制仓库写权限和发布分支、标签的修改权限，审查工作流及其调用脚本的变更。macOS job 中的第三方 Actions 固定到提交 SHA。不要在含凭据的步骤开启 shell tracing 或打印环境变量。签名产物中的开发者名称、Team ID 和公钥证书是公开信息，签名私钥和 API 私钥不应出现在产物中。

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
