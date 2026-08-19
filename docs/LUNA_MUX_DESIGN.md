# Luna Mux 设计方案

状态：已批准的设计基线  
最后更新：2026-08-20

## 1. 产品定义

Luna Mux 是面向 Coding Agent 的本地与远程终端工作台，形态接近终端复用器，而不是 IDE。它不提供代码编辑器、语言服务、Git 图形界面或其他传统 IDE 功能。

首版同步支持 Windows 和 macOS，范围包括：

- Windows 的 PowerShell 7、WSL，以及 macOS 的 zsh/bash；
- 通过 SSH 连接的远程 Linux 终端；
- 本地与远程统一的终端界面；
- 以项目为中心、可持久化递归分屏的 Mux Session；
- Coding Agent 状态、通知、应用控制和受控的跨 Pane/跨 Agent 协作；
- Session 级受管 Chrome 资源及原生 `agent-browser` 自动化；
- 融合连接管理、SSH、SFTP、文件传输、端口转发、凭据和终端能力，并与 Luna Remote 保持相近的 UI 与使用体验。

首版不会在 Luna Mux 退出后继续运行 Agent。终端后端保留未来接入守护进程的边界，但当前不实现后台 Session。

## 2. 仓库和产品边界

Luna Mux 与 Luna Remote 是独立应用和独立仓库。两者不通过 Git 共同历史、上游 Remote、分支合并或提交同步来维护关系。

- Luna Remote 是功能行为和交互体验的参考来源之一。
- Luna Mux 融合连接管理、SSH、SFTP、隧道、传输、凭据处理和终端等能力，并保持相近的 UI 与使用习惯。
- 需要参考 Luna Remote 的功能时，由 AI 阅读其当前代码和行为，再按照 Luna Mux 的领域模型与安全边界重新实现，不直接搬运提交。
- 应用标识、数据库、凭据、设置、浏览器配置目录、构建和发布完全隔离。
- Luna Remote 数据不会自动迁移；只有用户明确确认后，导入向导才读取稳定快照。

功能融合以实际需求为单位。导航、状态归属、数据关系和交互流程以 Luna Mux 的 Session、Pane、Runtime、Agent 与 Browser Resource 模型为准；相同能力可以保留熟悉的界面和操作方式。

通用代码逐步下沉到仓库内核心模块。产品功能可以依赖核心模块，核心模块不得反向依赖 Mux Session、Agent、Browser 或品牌代码。领域模型冲突时，以 Luna Mux 模型为准。

## 3. 产品元数据

`product/product.json` 是产品身份的唯一可编辑来源。修改后执行：

```bash
npm run product:sync
npm run product:check
```

元数据负责显示名称、产品键、可执行文件和包名、Bundle ID、数据库名、凭据服务、URL Scheme、描述和图标。运行时代码读取生成的 `ProductInfo` 或 Rust 常量，不重复写品牌字面量。

未来改名时，应先把旧身份加入 `legacyIdentities`，再通过显式、幂等、可校验的迁移复制旧数据。

## 4. 总体架构

```text
Luna Mux 桌面应用
|-- Mux Session / 分屏管理器
|-- TerminalRuntimeService
|   |-- InProcessLocalPtyTerminalBackend
|   |-- InProcessSshTerminalBackend
|   `-- 未来的 DaemonTerminalBackend
|-- AgentAdapter 注册表
|   |-- CodexAdapter
|   `-- ClaudeCodeAdapter
|-- Agent 面板与共享 Hook 接收器
|-- LunaControlService
|   |-- 受信任桌面适配器
|   |-- 本地 Luna MCP 适配器
|   `-- 未来的 CLI / 本地 IPC 适配器
|-- Session Browser Resource 管理器 / 本地 Chrome CDP 控制器
`-- SFTP / 隧道 / 传输工具
```

### 4.1 领域模型与术语

`Session` 专指项目级容器：

- `muxSessionId`：持久化项目容器；
- `paneId`：Session 分割树中的稳定叶节点；
- `runtimeId`：某个终端 Pane 当前的一次运行实例；
- `agentId`：终端 Runtime 中可选的 Coding Agent 进程；
- `browserResourceId`：属于 Session 的持久化浏览器定义；
- `targetId`：PowerShell、WSL、macOS Shell 或 SSH 目标。

