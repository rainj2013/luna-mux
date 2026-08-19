# Luna Mux 开发任务

本文档是按顺序维护的长期任务清单。任务开始后编号不再改变。状态值使用 `TODO`、`IN_PROGRESS`、`BLOCKED` 和 `DONE`。

## M0——独立产品基础

| 编号 | 状态 | 任务 | 验收条件 |
| --- | --- | --- | --- |
| M0.1 | DONE | 创建 Luna Mux 独立仓库 | 仓库、提交历史和发布流程独立存在 |
| M0.2 | DONE | 建立 Luna Remote 功能参考边界 | 不配置 Git 上游或同步历史；需要的能力由 AI 参考代码并按 Luna Mux 模型重新实现 |
| M0.3 | DONE | 建立产品元数据单一来源 | 元数据可校验并生成前端、Rust 标识常量和清单 |
| M0.4 | DONE | 隔离运行时身份和存储 | 包、数据库、凭据、标题、诊断和备份标识全部使用 Luna Mux 命名空间 |
| M0.5 | DONE | 用生成的产品信息替换品牌字面量 | 运行和构建代码中没有可手工编辑的重复品牌值 |
| M0.6 | DONE | 增加显式 Luna Remote 导入向导 | 支持选择性快照导入；凭据需单独确认；来源永不修改 |
| M0.7 | DONE | 建立设计、任务和进度文档 | 三份长期文档存在并互相引用 |
| M0.8 | DONE | 替换继承的 SSH 风格图标 | Windows 和 macOS 拥有原创默认图标和明确的可选图标 |

## M1——终端运行时抽象

| 编号 | 状态 | 任务 | 验收条件 |
| --- | --- | --- | --- |
| M1.1 | DONE | 定义运行时编号、目标、能力、事件和游标语义 | Rust 与 TypeScript 契约一致；状态、输出和退出测试不混用项目 Session 术语 |
| M1.2 | DONE | 用 `InProcessSshTerminalBackend` 封装现有 SSH 管理器 | SSH、缩放、流控、关闭、SFTP 和隧道回归通过；新 API 使用 `runtimeId` |
| M1.3 | DONE | 让 `TerminalPane` 与传输方式无关 | 渲染组件不包含 SSH 专属决策，只按 `runtimeId` 绑定 |
| M1.4 | DONE | 增加模拟终端后端契约测试 | 替换后端不影响 Mux Session、Pane 或 `TerminalPane` 行为 |
| M1.5 | DONE | 记录未来守护进程终端扩展契约 | 只定义运行时后端行为，不提前实现 IPC 协议 |

## M2——跨平台本地 PTY

| 编号 | 状态 | 任务 | 验收条件 |
| --- | --- | --- | --- |
| M2.1 | DONE | 在 Windows 和 macOS 验证 PTY 库 | `portable-pty` 已在 Windows 和 macOS 通过 Unicode、缩放、中断、自然退出和清理验证 |
| M2.2 | DONE | 实现 Windows PowerShell 7 目标 | 强制使用 `pwsh.exe`；行为与 SSH 一致；关闭时清理子进程树 |
| M2.3 | DONE | 实现 Windows WSL 发现和目标 | 已安装发行版可选择，工作目录和命令启动可预测 |
| M2.4 | DONE | 实现 macOS zsh/bash 目标 | 仅接受 zsh/bash；本地登录 Shell、Agent shim、进程组清理和重复开发重启已在 macOS 实机验证 |
| M2.5 | DONE | 统一终端能力和目标名称 | 只有远程工具存在差异，终端控制和外观保持一致 |
| M2.6 | DONE | 增加活动进程关闭和应用退出确认 | 确认退出后不残留受管 PTY 进程 |

## M3——Mux Session 与分屏窗格

