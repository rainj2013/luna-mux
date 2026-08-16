# Luna Mux 开发进度

本文档是长期开发检查点。凡实现状态发生变化的开发会话都应更新它。任务的权威清单仍是 [DEVELOPMENT_TASKS.md](DEVELOPMENT_TASKS.md)；本文记录当前重点、关键决策、验证证据和交接信息。

## 当前检查点

- 日期：2026-08-16
- 阶段：提供方无关的 Agent Adapter、运行时 Agent 检测、原生 `agent-browser` MCP，以及 Luna MCP 应用控制。
- 当前任务：M4.4、M5.1、M6.8——收尾真实 SSH 的 Hook/Luna MCP，以及 WSL 和 Claude Code 的 Browser MCP 端到端验收；远程 SSH Agent 浏览器操作已经通过。
- 任务进度：65 项中 53 项完成（81.54%），8 项进行中，4 项待办。
- 总体状态：Session/Pane 是产品外壳，每个 Pane 都是普通终端。`AgentAdapter` 注册表负责 Codex 与 Claude Code 的提供方差异；检测、面板、Luna MCP、Browser Resource、权限和审计保持共享。外部 Chrome 归 Session 所有，分发版浏览器 MCP 不依赖 Node 或 `npx`。
- 当前工作目录：`/Users/yangyujian/code/luna-mux`

## 已完成

### 产品、终端和工作区

- 建立独立产品身份、仓库历史、数据库、凭据命名空间、图标和导入边界；Luna Remote 仅作为功能与体验参考，不建立 Git 同步关系。
- 抽取统一 `TerminalBackend`，本地 PTY 与 SSH 共用 Runtime 契约、输出游标、流控和 React `TerminalPane`。
- Windows PowerShell 7、本地 PTY、WSL 发现逻辑和 macOS zsh/bash 路径已进入组合后端；进程关闭和应用退出具备清理边界。
- 建立持久化 `MuxSession`、`MuxPane`、递归 `MuxSplitNode` 和 SQLite 迁移；启动时恢复布局但不自动启动进程。
- 主导航收敛为 Session/Pane 树；SSH 连接是次级目标资源；Browser Resource 不进入分割布局。
- 完成横纵分屏、比例拖动、最大化、重命名、关闭和布局预设的实现路径。
- Windows 与 macOS 已分别完成一轮真实桌面开发验证；macOS 近期高频使用覆盖本地 PTY、登录 Shell、Pane 创建、分屏、比例调整、焦点和开发重启清理。

### Agent 适配器、Hook 与通知

- 建立提供方无关的 `AgentAdapter` 注册表，包含 Codex 和 Claude Code。
- 普通终端中手动执行 `codex` 或 `claude` 时，通过 Runtime 本地 shim 立即注册进程；结构化 Hook 绑定会话并在退出后清理身份。
- Codex 使用进程级 TOML/Hook，Claude Code 使用 `--settings` 和 `--mcp-config`；正常集成不修改用户全局配置。
- 远程 Agent 集成改为显式、默认关闭。启用后的脚本、凭据和 shim 隔离在 `~/.luna-mux/runtime/<runtime-id>`，正常断开前通过 SFTP 精确删除。
- Agent 事件只存生命周期元数据，不保存提示词、工具输入或工具输出。终端活动、BEL 和 OSC 只作为启发式证据。
- 等待、完成和错误事件已接入侧边栏和 Pane 边框提醒，并在对应 Pane 聚焦时抑制重复提醒。
- Windows 系统通知由 Rust Hook 事件源直接发送；macOS 使用单个隐藏 WebView 实现 Luna Mux 主题通知，8 秒自动收起且悬停时暂停计时，不依赖签名、系统通知权限、辅助进程或回环服务。点击后都会唤醒主窗口并切换到事件所属 Session/Pane。
- Codex 与 Claude Code 适配器已在 macOS 验证进程级启动注入；Agent 环境视图实际用于定位 Hook、Luna MCP、Browser MCP 和代理启动故障。

### Luna 控制与 Luna MCP

