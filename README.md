<div align="center">
  <img src="assets/icons/luna.png" width="96" alt="Luna Mux 图标">
  <h1>Luna Mux</h1>
  <p>面向 Coding Agent 的本地与远程终端工作台</p>
  <p>
    <a href="https://github.com/rainj2013/luna-mux/actions/workflows/release.yml"><img src="https://github.com/rainj2013/luna-mux/actions/workflows/release.yml/badge.svg" alt="构建状态"></a>
    <img src="https://img.shields.io/badge/platform-macOS%20%7C%20Windows-5b8def?style=flat-square" alt="支持平台">
    <a href="LICENSE"><img src="https://img.shields.io/github/license/rainj2013/luna-mux?style=flat-square" alt="MIT License"></a>
  </p>
  <p>
    <a href="#快速开始">快速开始</a>
    ·
    <a href="docs/DEVELOPMENT.md">开发文档</a>
    ·
    <a href="docs/LUNA_MUX_DESIGN.md">设计方案</a>
  </p>
</div>

<div align="center"><a href="README.en.md">English</a> · 简体中文</div>

Luna Mux 把项目终端、Coding Agent、远程服务器和受管浏览器放进同一个工作空间。一个 Session 可以保存项目目录、多个本地或 SSH 窗格及其分屏布局；Codex、Claude Code 等 Agent 仍运行在普通终端中，但可以获得状态提醒、浏览器工具，以及在明确权限边界内控制 Luna Mux 的能力。

> [!IMPORTANT]
> Luna Mux 当前处于 **0.1.0 预览阶段**。macOS 已用于连续开发和真实 Agent/SSH/浏览器任务；Windows 核心路径已经实现并经过实机测试，但真实 WSL、部分 Claude Code 场景和跨平台发布回归仍在收尾。暂不建议把它当作无人值守的生产基础设施。

## 核心能力

### 项目 Session 与终端窗格

- 一个 Session 对应一个项目上下文，可保存项目根目录、Pane 定义和递归分屏布局。
- 支持横向/纵向分割、拖动比例、布局预设、重命名、最大化和恢复。
- macOS 使用本地 zsh/bash PTY；Windows 支持 PowerShell 7 和 WSL；两端都可创建 SSH 终端。
- 本地和远程终端共用 xterm.js 界面、搜索、复制粘贴、主题、字体、背景和输出流控。
- 应用重启后恢复 Session、Pane 和布局定义，但不会擅自重连服务器或重新启动进程。

### 普通终端中的 Coding Agent

Luna Mux 没有特殊的“Agent 窗格”。你可以在任意普通终端中手动运行 `codex` 或 `claude`，也可以使用内置启动入口。受支持的 Agent 会通过进程级配置接入，不需要改写用户的全局 Codex 或 Claude Code 配置。

- 统一显示 Agent 的工作、等待输入、等待权限、完成和错误状态。
- 在侧边栏、窗格边框和桌面提示中标记需要关注的 Pane；点击通知可回到对应 Session 和 Pane。
- Agent 环境视图显示 Adapter、Hook、Luna MCP 和 Browser MCP 的实际健康状态。
- Agent 生命周期跟随 Luna Mux；退出应用会关闭受管终端、Agent 和 Chrome。

### Agent 控制 Luna Mux

Luna MCP 让终端中的 Agent 在受限范围内理解和操作 Luna Mux 自身，而不是把应用操作误当成浏览器操作。

Agent 可以在当前 Session 内：

- 发现 Session、Pane、Terminal Runtime 和其他受管 Agent；
- 创建 Pane、修改布局、读取有界终端输出并向终端写入输入；
- 查询 Agent 状态、投递任务或发送中断；
- 读取安全的连接摘要，修改主题和终端外观；
- 查询传输、隧道和控制事件。

同一 Session 是协作与授权边界。Agent 默认不能访问其他 Session；关闭 Runtime、启动传输或隧道等重要副作用仍可要求桌面确认。凭据、私钥内容和 AI Key 不会通过 MCP 暴露。

### Session 级受管浏览器

每个 Session 可以拥有一个受管 Chrome 资源。Chrome 使用隔离的持久配置目录，并以标准外部窗口运行，用户随时可以接管。

- 浏览器不是终端 Pane，不参与分屏布局。
- Agent 通过固定版本的原生 `agent-browser` MCP 操作网页，支持快照、交互、等待、标签页、截图、控制台、网络请求和 HAR。
- 第一次真正使用浏览器工具时按需启动 Chrome，并在后续调用中复用同一个 Runtime 和页面。
- 远程 SSH Agent 通过认证代理使用本机受管 Chrome，不向远端暴露原始 CDP 端口。
- Luna Mux 功能、网页操作和普通源码/Shell/Git 操作使用不同工具域，减少 Agent 错把“窗格”理解成浏览器标签页的情况。

### SSH、SFTP 与端口转发

- SSH 支持密码、私钥、SSH Agent、Host Key 校验、保活和一级跳板机。
- 连接可以分组、排序、备份，并从 OpenSSH Config 或 Luna Remote 数据库显式导入。
- SFTP 支持本地/远程目录浏览、上传、下载、预览、拖放、冲突处理、传输队列和失败重试。
- 支持本地转发、远程转发和 SOCKS5 动态转发。
- 远程 Agent 集成默认关闭；启用后，运行时文件隔离在远端 `~/.luna-mux/runtime/<runtime-id>`，正常断开时会清理。

### AI 命令助手