| 编号 | 状态 | 任务 | 验收条件 |
| --- | --- | --- | --- |
| M3.1 | DONE | 定义 `MuxSession`、Pane、分割树和启动配置结构 | 一个项目 Session 拥有递归终端 Pane；Pane 可覆盖工作目录和目标 |
| M3.2 | DONE | 增加 SQLite 迁移和仓储 API | 现有数据事务迁移，布局可完整往返 |
| M3.3 | DONE | 实现横向/纵向分割与尺寸调整 | Windows 和 macOS 实机开发中已反复使用横纵分屏与比例调整；受支持窗口尺寸下 Pane 不重叠 |
| M3.4 | DONE | 实现焦点、布局预设、重命名、最大化和关闭 | 布局变化不重启运行时；名称和布局持久化；Windows 与 macOS 键鼠流程已验证 |
| M3.5 | DONE | 恢复布局但不自动启动进程 | 重启后恢复 Pane 和启动配置，所有 Terminal Runtime 保持停止 |
| M3.6 | DONE | 将 SFTP、隧道和传输改为选中目标的工具 | 原能力保留，但不成为 Pane 类型 |
| M3.7 | DONE | 让 Session/Pane 成为唯一主导航 | 侧边栏只显示 Session/Pane 树，SSH 目标进入次级资源库 |
| M3.8 | DONE | 统一为普通终端 Pane 创建流程 | 新增 Pane 只选择本地或 SSH 目标；旧数据仍按终端打开 |

## M4——Agent 适配器与状态面板

| 编号 | 状态 | 任务 | 验收条件 |
| --- | --- | --- | --- |
| M4.1 | DONE | 实现 Luna 管理的 Agent 启动配置 | 每个进程绑定 `muxSessionId`、`paneId`、`runtimeId` 和 `agentId` |
| M4.2 | DONE | 实现本地结构化 Hook 接收器 | 工作、等待、完成和错误状态转换确定 |
| M4.3 | DONE | 实现可预览的适配器安装与卸载 | 用户现有 Hook 被合并和备份，不整体覆盖 |
| M4.4 | DONE | 实现远程 Hook 上传和反向转发 | 事件绑定 SSH 连接、终端运行时和随机令牌 |
| M4.5 | DONE | 构建活动 Agent 运行视图 | 已在 macOS 多轮 Agent 启动和 MCP 故障排查中验证提供方、Session、Pane、Runtime、Hook、Luna MCP、Browser MCP 健康状态及侧边栏提醒 |
| M4.6 | DONE | 增加 Windows/macOS 桌面通知 | Windows 由 Rust Hook 事件源驱动系统通知；macOS 使用跟随 Luna Mux 主题的轻量应用内通知窗口，不依赖签名、系统权限或辅助进程。两端点击都可唤醒应用并切到对应 Session/Pane |
| M4.7 | DONE | 增加仅终端的回退信号 | OSC、BEL 和活动信号明确标为启发式，不能冒充权威状态 |
| M4.13 | DONE | 为每个 Terminal Runtime 提供受限 Luna 上下文 | 本地和 SSH Runtime 获得窄权限 Hook/MCP 凭据 |
| M4.14 | DONE | 发现手动启动的 Agent | 进程启动即显示，`SessionStart` 绑定会话，退出后清除，不产生重复身份 |
| M4.15 | DONE | 将 Agent 控制限制在实时 Session 资源 | Agent 自动获得同 Session 的 Pane、Runtime 和 Agent 权限，无法跨 Session |
| M4.16 | DONE | 抽取 `AgentAdapter` 注册表并加入 Claude Code | Codex 与 Claude Code 共享 Hook、Luna MCP 和 Browser 基础设施；手动启动、快捷启动及 macOS 进程级注入已验证 |

## M4.8～M4.12——Agent 控制 Luna Mux

这些任务让外部 AI Agent 操作 Luna Mux 本身，与跨 Agent 协作分开。所有传输层必须调用 `LunaControlService`，不得直接访问 SQLite、`SessionManager` 或终端后端。

| 编号 | 状态 | 任务 | 验收条件 |
| --- | --- | --- | --- |
| M4.8 | DONE | 定义 `LunaControlService` 操作目录和版本化信封 | 调用方、授权、结构化错误、幂等和事件游标都有测试 |
| M4.9 | DONE | 增加认证后的 Agent 控制适配器 | 适配器注入 `ControlCaller`，请求 JSON 不能伪造身份 |
| M4.10 | DONE | 暴露 Session、Pane、Runtime 和目标发现及安全控制 | 修改操作使用显式权限，不暴露数据库访问 |
| M4.11 | DONE | 为破坏性操作增加审批策略 | 关闭、传输、隧道和浏览器副作用可要求用户批准；同 Session 协作保持即时 |
| M4.12 | DONE | 增加控制审计和事件游标读取 | 授权调用者获得有界事件；审计记录调用方、资源、操作、结果和时间 |

