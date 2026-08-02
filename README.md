# SSH Client

基于 Electron、React、ssh2 和 xterm.js 的跨平台 SSH/SFTP 桌面客户端。

## 开发环境

- Node.js 24（推荐使用 [fnm](https://github.com/Schniz/fnm) 管理）
- npm
- macOS 打包需要 Xcode Command Line Tools

首次安装依赖：

```bash
fnm use 24
npm ci
```

启动开发版本：

```bash
npm run dev
```

## 提交前检查

```bash
npm run typecheck
npm test
npm run build
```

## 打包流程

所有安装包输出到 `release/`。建议在干净的工作区和完整安装依赖后执行：

```bash
fnm use 24
npm ci
npm run typecheck
npm test
npm run dist:mac
npm run dist:win
```

只打 macOS 包：

```bash
npm run dist:mac
```

该命令会生成 Apple Silicon（arm64）和 Intel（x64）的 DMG、ZIP：

| 架构 | DMG | ZIP |
| --- | --- | --- |
| Apple Silicon | `SSH Client-<version>-arm64.dmg` | `SSH Client-<version>-arm64-mac.zip` |
| Intel | `SSH Client-<version>.dmg` | `SSH Client-<version>-mac.zip` |

只打 Windows x64 安装包：

```bash
npm run dist:win
```

产物为 `SSH Client Setup <version>.exe`。当前配置允许用户选择安装目录。

该脚本支持在 macOS 上交叉打包 Windows 版本，并跳过无法跨平台编译的 `cpu-features` 可选原生加速模块；`ssh2` 会自动使用其 JavaScript 实现，不影响 SSH 功能。正式发布前仍建议在 Windows 真机上完成安装、连接和卸载测试。

### 签名说明

未配置证书时，electron-builder 会生成未签名安装包。本机测试可以直接使用，但分发给其他用户时，macOS Gatekeeper 或 Windows SmartScreen 可能给出安全提示。正式分发前应配置 Apple Developer ID、公证流程和 Windows 代码签名证书。

### 校验 macOS 安装包

```bash
hdiutil verify "release/SSH Client-<version>-arm64.dmg"
hdiutil verify "release/SSH Client-<version>.dmg"
```
