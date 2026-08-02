# SSH Client

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
- 深色/浅色界面、终端外观、背景图和应用图标设置

## 开发环境

- Node.js 24（推荐使用 [fnm](https://github.com/Schniz/fnm) 管理）
- npm
- Rust stable（通过 [rustup](https://rustup.rs/) 安装）
- macOS：Xcode Command Line Tools
- Windows：Microsoft C++ Build Tools 和 WebView2 构建依赖

安装依赖并启动：

```bash
fnm use 24
rustup default stable
npm ci
npm run dev
```

`npm run dev` 会启动 Vite 前端和 Tauri 桌面应用。只启动或构建 Web 前端可使用 `npm run web:dev`、`npm run web:build`。

## 数据迁移

正式应用名为 `SSH Client`，标识符为 `com.local.sshclient`。

首次创建正式 Tauri 数据库时，应用会优先复制旧 `SSH Client Tauri Preview` 的非敏感数据；如果不存在预览数据，再读取旧 Electron 版数据库。连接、设置、Host Key 和传输历史会迁移，来源数据库不会被修改。

旧版密码不会迁移：Electron `safeStorage` 数据无法由 Rust 安全解密，Preview 凭据也属于不同的系统安全存储命名空间。首次连接时需要重新输入密码，之后可保存到当前应用的 Keychain 或 Credential Manager。

## 检查

```bash
npm run typecheck
npm test
npm run check
npm run web:build
```

其中 `npm test` 运行 Rust 单元测试，`npm run check` 同时检查 TypeScript 和 Rust。

## 打包

Tauri 安装包必须在目标平台原生构建，产物位于 `app/native/target/release/bundle/`。未配置证书时会生成未签名安装包；正式分发前应配置 Apple Developer ID、公证和 Windows 代码签名。

macOS 构建 DMG：

```bash
fnm use 24
npm ci
rustup default stable
npm run typecheck
npm test
npm run build:mac
```

校验 DMG：

```bash
hdiutil verify "app/native/target/release/bundle/dmg/SSH Client_<version>_aarch64.dmg"
```

Windows x64 构建普通 NSIS 安装包：

```powershell
fnm use 24
npm ci
rustup default stable-x86_64-pc-windows-msvc
npm run typecheck
npm test
npm run build:win
```

普通包内置 WebView2 bootstrapper；目标机器缺少 WebView2 Runtime 时，安装过程需要联网。构建包含 WebView2 Runtime 的离线包：

```powershell
npm run build:win:offline
```

离线包体积更大，构建机首次打包仍需联网下载 Microsoft WebView2 离线安装器。发布前应在 Windows 真机验证普通包和离线包的安装、旧版迁移、覆盖安装、卸载及重启恢复。