- 建立版本化、传输无关的 `LunaControlService`，包含调用方认证、资源授权、结构化错误、幂等、审批、审计和有界事件游标。
- Luna MCP 使用仅回环的 Streamable HTTP，Bearer Token 绑定 Runtime，退出时撤销；本地和 SSH Agent 共用同一控制边界。
- 同一 Mux Session 是协作边界：Agent 可发现并控制同 Session 的 Pane、Runtime 和 Agent，无法跨 Session。
- 已提供 Agent 列表、状态、任务、中断、有界终端输出读取、PTY 写入、缩放、流控和关闭操作。
- 设置 API 支持读取外观、修改主题和终端外观，并实时同步 SQLite、原生窗口和 React。
- 连接 API 只返回无凭据摘要；密码、私钥内容和 AI Key 不进入 MCP。
- 已提供 Session/Pane 元数据更新、`mux.pane.create` 和 `mux.layout.set`。Pane 创建会串行持久化 Pane 与布局并通知桌面启动 Runtime；完整布局严格校验 Session、叶节点、比例、重复、遗漏和深度。
- 已提供 Runtime 范围的传输、隧道和转发配置观察；关闭、启动传输/隧道等重要副作用保留桌面审批。
- 控制审计默认保留 30 天；终端输入、任务正文、浏览器输入和脚本只记录字节数等摘要。

### 浏览器资源与原生浏览器 MCP

- Browser Resource 已从 Pane 模型中移除，成为 Session 级外部 Chrome 生命周期对象；界面管理创建、启动、聚焦、重启、停止和删除。
- 每个资源使用隔离持久 Chrome 配置目录；首版强制每 Session 一个运行中的 Chrome。
- 原生 `agent-browser` 0.34.0 被固定为 Agent 浏览器契约，支持稳定快照引用、交互、等待、标签页、截图、控制台和网络检查。
- `luna-mux mcp browser` 包装器按 Session 解析预留 CDP 端点并启动带校验和的 sidecar；`mcp chrome` 仅作兼容别名。
- Agent 可在 Chrome 尚未启动时完成 MCP 初始化；首次真实浏览器工具调用会按需启动唯一 Browser Resource、等待 CDP 并预热 daemon。
- 修复首次调用后台 daemon 的 STDIO Handle 继承问题、Chrome 重启后的陈旧标签 pin、重复首标签页和崩溃恢复提示。
- 进程级配置禁用不可用的 Codex 私有 Browser Plugin、`node_repl` 路径和冲突 Skill，只保留 Session 感知的 `agent_browser` 路线，不修改用户全局配置。
- Codex 与 Claude Code 的进程级提示先执行三域语义分流：Luna Mux 自有资源使用 Luna MCP，网页内容使用 `agent_browser`，源码、Shell、Git、操作系统和外部服务使用对应原生工具。全部 Agent 可见的 Luna MCP 操作都补充了资源所有者、典型意图和反例；未限定的“窗格”、Pane、分屏和布局明确归 Luna Mux，不能映射成浏览器标签页或窗口。
- SSH Agent 使用认证的通用 STDIO 代理转发 MCP，不向远端暴露原始 CDP。
- macOS 真实 Codex 浏览器任务已完成当前页面复用、首页与文章页导航、性能 API 采样、网络请求、HAR、控制台和缓存前后对比；生命周期日志确认复用同一受管 Chrome。
- macOS 已验证 Chrome 发现、隔离配置、按需启动、外部窗口交互和应用开发重启清理，补齐此前只有 Windows 实机证据的缺口。
- 真实 SSH 远程 Agent 已通过认证 STDIO 代理操作本机受管 Chrome，确认远程 Browser MCP 不需要向远端暴露 CDP，也不会另起浏览器后端。

### 已淘汰方案

- 不再把 Coding Agent 作为 Pane 类型；Agent 是普通终端中的进程。
- 不再将 Browser 作为持续投屏 Pane；用户在外部 Chrome 中交互。
- 不再把自研 `browser.*` 或 `chrome-devtools-mcp` 作为 Agent 正式浏览器契约。
- 不再通过禁用全部插件强制浏览器路由；当前方案保留无关插件和用户配置。
- 不再把 Browser Resource 绑定到 SSH Pane；远程服务访问归独立隧道流程。

## 进行中

- M1.2：`InProcessSshTerminalBackend` 已接入，但旧 SSH/SFTP/隧道路径仍需完整回归。
- M2.3：Windows WSL 发现和启动代码已实现；仍需真实 WSL 发行版完成工作目录、命令、Agent shim 和清理复验。
- M4.4、M5.1：远程 Hook、Luna MCP 反向转发和清理代码已实现；待允许 TCP 转发的真实 SSH 服务验收。
- M4.6：跨平台通知已编译；待 Windows 实际通知和 macOS 权限流程验收。
- M4.13、M5.7：macOS 本地 Runtime 的窄权限 Hook/MCP 和手动启动已验证；真实 SSH 与 WSL 的完整身份交接仍待复验。
- M6.8：macOS 本地 Codex 和真实 SSH 远程 Agent 的浏览器任务已通过；仅剩 Claude Code 与真实 WSL 路径的等价验收。
- Microsoft Pinyin 在受管 Codex TUI 中的全角智能引号差异暂缓处理；证据不足以归因于 PTY、xterm、WebView2 或 Codex。