AI 命令助手与 Codex/Claude Agent 相互独立。它使用用户配置的 OpenAI 兼容服务，为当前本地或 SSH 终端生成 Linux Shell、PowerShell、CMD 或 macOS 命令。

生成结果包含说明、前提、警告和风险等级，可以复制、只填入终端，或经过风险确认后执行。附带终端上下文时，可选的常见个人信息脱敏会在请求前执行；不配置 AI 服务不会影响其他功能。

## 运行模型

```text
Luna Mux
`-- Mux Session（项目与授权边界）
    |-- Pane
    |   `-- Terminal Runtime（本地 PTY 或 SSH）
    |       `-- 可选的 Codex / Claude Code Agent
    `-- Browser Resource
        `-- 受管的外部 Chrome Runtime
```

Session、Pane 和 Browser Resource 会持久化；Terminal Runtime、Agent 进程、Chrome 进程、输出缓冲和临时授权只存在于当前应用生命周期。这个边界让恢复布局保持可预测，也避免应用退出后遗留无人管理的进程。

## 平台状态

| 能力 | macOS | Windows |
| --- | --- | --- |
| 本地终端 | zsh/bash 已验证 | PowerShell 7 已验证；WSL 完整验收中 |
| SSH、SFTP、传输与隧道 | 已实现并持续回归 | 已实现并经过实机测试 |
| Codex Hook、Luna MCP、Browser MCP | 本地及真实 SSH 路径已验证 | PowerShell 路径已验证；WSL 待验收 |
| Claude Code Adapter | 基础启动与注入已验证 | 等价端到端场景待补齐 |
| Agent 通知 | 主题化应用内通知，可点击定位 Pane | 原生系统通知，待继续实机回归 |
| 安装包 | 未签名 DMG | 未签名 NSIS 在线/离线安装包 |

长期进度、验收证据和未完成项见 [开发进度](docs/DEVELOPMENT_PROGRESS.md) 与 [开发任务](docs/DEVELOPMENT_TASKS.md)。

## 快速开始

当前仓库尚未发布稳定安装包，建议开发者从源码运行。需要：

- Node.js 24 与 npm；
- Rust stable 1.85 或更高版本；
- macOS：Xcode Command Line Tools；
- Windows：MSVC Build Tools、Windows SDK、WebView2 Runtime 和 NASM。

```bash
git clone https://github.com/rainj2013/luna-mux.git
cd luna-mux
npm ci
npm run dev
```

`npm run dev` 会同步当前平台的 `agent-browser` sidecar，在 `127.0.0.1:1420` 启动 Vite，并编译、运行 Tauri 桌面应用。首次 Rust 编译需要较长时间。

完整环境安装、国内 Cargo 镜像、平台打包和常见问题见 [开发文档](docs/DEVELOPMENT.md)。

## 检查与构建

```bash
npm run check
npm test
npm run web:build
```

构建当前平台的桌面安装包：

```bash
npm run build:mac
# 或在 Windows 上
npm run build:win
npm run build:win:offline
```

GitHub Actions 可手动构建 macOS Intel、macOS Apple Silicon、Windows 在线和 Windows 离线安装包。推送 `v*` 标签时会创建 GitHub Release。当前构建使用 ad-hoc 或无签名方式，不需要付费开发者身份，但首次运行可能触发 macOS“隐私与安全性”确认或 Windows SmartScreen。

## 数据与安全

- 数据库、设置、浏览器目录和凭据命名空间与 Luna Remote 完全隔离。
- 密码和 API Key 保存在 macOS Keychain 或 Windows Credential Manager，而不是项目数据库。
- Agent 事件只保存生命周期元数据，不保存提示词、工具输入或工具输出。
- Luna MCP 使用仅回环的认证传输，Token 绑定 Runtime，并在 Runtime 退出时撤销。
- 远程 Agent 支持文件不会修改远端 Shell 启动文件或用户级 Agent 配置。
- 浏览器自动化只连接 Luna Mux 启动的隔离 Chrome；远程 Agent 不获得原始 CDP 地址。

## 项目文档

- [开发环境、检查与打包](docs/DEVELOPMENT.md)
- [产品与架构设计](docs/LUNA_MUX_DESIGN.md)
- [终端运行时架构](docs/TERMINAL_ARCHITECTURE.md)
- [当前开发进度](docs/DEVELOPMENT_PROGRESS.md)
- [长期任务清单](docs/DEVELOPMENT_TASKS.md)

产品名称、应用标识和存储命名空间统一维护在 `product/product.json`。修改后运行 `npm run product:sync`，并用 `npm run product:check` 校验生成结果。

## 功能融合与 Luna Remote

[Luna Remote](https://github.com/rainj2013/luna-remote) 是面向 SSH/SFTP 日常使用的独立桌面客户端；Luna Mux 面向项目终端、Coding Agent 协作和受管浏览器工作流。Luna Mux 融合了连接管理、SSH、SFTP、文件传输和端口转发等成熟能力，并在布局、主题、终端外观和常用交互上延续相近的 UI 与使用体验。

两个项目在 Git 层面完全独立，不维护共同历史、上游 Remote 或提交同步关系。后续需要参考 Luna Remote 的功能时，由 AI 阅读当前代码和行为，再按照 Luna Mux 的 Session、Pane、Runtime 与权限模型重新实现。Luna Mux 不会自动读取或修改 Luna Remote 数据；只有用户主动使用导入向导时，才会读取选定的数据快照。

## 许可证

Luna Mux 基于 [MIT License](LICENSE) 发布。第三方组件、字体和内置工具保留各自的许可证与分发条件。
