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
    <a href="docs/DEVELOPMENT.md">开发文档</a>
    ·
    <a href="docs/LUNA_MUX_DESIGN.md">设计方案</a>
  </p>
</div>

<div align="center"><a href="README.en.md">English</a> · 简体中文</div>

Luna Mux 是一个面向 Coding Agent 的终端工作台。它以项目目录为单位维护 Session，一个 Session 里可以有多个终端窗格，每个窗格既可以是本地终端，也可以是 SSH 远程终端。在这些终端里启动 Codex、Claude Code 等 Agent 时，Luna Mux 会自动注入 Hook 和 MCP 来扩展 Agent 的能力：Hook 负责状态监控，Luna Mux MCP 让 Agent 控制 Luna Mux 自身的各个功能，agent-browser MCP 让 Agent 操作浏览器。此外还内置了移植自 Luna Remote 的完整 SSH 与 SFTP 能力。

## 项目 Session 与终端窗格

一个 Session 对应一个项目目录，保存项目根目录和窗格布局。

- 一个 Session 里有多个终端窗格，支持横向/纵向分割、拖动比例、布局预设、重命名、最大化和恢复。
- 每个窗格既可以是本地终端（macOS 的 zsh/bash、Windows 的 PowerShell 或 WSL），也可以是 SSH 远程终端。
- 本地和远程终端共用 xterm.js 界面，包括搜索、复制粘贴、主题、字体、背景和输出流控。
- 应用重启后恢复 Session 与布局，但不擅自重连服务器或重启进程。

## 终端里的 Agent

在任意终端窗格里启动 `codex` 或 `claude`，Luna Mux 会自动发现这个 Agent，并注入 Hook 和 MCP 来扩展它的能力。你也可以在新建窗格时选择已保存的启动配置，让 Agent 在 Shell 就绪后自动启动。

### 状态监控（Hook）

- Hook 把 Agent 的工作、等待输入、等待权限、完成和错误状态反馈给 Luna Mux。
- 需要关注的窗格会在侧边栏、窗格边框和桌面通知中标记，点击通知即可回到对应位置。
- Agent 环境视图显示 Adapter、Hook、Luna MCP 和 Browser MCP 的健康状态。
- Agent 生命周期跟随应用，退出时关闭受管终端、Agent 和 Chrome。

### 控制 Luna Mux（Luna Mux MCP）

- Luna Mux MCP 向 Agent 开放 Session、窗格、终端、Agent、连接、设置、诊断、传输和隧道等控制能力。
- Agent 可以发现 Session、窗格、Terminal Runtime 和其他受管 Agent。
- Agent 可以创建窗格、修改布局、读取有界终端输出、写入终端输入。
- Agent 可以查询 Agent 状态、投递任务、发送中断。
- Agent 可以读取安全的连接摘要、修改主题和终端外观、运行内置诊断。
- 关闭 Runtime、启动传输或隧道等重要副作用可要求桌面确认；凭据、私钥和 API Key 不通过 MCP 暴露。

### 浏览器自动化（agent-browser MCP）

- 每个 Session 可以拥有一个隔离的 Chrome，Agent 通过 [`agent-browser`](https://github.com/vercel-labs/agent-browser) MCP 自动化网页操作。
- 支持快照、交互、等待、标签页、截图、控制台、网络请求和 HAR。
- 浏览器以独立的满屏窗口运行，用户随时可以接管。
- 首次使用时按需启动 Chrome，后续复用同一 Runtime 和页面。
- 远程 SSH Agent 通过认证代理使用本机 Chrome，不向远端暴露原始 CDP 端口。

## SSH 与 SFTP

完整 SSH 与 SFTP 能力移植自 Luna Remote。

- SSH 支持密码、私钥、SSH Agent、Host Key 校验、保活和一级跳板机。
- 连接可分组、排序、备份，可从 OpenSSH Config 或 Luna Remote 数据库导入。
- SFTP 支持本地/远程目录浏览、上传、下载、预览、拖放、队列和失败重试。
- 支持本地转发、远程转发和 SOCKS5 动态转发。

## AI 命令助手

AI 命令助手与 Codex/Claude Agent 相互独立，使用用户配置的 OpenAI 兼容服务，为当前本地或 SSH 终端生成 Linux Shell、PowerShell、CMD 或 macOS 命令。

- 结果包含说明、前提、警告和风险等级，可复制、只填入终端，或经风险确认后执行。
- 附带终端上下文时，可选的常见个人信息脱敏在请求前执行。
- 不配置 AI 服务不影响其他功能。

## 开发

环境搭建、检查命令和打包发布说明见 [开发文档](docs/DEVELOPMENT.md)。

## 许可证

Luna Mux 基于 [MIT License](LICENSE) 发布。第三方组件、字体和内置工具保留各自的许可证与分发条件。