## 下一步

1. 用真实 SSH 主机复验 Hook/MCP 反向转发、令牌交接、重连和清理，完成 M4.4 与 M5.1。
2. 在真实 WSL 发行版验证目标发现、工作目录、Agent shim、Luna MCP、Browser MCP 和进程树清理；远程 SSH 浏览器路径无需重复验收。
3. 在最新预览中用 Claude Code 完成一次与 macOS Codex 等价的 Luna MCP 与浏览器任务，收尾 M6.8。
4. 验证 Windows 系统通知的真实 Agent 事件，并持续回归 macOS 应用内通知的点击路由和聚焦抑制。
5. 使用至少两个 Agent 验证同 Session 输出读取、任务投递、跨 Session 隔离、输入、中断、审计、未读和通知。

## 验证证据

| 检查 | 结果 | 日期 |
| --- | --- | --- |
| macOS 桌面与本地 PTY | 连续开发运行覆盖 zsh 登录 Shell、Unicode 终端、Pane 创建、横纵分屏、比例调整、焦点、关闭和 Tauri 热重启清理 | 2026-08-16 |
| macOS Codex、Luna MCP 与原生浏览器 | 真实 Codex 进程完成 Hook/MCP 注入、Luna Mux 设置与 Pane 控制，以及页面复用、导航、性能采样、HAR、控制台和网络调试；Chrome Runtime 持续复用 | 2026-08-16 |
| 远程 SSH Agent 浏览器 | 真实远程 Agent 通过认证 STDIO 代理操作本机受管 Chrome，Browser MCP 页面操作通过，远端未暴露原始 CDP | 2026-08-16 |
| 路由与 MCP 工具描述 | 三域路由覆盖 Luna Mux、网页和普通开发操作；Rust 132 项通过、4 项真实浏览器测试默认忽略 | 2026-08-16 |
| Agent 提醒投递 | Rust Hook 事件源驱动侧边栏和 Pane 边框提醒；Windows 原生通知与 macOS 主题化应用内通知都携带 Session/Pane 路由 | 2026-08-16 |
| Luna MCP 设置、连接摘要、Session/Pane、传输和隧道扩展 | TypeScript 检查通过；Rust 129 项通过、4 项真实浏览器测试默认忽略；完整 macOS DMG 构建成功 | 2026-08-15 |
| `mux.pane.create` 与 `mux.layout.set` | 前端生产构建通过；Rust 130 项通过、4 项忽略；幂等双 Pane 创建和非法布局拒绝测试通过 | 2026-08-16 |
| AgentAdapter 与 Claude Code 基础 | 注册表、配置、Header/进程绑定和共享 Hook 测试通过；Claude Code 2.1.232 接受进程级参数且未修改全局配置 | 2026-08-14 |
| 原生 `agent-browser` 替换验证 | 固定版本和 SHA-256 通过；完成 MCP 初始化、目录发现、快照、输入、回车、导航和结果读取；复用已有 Chrome | 2026-08-14 |
| 首次浏览器工具回归 | 真实单进程测试完成 Chrome 启动、daemon 预热、`agent_browser_get_url`、Runtime 复用和清理 | 2026-08-14 |
| 陈旧标签恢复 | 关闭被固定页面后，通过临时解除 pin、重绑和恢复 pin 复用 Chrome，未创建额外标签页 | 2026-08-14 |
| Codex Browser Skill 路由 | Codex 0.147.0 的 `skills/list` 证明精确 `SKILL.md` 路径被禁用，目录路径不足以禁用 | 2026-08-14 |
| Browser Resource 数据迁移 | 迁移保留稳定资源，删除旧授权，移除 Browser 布局叶并折叠分割树 | 2026-08-13 |
| Windows 真实 Chrome 操作 | Chrome 151 下截图、等待、输入、点击、按键、滚动、求值、聚焦和清理通过 | 2026-08-13 |
| Windows PowerShell 与受管 Codex | Profile 加载、shim 解析、交互提示和受管 Codex 0.147.0 启动通过 | 2026-08-13 |
| Session-first 界面 | 1360×860 与 980×640 下 Session/Pane 层级、浏览器视图和新增入口无重叠 | 2026-08-13 |
| 终端光标根因 | 真实 ConPTY 追踪复现 DEC 光标与同步输出帧；渲染保护、输入恢复及构建检查通过 | 2026-08-13 |
| 本地 PTY | Windows ConPTY 的 Unicode、缩放、中断、自然退出和进程树清理测试通过 | 2026-08-12 |
| 产品与生成契约 | 产品、Runtime、图标、多语言、类型和原生核心检查通过 | 2026-08-12 |
| 安全依赖审阅 | `npm audit` 报告间接 `nanoid` 高风险和 `postcss` 中风险；当前运行路径不接受攻击者控制的相关输入，升级待处理 | 2026-08-12 |

