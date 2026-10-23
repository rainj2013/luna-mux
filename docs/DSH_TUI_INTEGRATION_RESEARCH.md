# DeepSeek Harness / dsh-TUI 接入调研

调研日期：2026-10-23。以下保留初始调研结论，基于上游文档和源码、npm 发布元数据以及当时的 Luna Mux 实现。后续已完成首版接入和隔离上游验证。未调用模型，尚未完成真实窗格及跨平台端到端验收。

## 结论

**可以适配。** dsh-TUI 输出 ANSI 到真实 TTY，适合继续使用 Luna Mux 的现有 PTY + xterm.js 窗格。要达到 Codex / Claude Code 的托管体验，需要新增 Agent adapter、启动配置注入及状态接入，不能只增加一个启动按钮。

建议分两步：先以 `dsh-tui --backend dsh` 为明确的接入对象，复用其 OSC 7501 状态报告与官方 MCP client；需要精确会话身份、后台任务控制和完整生命周期时，再增加一个小型 Cordis 桥接插件。无需 fork dsh 或 dsh-TUI，也无需重写其界面。

## 调研基线

| 项目 | 核实结果 |
| --- | --- |
| 官方 dsh | [deepseek-ai/deepseek-harness](https://github.com/deepseek-ai/deepseek-harness)，MIT，开发预览阶段；官方明确提示存在破坏性变更 |
| dsh 源码基线 | `687f282e8625c6b8d11ff58211a90524be2e4317`，根包版本与 npm latest 均为 `0.2.1-rc.2` |
| 社区 TUI | [ccch1mneyyy/dsh-TUI](https://github.com/ccch1mneyyy/dsh-TUI)，MIT，包名 `@deepseek-harness-tui/dsh-tui` |
| TUI 源码基线 | `3ffbf24628fc850e5dd3406798e34ca6ee16b924`，源码与 npm latest 均为 `0.16.1` |
| 环境要求 | 两者当前源码要求 Node `^22.19 || >=24`；Luna Mux 仓库使用 Node 24，但用户目标终端的 Node 需要独立探测 |

dsh 是插件化 Agent harness；dsh-TUI 是其终端前端，当前也支持 Claude SDK、Codex app-server 和扩展后端。TUI 正从挂载插件向自有入口及组合根迁移，不能假设启动器始终只是 `dsh --profile dsh-tui` 的别名。

## 与现有 Luna Mux 的对照

| 能力 | 现有基础与接入判断 |
| --- | --- |
| 启动、分屏、缩放、重连 | 复用现有 Runtime 和 PTY。新增 `dsh-tui` adapter/profile、目标环境命令探测；远端需在相应环境安装 |
| 手动命令识别 | 复用 runtime shim 机制。优先识别 `dsh-tui`，再覆盖 `dst`；不能把 `dst run` 等非交互子命令当作 TUI |
| 工作中、权限等待、问答等待、完成、错误 | TUI 已输出 OSC 7501。Luna Mux 当前仅把包含 OSC 的输出识别为泛化 `TerminalOsc`，没有解析其状态 |
| Luna Mux MCP | dsh 官方 client 支持 stdio 和 Streamable HTTP、自定义 headers、显式子进程 env。可复用现有 endpoint、授权和远端代理 |
| Browser MCP | 可作为独立 MCP client 配置注入，但还需复用 Luna Mux 的 Browser Resource 路由约束和凭据代理，单纯连上 MCP 不代表行为已对齐 |
| 发送任务 | 现有 `agents.send_task` 写入 `task + CR`。TUI 有输入框，可作为首版基础；必须验证多行、粘贴、输入法，以及权限弹层和启动页出现时的行为 |
| 中断 | TUI 的 Ctrl+C 支持工作时中断；连续中断可能强制退出。必须验证现有 Runtime interrupt 在各平台的实际效果 |
| 会话恢复 | 启动器支持 `--resume <id>`；需拿到当前实例的准确 session id，不能使用全局“上次会话”文件推断多窗格归属 |
| 图片与终端通知 | TUI 支持 Kitty/Sixel、文本回退和多种通知。Luna Mux 当前 TerminalPane 未挂载图片 addon，不能承诺图片预览；建议通知最终由 Luna Mux 统一，避免重复 |

主要本地入口：

- `app/native/src/agent_adapters.rs`：adapter 注册、托管启动、手动 shim、远端能力声明。
- `app/native/src/agent_profiles.rs`、`agent_command.rs`：profile 和目标 shell 内命令探测。
- `app/native/src/commands.rs`：启动编排及上下文注入。
- `app/native/src/agent_hooks.rs`：现有 Agent 身份、事件、状态和终端输出信号。
- `app/native/src/control_service.rs`：`agents.send_task`、`agents.interrupt`。
- `app/native/src/agent_session_title.rs`：当前 Codex / Claude 专用的标题提取。
- `app/frontend/src/components/TerminalPane.tsx`、`terminal-input.ts`、`App.tsx`：终端交互、adapter 映射和名称显示。

## 最有价值的现成接口：OSC 7501

TUI 默认向 TTY 发送程序状态；`DSH_TUI_PROGRAM_STATUS=0` 可关闭。消息样例：

```text
ESC ] 7501 ; state=working:app=dsh-tui ESC \
ESC ] 7501 ; state=blocked:app=dsh-tui:kind=permission:msg=<base64> ESC \
```

| TUI 状态 | Luna Mux 建议映射 |
| --- | --- |
| `working` | Working |
| `blocked` + `permission` | Waiting / Permission |
| `blocked` + `question` | Waiting / Input |
| `blocked` + `auth` | Waiting / External，并保留登录原因 |
| `done` | Completed |
| `error` | Error |
| `idle` | Waiting / Input；不能映射为任务成功 |

实现建议：在原生 Runtime 输出消费路径做有界、增量解析，再通过现有 Agent 状态服务发布；避免依赖当前是否挂载前端 TerminalPane。处理跨 chunk 的 ESC/ST、同一 chunk 多条序列、非法 base64、超长内容及重复状态，不能按每块字符串正则匹配。

重要边界：

- 当前 TUI 的报告携带 app、title 和阻塞原因，**没有发送其 session id**。协议支持的 `id` 是状态记录地址，也不能直接当作 dsh session id。
- 输出描述前台绑定会话，不能据此把所有 dsh 子 Agent 注册成 Luna Mux Agent。
- OSC 是终端内容，子进程也能输出。应绑定已知 Runtime/托管进程，只用作状态证据；不得据此授予权限、改变凭据或认定为已认证 Hook。
- 当前 `ManagedAgentEvidence` 只有 `StructuredHook` 和 `TerminalHeuristic`。采用 OSC 时要明确证据分类及优先级，不能冒充现有结构化 Hook。
- TUI 源码已区分恢复历史与本轮结束，取消后报告 idle；Luna Mux 必须保留这些语义，避免恢复会话即发完成通知。
- 退出仍需由进程包装器或生命周期桥接报告；不能假定 TUI 一定会发送最终清理序列。

## 启动与 MCP 注入

建议 adapter id 为 `dsh-tui`，内置命令为 `dsh-tui`，托管时显式选择 `--backend dsh`。否则 TUI 会使用用户记住的后端，Luna Mux 可能标记 dsh 却实际运行 Codex。

官方安装路径是全局安装 `@deepseek-ai/dsh` 与 `@deepseek-harness-tui/dsh-tui`；TUI 首次启动会初始化 profile，需要 pnpm。安装/初始化与日常启动应区分，不能把首次安装进度误判为 Agent 已就绪。调研后已完成首版接入，并在隔离目录安装固定版本进行配置和 MCP 验证。

启动器支持前置 `--patch <file>`，追加运行期 overlay；当前源码中，存在这类 host 参数时会走 `dsh --profile` 路径，因此需要同时验证普通自有入口和带 patch 的启动路径。

建议每个 Runtime 生成临时 overlay，加入 `@deepseek-ai/dsh-mcp-client` 实例，并按目标使用：

- 本地：Streamable HTTP + 现有授权 headers，或现有 stdio bridge。
- WSL / SSH：复用现有 remote-agent、路径转换和代理，不能直接假设远端 `127.0.0.1` 指向桌面端。
- Browser：复用认证后的 Browser MCP 代理及路由指令，不把浏览器 CDP 端口暴露给 dsh。

保留用户 profile 与权限策略，不全局写入 `~/.dsh` 配置，也不为接入而改成无限制权限。密钥通过环境或现有凭据文件读取，不能落入启动命令、日志或可读配置。**dsh stdio client 会清洗父环境中的敏感名字和 `DSH_*`，所以 bridge 所需 env 必须显式配置**，不能只指望继承。

注意 Cordis patch 覆盖已有行时会替换整段 config；新增唯一 id 的 MCP/桥接行比局部覆盖用户配置更容易控制。还需验证各 preset 下工具可见性、初始化失败和同名 server 的处理。

## 完整接入：增加一个薄桥接插件

OSC 适合快速获得前台状态。更完整的托管可增加 Luna Mux 自有 Cordis 插件，通过官方服务/事件转发带 Runtime 授权的结构化消息：

- 从 `session/event` 的 `turn/start`、`turn/end` 取得 session、turn 和结束原因。
- 从 `approval/asked`、`approval/decided` 记录真实权限等待；不接管 TUI 的审批决策。
- 从官方问答、会话创建/销毁和标题事件补齐身份、待输入、恢复与生命周期；具体事件需实现时按锁定版本再次核实。
- 桥接只保留连接、订阅和最小映射，不建立第二份会话日志。dsh 会话由 dsh 管理；Luna Mux Runtime 和 Agent 快照仍由现有服务管理。
- 断连、重绑、新会话、应用退出时清理订阅并防止旧会话事件污染当前 Agent。投递失败不能阻塞 TUI 或自动同意审批。

**不建议直接复用 Claude adapter。** dsh 官方虽然有 `@deepseek-ai/dsh-hooks-claude-code`，但当前只实现 command hook 子集；HTTP hook 被跳过，且没有完整的 PermissionRequest / SessionEnd 等事件。其 Stop 位于 `agent/turn-stopping`，其他 hook 还可能要求继续，不能当作最终完成的完整证明。该包位于 experimental，适合验证原型，不宜作为完整托管的唯一基础。

会话标题/历史优先通过公开 API 或事件获取。官方 Hook 文档明确 `transcript_path` 当前为空且默认日志可能压缩，不能直接套用 Luna Mux 的 Claude JSONL 标题读取器。

## 建议交付顺序与验收

1. **最小运行验证**：在真实 Luna Mux 窗格启动固定版本 TUI，验证 ANSI、鼠标、焦点、窄分屏、中文输入、粘贴、缩放和关闭。验证前不宣称已兼容。
2. **首版托管**：新增 adapter/profile、进程启动/退出包装、OSC 状态解析、Runtime overlay 和 Luna Mux/Browser MCP。覆盖手动启动与产品入口启动，先限制为 dsh 后端。
3. **补齐深度体验**：按需求加入薄插件，完成准确 session/turn、标题、显式恢复、问答原因和更可靠的任务控制。再评估 `/kernel` 切换等多后端模式。

关键验收：

- 两个窗格同时运行，状态、授权与任务互不串线；`--resume <id>` 只恢复目标会话。
- 主任务有子 Agent 时，子任务结束不会误报主任务完成；取消和错误分别显示。
- 权限/问答弹层存在时，发任务不能被当成审批回答；多行任务只提交一次。Ctrl+C 中断不误杀整个 TUI。
- MCP 实际列出并调用工具；Browser 请求绑定当前 Session 的资源；断开后错误可见。
- 程序异常退出、TUI `/update` 重启及切换会话后，不留下永久 Working 或重复完成通知。
- 分别验证 PowerShell 5.1、PowerShell 7、WSL、macOS、SSH；远端覆盖断线和代理生命周期。

正式实现属于 L3：遵守 `AI_DEVELOPMENT_GUIDELINES.md` 的架构验收、完整检查、相关原生测试、平台实机验证和独立 Review。当前调研没有实机运行证据，尤其不能据源码支持 Windows 就宣称 PowerShell/ConPTY 已通过。

## 主要上游证据（固定到调研提交）

- [官方项目定位、开发预览与许可](https://github.com/deepseek-ai/deepseek-harness/blob/687f282e8625c6b8d11ff58211a90524be2e4317/README.md)
- [官方 MCP client 配置与环境清洗](https://github.com/deepseek-ai/deepseek-harness/blob/687f282e8625c6b8d11ff58211a90524be2e4317/packages/mcp/mcp-client/README.md)
- [官方会话事件类型](https://github.com/deepseek-ai/deepseek-harness/blob/687f282e8625c6b8d11ff58211a90524be2e4317/packages/core/session/src/types.ts)
- [官方权限请求与审计事件](https://github.com/deepseek-ai/deepseek-harness/blob/687f282e8625c6b8d11ff58211a90524be2e4317/packages/interaction/user-approval/src/types.ts)
- [Claude Hook 桥接范围与限制](https://github.com/deepseek-ai/deepseek-harness/blob/687f282e8625c6b8d11ff58211a90524be2e4317/packages/experimental/hooks-claude-code/README.md)
- [TUI 安装与终端集成](https://github.com/ccch1mneyyy/dsh-TUI/blob/3ffbf24628fc850e5dd3406798e34ca6ee16b924/docs/getting-started.en.md)
- [TUI 启动器、恢复、后端及 patch 分流](https://github.com/ccch1mneyyy/dsh-TUI/blob/3ffbf24628fc850e5dd3406798e34ca6ee16b924/bin/dsh-tui.js)
- [TUI OSC 状态生成与历史重放边界](https://github.com/ccch1mneyyy/dsh-TUI/blob/3ffbf24628fc850e5dd3406798e34ca6ee16b924/src/ink/hooks/use-program-status.ts)
- [TUI OSC 协议编码与限制](https://github.com/ccch1mneyyy/dsh-TUI/blob/3ffbf24628fc850e5dd3406798e34ca6ee16b924/src/ink/termio/osc7501.ts)
- [TUI 架构与权限边界](https://github.com/ccch1mneyyy/dsh-TUI/blob/3ffbf24628fc850e5dd3406798e34ca6ee16b924/docs/architecture.en.md)
