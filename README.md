<div align="center">
  <h1>Luna Mux</h1>
  <p>面向 Coding Agent 的本地与远程终端工作台</p>
  <p><strong>当前状态：设计与产品基础阶段，尚未发布可用版本。</strong></p>
</div>

<div align="center">简体中文</div>

Luna Mux 是从 Luna Remote 独立出来的新产品和代码仓库。它复用成熟的 SSH、SFTP、端口转发和终端能力，并在此基础上建设跨平台本地终端、自由分屏、Coding Agent 状态管理、跨 Agent 控制和浏览器验证流程。

Luna Mux 不是传统 IDE，不计划内置代码编辑器、语言服务或 Git GUI。

## 首版目标

- Windows：PowerShell、WSL 和远程 SSH
- macOS：本地登录 Shell 和远程 SSH
- 本地与远端共用同一套 xterm.js 终端界面
- 可持久化的工作区和递归横向/纵向分屏
- Codex 状态、通知和受控跨 Agent 操作
- 本地 Chrome 浏览器资源，远端开发服务通过 SSH 转发访问
- 保留 SFTP、文件传输、端口转发和系统凭据存储

首版 Agent 生命周期与 Luna Mux 应用一致。应用退出后 Agent 不继续运行，但 `SessionBackend` 会保留未来接入后台守护进程的扩展边界。

## 开发资料

- [完整设计方案](docs/LUNA_MUX_DESIGN.md)
- [开发分步任务](docs/DEVELOPMENT_TASKS.md)
- [当前开发进度](docs/DEVELOPMENT_PROGRESS.md)
- [Luna Remote 上游同步规则](docs/UPSTREAM_SYNC.md)
- [开发环境](docs/DEVELOPMENT.md)

产品名称和技术标识统一维护在 `product/product.json`。修改后运行：

```bash
npm run product:sync
npm run product:check
```

## 仓库关系

Luna Mux 与 Luna Remote 是两个独立软件。当前仓库保留 Luna Remote 的 Git 历史以便追踪来源，但拥有独立的应用标识、数据库、凭据命名空间和发布流程。Luna Remote 后续更新通过提交级审阅和选择性移植同步。

## 许可证

本项目沿用 [MIT License](LICENSE)，第三方组件和字体保留各自许可证。