## 决策记录

| 日期 | 决策 |
| --- | --- |
| 2026-08-12 | Luna Mux 是独立产品和仓库，不是 Luna Remote 功能分支。 |
| 2026-08-16 | Luna Mux 不维护 Luna Remote 的 Git 历史、上游 Remote 或提交同步；需要融合的功能由 AI 参考现有代码并按 Luna Mux 模型重新实现，同时保持相近的 UI 与使用体验。 |
| 2026-08-12 | Windows 与 macOS 都是首版平台；签名与分发治理暂不纳入当前范围。 |
| 2026-08-12 | 本地和 SSH 终端共用 `TerminalPane`，差异只通过能力表达。 |
| 2026-08-12 | Agent 随应用退出；`TerminalBackend` 保留未来守护进程扩展点。 |
| 2026-08-12 | `MuxSession` 是项目级容器，Pane 是稳定布局节点，`TerminalRuntime` 是可替换运行实例。 |
| 2026-08-12 | Session/Pane 是唯一主导航；连接只是创建 SSH Pane 的可复用目标。 |
| 2026-08-12 | 定义持久化，实时进程不持久化；启动恢复停止状态的布局。 |
| 2026-08-12 | 控制传输层不得从请求 JSON 反序列化身份；认证适配器注入 `ControlCaller`。 |
| 2026-08-12 | `LunaControlService` 是 Agent 控制应用的唯一核心入口。 |
| 2026-08-12 | Hook Token 只通过进程环境传递，绑定 Runtime 并在退出时撤销。 |
| 2026-08-12 | Agent 历史和未读状态首版仅保存在有界内存中。 |
| 2026-08-13 | 同一 Mux Session 是跨 Pane、跨 Agent 协作和授权边界。 |
| 2026-08-13 | 幂等键与调用方、操作、资源和完整参数不可分割。 |
| 2026-08-13 | 终端活动、BEL 和 OSC 只能作为启发式证据。 |
| 2026-08-13 | Luna MCP 是应用拥有的仅回环 HTTP 服务；外部客户端不能自行批准操作或删除审计。 |
| 2026-08-13 | Agent 不是 Pane 类型；手动启动的受支持进程由 Hook 动态注册。 |
| 2026-08-13 | Browser 是 Session 级资源，不是 Pane；用户使用外部 Chrome。 |
| 2026-08-14 | Browser Resource 只负责本地 Chrome；远程服务访问使用独立 SSH 隧道。 |
| 2026-08-14 | 使用固定版本的原生 `agent-browser` 作为 Agent 浏览器契约，淘汰运行时 Node/`npx` 依赖。 |
| 2026-08-14 | Sidecar 是带版本和平台 SHA-256 的构建输入。 |
| 2026-08-14 | Agent 提供方由原生 `AgentAdapter` 实现，不由 Pane 类型或前端分支实现。 |
| 2026-08-15 | 普通 SSH Pane 的 Agent 集成默认关闭；启用后使用 Runtime 隔离文件和认证反向转发。 |
| 2026-08-15 | Agent 可读无凭据连接摘要和安全设置；不得获得 `Application` 超级授权。 |
| 2026-08-16 | Pane 创建和布局写入进入 Luna MCP；并发修改串行化，完整布局必须通过严格 Session 校验。 |
| 2026-08-16 | 仓库自有说明文档统一使用中文；代码标识、命令和协议字段保留原名，许可证保留法律原文。 |

## 开发更新规范

今后的每次实现会话必须：

1. 修改前阅读本文和 [DEVELOPMENT_TASKS.md](DEVELOPMENT_TASKS.md) 中的当前里程碑。
2. 除非明确记录并行工作，否则只设置一个主任务为 `IN_PROGRESS`。当前主任务是 M6.8。
3. 重要决策立即加入决策记录。
4. 只有写入验证命令和结果后，任务才能标记为 `DONE`。
5. 结束前更新“当前检查点”“已完成”“进行中”和“下一步”。
6. 不能仅因代码已写完就宣称完成，必须提供验收证据。