一个 Mux Session 包含名称、项目根目录、递归终端布局和零到多个 Browser Resource。Pane 可覆盖目标与工作目录。Terminal Pane 最多拥有一个实时 Runtime；重启 Runtime 不改变 `paneId`。Browser Resource 不属于分割树。

### 4.2 终端后端

所有终端后端实现同一组操作：创建、写入、缩放、流控、中断、关闭、列举、读取输出和报告能力，并以 `runtimeId` 发出标准化状态、带游标输出和退出事件。

当前 Runtime 在 Luna Mux 进程内运行。未来 `DaemonTerminalBackend` 可在同一接口后实现附加/分离、IPC、持久输出和远程桥接，但启用前必须定义认证、进程所有权、关闭、背压、重连和版本协商。守护进程字段不能泄漏到 `TerminalPane`。

本地与 SSH Runtime 共用 React `TerminalPane` 和 xterm.js。主题、字体、背景、搜索、复制粘贴、快捷键、缩放、UTF-8、WebGL 回退和输出流控保持一致。SFTP 和 SSH 转发只由能力标记控制，不分叉终端界面。

- Windows 使用 ConPTY、PowerShell 7/5.1 和已选择的 WSL 发行版，并通过 Job Object 清理进程树。
- macOS 使用 Unix PTY 和用户配置的 zsh/bash，并通过进程组和信号清理。
- 关闭活动 Pane 需要确认；退出应用时一次确认并清理全部 PTY、SSH Channel 和受管 Chrome。

#### 4.2.1 数据流与输出背压

```text
Terminal Runtime
  -> backend (SSH russh / local portable-pty)
  -> Rust UTF-8 增量解码与 runtime event
  -> Tauri `terminal-runtime:event`
  -> React TerminalPane
  -> xterm.js
```

应用不嵌入 Terminal.app、Ghostty 或 Windows Terminal。每个终端 Pane 对应一个独立的 Terminal Runtime；当前 SSH 实现由一条 Rust SSH connection 和 PTY channel 承载，断开其中一个不会影响其他 Pane。

Rust 后端通过 `russh` 请求 `xterm-256color` PTY，并把窗口尺寸变化直接发给远端。本地 PTY 采用 `portable-pty`；Windows ConPTY 启动时可能发送 `ESC[6n` 光标位置查询，正式的 xterm.js `TerminalPane` 负责响应，PTY 后端不自行实现终端仿真。

SSH 和本地 PTY 数据块在 Rust 端用增量 UTF-8 解码器处理，跨数据块的多字节字符不会被截断。Runtime 输出使用有界环形缓冲区和 UTF-8 字节游标，`TerminalPane` 挂载时从游标增量追赶。xterm 写入队列积压时，前端通过流控暂停后端读取，积压恢复后再继续，避免大量输出长期占用 WebView 事件循环。

运行中的 Pane 在跨 Session 和 Session 内视图切换时保持同一个 xterm 实例，非当前工作区只隐藏并停用 WebGL 渲染。确实需要卸载组件（如布局重排）时，使用 xterm 官方序列化格式暂存正常屏幕、备用屏幕和滚动缓冲区，并从最后完成渲染的 UTF-8 字节游标继续追赶；不能只重放有界 PTY 原始输出，因为全屏 TUI 的清屏和光标控制序列无法重建已丢失的滚动历史。删除 Pane 或 Session 时同步丢弃对应快照。

终端启用 WebGL 插件，初始化失败时自动回退到 xterm 默认渲染器。透明背景由 xterm 透明画布与终端容器背景图组合实现，不改变整个系统窗口透明度。

#### 4.2.2 安全边界

WebView 只能通过 Tauri 命令调用显式注册的 Rust 能力。外部链接仅允许 HTTP/HTTPS，文件选择使用原生对话框，凭据保存在 macOS 钥匙串或 Windows 凭据管理器。终端和文件传输不启动 Node.js 子进程或辅助进程。

### 4.3 Mux Session 与窗格

Mux Session 通常对应一个项目，项目根目录为本地终端和项目工具提供默认工作目录，也允许为空。每个 Pane 可单独覆盖工作目录和目标。Agent 是终端中的被检测进程，不是 Pane 类型。

布局是由 Pane 叶节点和横向/纵向分割节点组成的递归树，每个分割节点带比例。用户可以分割、缩放、聚焦、最大化、恢复、重命名和关闭 Pane，也可以应用平衡横排、竖排或网格预设。布局变化不应重启已有 Runtime。

