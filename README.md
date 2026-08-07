# Luna Remote

基于 Tauri 2、React、xterm.js 和 Rust 的跨平台 SSH/SFTP 桌面客户端，支持 macOS 和 Windows。

## 项目结构

```text
app/
  frontend/   React、xterm.js 和 Tauri API bridge
  native/     Rust 后端、Tauri 配置和平台能力
assets/icons/ 应用及界面图标
docs/         架构和设计资料
```

## 功能

- 同一机器同时建立多个 SSH session
- 密码、键盘交互、私钥和 SSH Agent 认证
- macOS Keychain 与 Windows Credential Manager 安全凭据存储
- Host Key 校验、session 保活和一级跳板机
- SFTP 浏览、预览、上传、下载、目录传输和冲突处理
- 本地、远程和 SOCKS5 动态端口转发
- 部署差异预览与批量上传
- 可配置 OpenAI 兼容服务的 AI 命令助手、风险提示和终端上下文脱敏
- 深色/浅色界面、终端外观、背景图和应用图标设置

## 开发环境

- Node.js 24 和 npm（推荐使用 [fnm](https://github.com/Schniz/fnm) 管理；仓库根目录的 `.node-version` 已固定主版本）
- Rust stable 1.85 或更高版本（项目使用 Rust 2024 edition，推荐通过 [rustup](https://rustup.rs/) 安装）
- macOS：Xcode Command Line Tools
- Windows：Microsoft C++ Build Tools、Windows SDK 和 WebView2 Runtime

项目仅支持 macOS 和 Windows，安装包需要在对应目标平台原生构建。以下命令均在仓库根目录（包含 `package.json` 的目录）执行。

### macOS

安装 Xcode Command Line Tools：

```bash
xcode-select --install
```

如果尚未安装 `fnm`，可以通过 Homebrew 安装：

```bash
brew install fnm
```

将下面一行加入 `~/.zshrc`，然后重新打开终端；也可以直接执行它为当前终端初始化环境：

```bash
eval "$(fnm env --use-on-cd --shell zsh)"
```

安装 Rust：

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

只开发 macOS 桌面应用不需要安装完整 Xcode。`tauri info` 可能提示缺少 Xcode，该提示主要影响 iOS 开发。

### Windows

在 PowerShell 中安装 `fnm`、Rust、Visual Studio 2022 Build Tools 和 WebView2 Runtime：

```powershell
winget install --exact --id Schniz.fnm
winget install --exact --id Rustlang.Rustup
winget install --exact --id Microsoft.VisualStudio.2022.BuildTools --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
winget install --exact --id Microsoft.EdgeWebView2Runtime
```

安装完成后重新打开 PowerShell。将下面一行加入 PowerShell 的 `$PROFILE`，并在当前终端执行一次：

```powershell
fnm env --use-on-cd --shell powershell | Out-String | Invoke-Expression
```

确保 Rust 使用默认的 MSVC 工具链。Visual Studio 安装器中应包含“使用 C++ 的桌面开发”、MSVC v143 和 Windows 10/11 SDK。

### 安装项目依赖

获取或解压源码后，进入仓库根目录并执行：

```console
fnm install 24
fnm use
rustup default stable
npm ci
```

确认工具链可用：

```bash
node --version
npm --version
rustc --version
cargo --version
```

`node --version` 应为 `v24.x`，`rustc` 应为 `1.85` 或更高版本。首次安装依赖需要访问 npm registry 和仓库配置的 Cargo 镜像。

### 启动开发环境

```bash
npm run dev
```

该命令会在 `127.0.0.1:1420` 启动 Vite，并编译、启动 Tauri 桌面应用。首次 Rust 编译耗时会明显长于后续增量编译。出现 Luna Remote 窗口即表示环境搭建完成。

只启动或构建 Web 前端可使用：

```bash
npm run web:dev
npm run web:build
```

Web 页面离开 Tauri Runtime 后无法完整使用本地数据库、SSH、SFTP、系统凭据和文件对话框等桌面能力。

开发启动不需要环境变量或 API Key。AI 命令助手为可选功能，可在应用的“设置 -> AI”中配置 OpenAI 兼容服务。

> 开发版和安装版当前使用相同的应用标识 `com.local.sshclient`，会访问同一份应用数据和系统凭据。调试已有安装环境前，请先在应用中导出连接备份。

## 检查

```bash
npm run check
npm test
npm run web:build
```

其中 `npm run check` 依次执行 TypeScript 类型检查和 Rust `cargo check`，`npm test` 运行 Rust 单元测试。提交代码前应保证三条命令全部通过。

## 打包

Tauri 安装包必须在目标平台原生构建，产物位于 `app/native/target/release/bundle/`。未配置证书时会生成未签名安装包；正式分发前应配置 Apple Developer ID、公证和 Windows 代码签名。

仓库已固定使用 rsproxy 的 Cargo sparse registry，避免企业网络下 crates.io 下载缓慢以及切换源造成缓存不一致。首次构建或 `package-lock.json` 变化后执行一次：

```bash
fnm use
npm ci
rustup default stable
```

日常重新打包不需要重复安装依赖，也不需要临时修改 Cargo 配置。

macOS 构建 DMG：

```bash
npm run build:mac
```

校验 DMG：

```bash
hdiutil verify "app/native/target/release/bundle/dmg/Luna Remote_<version>_<arch>.dmg"
```

`<arch>` 在 Apple Silicon 构建中通常为 `aarch64`，在 Intel 构建中通常为 `x64`；以 `bundle/dmg/` 中的实际文件名为准。

Windows x64 构建普通 NSIS 安装包：

```powershell
npm run build:win
```

普通包内置 WebView2 bootstrapper；目标机器缺少 WebView2 Runtime 时，安装过程需要联网。构建包含 WebView2 Runtime 的离线包：

```powershell
npm run build:win:offline
```

离线包体积更大，构建机首次打包仍需联网下载 Microsoft WebView2 离线安装器。发布前应在 Windows 真机验证普通包和离线包的安装、覆盖安装、卸载及重启恢复。

## 常见问题

### Cargo 依赖下载失败

仓库的 `.cargo/config.toml` 将 crates.io 替换为 rsproxy sparse registry。先确认可以访问 `https://rsproxy.cn`，然后重试：

```bash
cargo fetch --manifest-path app/native/Cargo.toml
```

如果所在网络无法使用 rsproxy，可在本地删除 `.cargo/config.toml` 中的 `[source.crates-io]` 和 `[source.rsproxy-sparse]` 配置，恢复使用 crates.io；不要将这一临时修改提交到仓库。

### 端口 1420 被占用

Vite 使用固定端口 `1420`。关闭占用该端口的进程后重新运行 `npm run dev`：

```bash
lsof -nP -iTCP:1420 -sTCP:LISTEN
```

Windows PowerShell：

```powershell
Get-NetTCPConnection -LocalPort 1420 -ErrorAction SilentlyContinue
```

### Windows 无法编译或启动

重新打开 Visual Studio Installer，确认已安装“使用 C++ 的桌面开发”、MSVC v143、Windows SDK；再确认 WebView2 Runtime 已安装。修改工具链后应重新打开 PowerShell。

### macOS 出现签名或 Keychain 提示

本地 `npm run dev` 和未签名构建不需要分发证书。首次保存密码或 API Key 时，macOS 可能要求授权访问 Keychain，这是系统凭据存储的正常行为。