## M5——跨 Agent 的 Luna MCP

| 编号 | 状态 | 任务 | 验收条件 |
| --- | --- | --- | --- |
| M5.1 | DONE | 实现有范围的进程内 MCP 传输 | 本地和 SSH Agent 只能用 Runtime 令牌连接；应用控制复用 M4.8～M4.12 |
| M5.2 | DONE | 增加 Agent 列表、状态、任务和中断工具 | 只能发现获授权的 Agent |
| M5.3 | DONE | 增加有界输出环和游标读取 | 增量读取及截断语义通过契约测试 |
| M5.4 | DONE | 将 Mux Session 设为协作边界 | 同 Session 实时资源自动可见，其他 Session 不可访问 |
| M5.5 | DONE | 增加原始 PTY 写入 | 输入对目标实时终端只投递一次 |
| M5.6 | DONE | 增加 30 天审计保留和清理 | 调用方、目标、操作、摘要、结果和时间可查询及清理 |
| M5.7 | DONE | 让手动启动的 Agent 获得 Luna MCP | PowerShell、macOS 和 SSH 终端获得 Runtime 级身份且不修改用户全局配置 |
| M5.8 | DONE | 让远程 Agent 集成显式且可恢复 | 默认关闭；启用后的文件隔离在 Runtime 目录并在正常断开前清理 |

## M6——浏览器资源

| 编号 | 状态 | 任务 | 验收条件 |
| --- | --- | --- | --- |
| M6.1 | DONE | 在 Windows/macOS 发现并启动 Chrome | 两个平台均已实机发现并启动 Chrome；CDP 只绑定回环地址，受管进程随 Luna Mux 关闭 |
| M6.2 | DONE | 实现隔离的 Browser Resource 配置目录 | macOS 重复调试已验证同一资源复用持久配置；不同资源目录隔离，临时配置可删除 |
| M6.3 | DONE | 实现 Session 级 Browser Resource 管理 | 浏览器不进入分割树；支持创建、启动、聚焦、重启、停止和删除 |
| M6.4 | DONE | 保持浏览器资源本地化并分离远程服务转发 | 新资源不绑定 SSH；远程服务使用独立隧道流程 |
| M6.5 | DONE | 用原生 `agent-browser` 替换自研浏览器工具原型 | Windows 和 macOS 的真实 Agent 任务已覆盖页面复用、交互、等待、标签页、控制台、网络、HAR 和截图，并复用受管 Chrome |
| M6.6 | DONE | 增加外部 Chrome 交互 | Windows 与 macOS 均可启动并聚焦真实 Chrome，用户可直接接管交互 |
| M6.7 | DONE | 增加 Session 感知的原生 Browser MCP 包装器 | `luna-mux mcp browser` 启动校验过的 sidecar，不依赖 Node、`npx` 或公开 CDP 端口 |
| M6.8 | DONE | 为普通 Agent 进程配置 `agent-browser` MCP | macOS 本地 Agent、真实 SSH 远程 Agent、WSL 与 Claude Code 的浏览器任务均已通过；Codex/Claude shim 使用同一路由 |
| M6.9 | DONE | 停用 Agent 可见的自研 `browser.*` 方法 | Luna 控制层仅保留内部能力，Agent 只看到统一浏览器 MCP |
| M6.10 | DONE | 强制首版每 Session 只运行一个 Chrome | 界面和包装器都阻止同 Session 多 Runtime；macOS 调试日志确认工具调用复用同一受管 Chrome 与默认页面 |

## M7——持续兼容

| 编号 | 状态 | 任务 | 验收条件 |
| --- | --- | --- | --- |
| M7.1 | TODO | 建立 Windows/macOS 持续集成检查 | 两个平台执行类型检查、Rust 测试、构建、产品元数据和多语言检查 |
| M7.2 | TODO | 增加终端一致性场景 | PowerShell、WSL、macOS Shell 和 SSH 共用行为测试 |
| M7.3 | TODO | 按需参考 Luna Remote 相关能力 | 由 AI 阅读参考代码并重新实现，记录需求、设计差异和验证证据，不建立 Git 同步关系 |
| M7.4 | TODO | 执行里程碑回归门禁 | 只有适用的 Windows 和 macOS 验收全部通过才算完成 |