数据库持久化 Session、项目根目录、布局、Pane 目标/工作目录/标题、兼容用启动配置字段和 Browser Resource。重启应用只恢复定义，不自动重连、执行 Shell、启动 Chrome 或恢复滚动缓冲区；Runtime ID 和进程 ID 不跨应用重启复用。

主侧边栏只包含 Session 和 Pane 树。新增工作始终从选中的 Session 开始：`新增窗格` 创建普通终端并选择本地或 SSH 环境；用户可在其中运行 `codex`、`claude` 或其他命令。Browser Resource 从 Session 的浏览器视图管理。SSH 连接保存在次级资源库，不与 Session 并列为主导航。

### 4.4 Agent 适配器集成

不存在特殊的 Agent Pane。用户可以在任何普通终端中手动启动受支持的 Agent。Luna Mux 为 Runtime 注入稳定的 Session/Pane/Runtime 上下文和窄权限启动凭据；Runtime 本地 shim 会立即上报 `AgentProcessStart`，结构化 `SessionStart` 再绑定提供方会话。`SessionEnd`、进程退出或 Runtime 退出会清理身份。

`AgentAdapter` 是原生扩展边界。每个适配器负责：

- 内置启动配置和自动配置编号；
- 手动命令 shim 与托管启动命令；
- Hook/MCP 配置；
- 远程 Hook 传输要求；
- 可选持久化用户 Hook 的兼容策略。

Runtime、面板、Hook 接收器、Luna MCP、Browser Resource、授权和审计保持提供方无关。增加 Agent 提供方时，应实现并注册一个适配器，再补充契约测试。

`启动 Agent` 只是便利入口：它创建普通 Terminal Pane、保存所选配置，并在 Shell 就绪后启动 Agent 命令。普通 `新增窗格` 仍只启动终端。

Codex 与 Claude Code 的结构化 Hook 统一为：

- `working`；
- `waiting`，原因可为 `input`、`permission`、`external` 或 `unknown`；
- `completed`；
- `error`。

Codex 使用进程级 TOML 覆盖和命令 Hook；Claude Code 使用进程级 `--settings` HTTP Hook 和 `--mcp-config`。正常集成不会重写 `~/.claude`。两者共用 Luna MCP 和原生 `agent_browser` MCP。

远程 Agent 集成默认关闭。关闭时，普通 SSH Pane 不探测 Agent 命令、不打开 SFTP、不上传文件、不建立反向转发，也不修改交互 Shell。启用前必须展示远程文件变更和审计/EDR 风险。

启用后，支持文件全部放在 `~/.luna-mux/runtime/<runtime-id>`；该目录只加入当前 Pane 的 `PATH`。Luna Mux 不改远程 Shell 启动文件或用户 Agent 配置，正常断开前通过 SFTP 删除该精确目录。网络中断或进程崩溃时无法保证远程清理。

权限请求仍由 Agent TUI 审批；Luna Mux 负责通知并聚焦对应终端。Agent 视图显示提供方、所有者 Pane、目标、启动方式、Hook、Luna MCP 和 Browser MCP 健康信息；未读和注意状态继续体现在侧边栏和终端边框。

### 4.5 统一 Luna 控制 API

`LunaControlService` 是人与本地 AI Agent 操作应用的唯一核心边界。受信任桌面 UI、本地 Luna MCP 以及未来 CLI/IPC 都调用同一操作目录；适配器认证调用方后才能调用服务，不得直接访问 SQLite、`SessionManager`、终端后端或 Browser Runtime。

资源图如下：

```text
Application
`-- Mux Session
    |-- Pane
    |   `-- Terminal Runtime
    |       `-- 可选 Agent 进程
    `-- Browser Resource
        `-- 可选 Browser Runtime
```

请求、授权、事件和审计使用稳定资源 ID，不使用显示名称或进程 ID。操作目录通过能力发现逐步扩展，当前范围包括：

- Session、Pane、终端目标、Runtime、Agent、传输和隧道的发现与状态；
- Session/Pane 元数据、Pane 创建和完整布局更新；
- 有界终端输出读取、写入、缩放、流控、中断和关闭；
- Agent 状态、任务投递和中断；
- 白名单应用设置读取与更新；
- 有界控制事件流。

Agent 可见 MCP 遵循以下规则：

- 安全全局设置使用独立 `Settings` 授权，绝不向 Agent 发放 `Application` 超级资源；
- 连接发现只返回操作元数据，不返回凭据值、私钥内容或 AI Key；
- Session、Pane 和布局写入限制在调用方当前 Session；
- 创建 Pane 时持久化 Pane 和分割树，再通知实时桌面通过正常 Tauri 路径启动 Runtime；并发创建串行执行，避免布局互相覆盖；
- 完整布局必须且只能包含当前 Session 的全部 Pane，每个 Pane 恰好一次，比例和嵌套深度受限；
- 主题和终端外观同时更新 SQLite、原生窗口和 WebView；
- 传输与隧道观察按 Runtime 限制；启动传输/隧道、关闭 Runtime 等重要操作保留桌面审批；
- 浏览器自动化不进入 Luna MCP 工具目录，只由 Session 感知的 `agent_browser` MCP 提供。

控制信封包含契约版本、请求编号、可选资源范围、参数和幂等键。调用方身份由认证适配器注入，不出现在请求体中。未授权、版本过期、参数错误、不可用和内部错误使用结构化错误码。修改操作定义幂等行为，重试不能重复创建资源或副作用。

#### 4.5.1 跨窗格与跨 Agent 控制

Pane 是资源，不是调用方。终端中的 Agent 获得认证身份；Mux Session 是协作与安全边界。默认情况下，它可以发现同 Session 的 Pane、实时 Runtime 和 Agent，但不能访问其他 Session。普通 Shell Pane 与 Agent Pane 同样是一级控制目标。

常用操作示例：

```text
terminal.runtime.output.read  读取同 Session 实时 Pane 的有界输出
terminal.runtime.write        向同 Session 实时 Pane 写入 PTY 输入
terminal.runtime.interrupt    中断同 Session 的 Terminal Runtime
agents.get_status             读取已检测 Agent 的结构化状态
agents.send_task              通过目标适配器投递任务
agents.interrupt              中断已检测 Agent
mux.pane.create               创建 Pane、插入布局并按需启动 Runtime
mux.layout.set                写入经过完整校验的 Session 分割树
```

同 Session 成员关系本身就是输出读取、PTY 写入和 Agent 任务投递的信任决定，不再增加第二层 Luna Mux 审批。关闭、传输、隧道和其他破坏性生命周期操作仍按各自策略审批。跨 Session 权限不能由显示名称、进程父子关系或相同 SSH 目标推断。

终端输出使用有界内存环和单调游标；旧输出被覆盖时返回截断标记和新的最早游标。应用退出后输出消失。控制操作默认保留 30 天审计，记录调用方、目标、时间、操作、输入摘要、审批和结果。

### 4.6 浏览器资源

Chrome 始终运行在 Luna Mux 所在桌面。每个 Browser Resource 属于一个 Mux Session，使用隔离的持久配置目录，也可请求一次性临时目录。

Browser 不是 Pane，不参与终端分割、缩放或最大化。用户在标准外部 Chrome 窗口中交互；Luna Mux 浏览器视图只管理资源名称、生命周期、启动、聚焦、重启、停止和删除。启动本地资源只打开一个可控的 `about:blank` 页面，不自动访问网络地址；旧数据库 `url` 字段仅用于兼容历史远程记录。

Chrome CDP 绑定随机回环端口。Browser Runtime 的进程 ID、CDP 端口、WebSocket 和临时转发地址不进入 SQLite，也不持续投屏。Agent 只在需要时请求可访问性快照或截图。

首版每个 Session 只允许一个运行中的受管 Chrome。包装器从环境继承 `muxSessionId`；同 Session 出现多个 Browser Runtime 时拒绝隐式选择。直接 CDP 无法强制每 Agent 授权，因此在未来加入认证 CDP 代理前，不宣称存在细粒度权限。

Luna Mux 不重复实现浏览器自动化。它启动隔离的外部 Chrome，并将固定版本、校验过的原生 `agent-browser` 连接到稳定回环 CDP。该工具提供稳定引用、交互、等待、截图、标签页、控制台、网络检查和类型化 MCP。旧 `browser.*` Luna MCP 方法只保留内部迁移/诊断用途。

`luna-mux mcp browser` 根据当前 Session 解析端点并启动内置 sidecar；`luna-mux mcp chrome` 只作为兼容别名。分发时不依赖 Node.js 或 `npx`，Node 只用于仓库的 Vite/Tauri 构建。

SSH Pane 使用上传的通用 Python STDIO 代理，通过 SSH 服务端分配的远程回环端口和 Runtime 随机凭据连接桌面。桌面验证后启动同一个 Session 级 sidecar。原始 CDP 不转发到远端，代理不内嵌凭据，桥接随 Runtime 失效。

在 Luna Mux 终端中，进程级配置会禁用不可用且会抢占请求的 Codex Browser Plugin、`node_repl` 浏览器路径和对应缓存 Skill，同时合并用户原有 Skill 配置，不修改全局文件。Browser 请求只保留 `agent_browser` 一条受支持路线。

工具路由首先区分三个资源域：Luna Mux 自身拥有的应用设置、连接摘要、Session、Pane、Terminal Runtime、受管 Agent、SFTP 传输和 SSH 隧道交给 Luna MCP；URL、网页、DOM、链接、表单、页面截图、浏览器标签页/窗口、浏览器控制台和页面网络流量交给 `agent_browser`；源码、文件、Git、构建、普通 Shell 命令、操作系统设置和外部服务交给对应的原生工具。Agent 不能仅因为自己运行在 Luna Mux 中就把普通开发操作路由给 Luna MCP，也不能用 `agents.*` 代替自身的子 Agent/委派机制。

每项 Luna MCP 工具描述都说明资源所有者、典型用户意图和相邻域的反例。未限定语境的“窗格”、Pane、分屏和布局始终指 Luna Mux 应用资源，分别交给 `mux.pane.create`、`mux.panes.list` 和 `mux.layout.set`，不得映射为浏览器标签页或窗口。浏览器路由约束还包括：交互前先获取当前 URL 和快照、复用当前页面、禁止通过 Shell 启动或恢复浏览器、阻止安装/升级/连接/关闭等资源生命周期工具。Agent 在确有上下文隔离或多页面需求时可自主使用标签页和窗口，但日常导航和错误恢复不应无故新建标签页。

启动按首次真实工具调用触发。每个 Session 预留稳定 CDP 端口，即使 Chrome 尚未运行，MCP 也能初始化并发布目录。`PreToolUse` 在第一项浏览器调用前要求 Luna Mux 启动唯一候选资源、等待 CDP、预热 Session 级 daemon，再放行原调用。多个 Agent 复用同一端口和 Runtime。

预热解决 `agent-browser` 0.34.0 在 Windows 上创建后台 daemon 时可能继承 STDIO Handle 而导致等待 EOF 的问题；macOS 保持同一路径。如果 Chrome 重启或固定标签页消失，预热会临时解除 pin、采用现有页面、恢复严格 pin 并验证，不创建新标签页。

远程开发服务继续使用独立 SSH 本地转发，然后由浏览器访问回环 URL。新建 Browser Resource 不选择 SSH Pane，也不持久化隧道所有权；历史 `sourcePaneId` 只为迁移兼容而读取。

## 5. 数据与兼容

Luna Mux 使用 `com.luna.mux`、`luna-mux.db` 和 `com.luna.mux.credentials`，不得自动打开或修改 Luna Remote 存储。

导入向导可复制选中的连接、分组、主机密钥、设置和转发配置。凭据复制需要单独明确选择，并通过系统凭据库完成。导入读取稳定快照，并事务写入 Luna Mux。

Windows 和 macOS 都是首版平台。里程碑只有在适用的双平台验收通过后才完成。签名、公证、自动更新和分发治理暂不在当前范围。

## 6. 交付顺序

1. 建立独立仓库、产品元数据、隔离身份、数据导入边界和功能参考边界。
2. 抽取 `TerminalBackend`，移除 `TerminalPane` 中的 SSH 假设。
3. 加入 Windows PowerShell/WSL 与 macOS zsh/bash PTY，并统一终端行为。
4. 用项目级 Mux Session 和持久递归分屏替换单终端标签模型。
5. 加入 Agent Adapter、状态面板、事件转发和通知。
6. 实现 Luna Control Service，再通过该边界提供 Luna MCP 和跨 Agent 操作。
7. 加入 Session 级受管 Chrome、原生浏览器 MCP 和远程服务转发。
8. 按需参考相关产品能力，由 AI 重新实现并执行跨平台回归。
