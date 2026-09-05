# 跨窗格终端交互执行计划

## 范围与架构验收

本次实施第一期通用交互闭环（L3），适用于现有 Shell、SSH、MySQL、Redis 等 PTY 程序。浏览器不变。用户已授权按计划实施，不另设审批步骤；已有关闭、传输、隧道审批不变。

- 调用链：认证 MCP / Control API → InProcessControlService → TerminalInteractionManager → 现有 TerminalBackend → local PTY / SSH。桌面终端输入通过同一个输入协调入口。
- 唯一数据归属：Runtime 后端继续拥有输出环、游标和生命周期。控制服务持有一个交互管理器，只存执行元数据、去重摘要、等待进度和输入协调锁，不复制终端输出历史，不持久化命令正文。
- 兼容：保留旧读写接口及 JSON；以新增 Control operations 扩展 v2 目录，同步 Rust descriptor、MCP schema、JSON contract 和测试。不修改数据库、终端 Runtime JSON 或前端状态模型。
- 平台：发送原始 PTY 输入，不追加 Shell 语法、不解析本地命令、不要求 stdin EOF。Enter 使用 CR；粘贴换行统一为 CR，bracketed paste 必须显式启用，不能假定目标程序已开启。Ctrl-C 使用既有 interrupt 入口。
- 权限：所有新操作以 TerminalRuntime 为资源，经既有授权检查；执行 ID 不能替代 Runtime 授权，执行记录同时校验创建者。桌面用户输入优先接管；同 Runtime 的 Agent 输入遇到未结束交互返回 conflict，不静默排队执行过时命令。

## 本次实施步骤

1. 新增类型化输入和等待参数，严格验证未知字段、长度、超时和输出上限。输入支持 text + submit、命名按键、显式粘贴；等待支持字面量 contains / suffix、静默、退出和超时。默认不推断 CLI 类型或命令成功。
2. 实现有界输出等待：复用后端游标读取，最多等待 30 秒，单次最多返回 1 MiB；处理跨块匹配、UTF-8、截断、输出额度耗尽、Runtime 退出。静默结果与匹配、超时明确区分。通过有界异步轮询兼容现有后端，无新增事件流或 Shell 集成。
3. 实现执行记录和去重：interact 要求稳定 idempotencyKey；写入前建立记录并捕获输出末尾游标。写入任务独立于 MCP 调用存活，重试不重新输入；同键不同输入/参数冲突。只保存请求 SHA-256 摘要。记录在本次应用运行期间保留，达到 1024 条时拒绝新执行，绝不静默淘汰去重记录后重发命令。
4. 暴露 terminal.runtime.interact、terminal.runtime.execution.read / wait / cancel、terminal.runtime.output.wait。返回 executionId、输入状态、观察结果、输出游标和截断信息。等待超时或额度耗尽保留执行，可继续等待；cancel 只结束观察并释放交互占用，绝不发送中断。中断仍使用已有操作，结果不宣称目标已停止。
5. 接入输入协调（含审查发现的背压边界）：本地 PTY 阻塞写入移至 blocking pool，写入不持有 Runtime 表锁；用真实 raw-mode PTY 停止读取输入验证异步等待可响应。Agent 新交互及旧 write / send_task 共用 Runtime 输入锁；桌面输入、中断结束先前交互的观察，并与输入写入串行。等待匹配或静默仅表示满足调用者条件，释放占用不证明 Shell 已就绪。
6. 同步工具说明、契约和设计文档；增加管理器、Control 权限/审计/契约、MCP schema 和本地 PTY 场景测试。
7. 执行 npm run check、npm test、npm run runtime:test、npm run native-core:test。按开发规范使用独立上下文审查差异，修复有证据的问题并复测。记录未验证的平台和限制。

## 验收矩阵

| 场景 | 预期 |
| --- | --- |
| 命令写入、快速输出 | 从写入前游标读取，历史输出不触发匹配 |
| 重试 / 并发重试 / 调用取消 | 一个执行 ID，PTY 最多写入一次 |
| 同键不同参数 | conflict，无新写入 |
| 超时后续等 | 原执行继续，绝不自动重发 |
| 超长输出 / 环覆盖 / UTF-8 / 跨块匹配 | 有界响应、正确游标、截断显式可见 |
| 静默 / 提示符文本 | 仅报告 idle / matched，无 fabricated exitCode 或成功状态 |
| 取消等待 | 不发送任何 PTY 字节；重复取消幂等 |
| 写入失败 | 保留不可自动重试的记录，可能部分写入标记为不确定 |
| 并发 Agent / 桌面接管 | Agent 冲突；用户接管使等待返回 inputChanged |
| Runtime 退出 / 不存在 | 明确返回生命周期结果或结构化错误 |
| 越权 / 执行 ID 指向其他 Runtime 或创建者 | 拒绝且无副作用 |
| 审计 | 不持久化输入正文、匹配文字或去重键 |

## 后续路线图（不在本次实现范围）

第二期：Runtime 终端解析器与屏幕快照、模式感知按键和粘贴、可配置 CLI 提示规则。实施及验证见 [二期计划](terminal-screen-interactions.md)。

第三期：可选 Shell integration 提供命令开始/结束、cwd 和可靠退出码；据此增加条件执行。需逐一实现 macOS、PowerShell 5.1/7、WSL 和 SSH，不能向任意 CLI 注入 Shell 标记。

第四期：按用户配置启用受限输出落盘、搜索和过期清理。单独定义敏感数据、磁盘限额及去重记录保留策略。

## 验证结果

本次实施步骤 1–7 已完成。2026-09-05 验证结果：

| 检查 | 结果 |
| --- | --- |
| `npm run check` | 通过，包含 product/runtime/icons/i18n 检查、8 项终端输入测试、TypeScript typecheck 和 native cargo check |
| `npm test` | 197 passed，0 failed，4 ignored；忽略项目保留仓库原有配置 |
| `npm run runtime:test` | 8 passed |
| `npm run native-core:test` | 12 passed |
| `git diff --check` | 通过 |
| macOS 真实 PTY | 超时续等、重复请求不重发、raw-mode 程序暂停读取 64 KiB 输入时等待定时器仍可响应，最终读取成功 |
| 独立上下文审查 | 已完成；发现项均修复，并添加对应回归测试 |

审查修复包括：自动输入使用非阻塞占锁，禁止排队执行过时输入；后缀匹配必须读至当时输出末尾；本地阻塞写入不占用 async worker 或 Runtime 表锁；旧写入、桌面输入和中断的协调锁由独立任务持有至底层完成，防止调用取消提前释放锁。额外覆盖 1024 条记录容量耗尽、去重保留及异步 waiter 取消后的恢复。

原生检查仍有既有未使用代码警告和 russh future-incompatibility 提示，没有为了消除警告修改无关模块。

未覆盖平台：Windows PowerShell 5.1、PowerShell 7、WSL 和真实 SSH/数据库 CLI 实机。本次复用现有后端与 CR 输入协议，未假设 POSIX quoting、EOF 或本地信号适用于远端；但 Windows PTY 背压/关闭和 SSH 写入/断连的实机行为仍需验证，不能由 macOS 测试推定通过。此处为一期验证时的范围；二期新增屏幕解析、模式感知输入和配置式 CLI 观察，见二期记录。一期显式 paste 接口仍由调用者指定目标模式。

## 调用示例

先从当前 Session 的 Pane/Runtime 列表确定目标 `resourceId`。以下为 MCP 工具 `terminal.runtime.interact` 的输入：

```json
{
  "resourceId": "<runtime-id>",
  "idempotencyKey": "query-001",
  "arguments": {
    "input": { "type": "text", "text": "SELECT 1;", "submit": true },
    "wait": { "text": "mysql> ", "matchMode": "suffix" },
    "timeoutMs": 1000,
    "maxBytes": 65536
  }
}
```

读取返回的 `executionId`。若 `observation.reason` 为 `timeout` / `outputLimit` / `outputGap`，使用 `terminal.runtime.execution.wait` 续等：

```json
{
  "resourceId": "<runtime-id>",
  "arguments": { "executionId": "<execution-id>", "timeoutMs": 10000 }
}
```

响应丢失时可用原幂等键和相同参数恢复执行 ID，再用 `execution.read` 重读；不能换键重发同一 SQL。并发 waiter 返回 conflict 时，错误 details 包含 executionId，可先 read；不要创建新的输入请求。需要交还终端时调用 `execution.cancel`（同一 Runtime + executionId），随后检查终端现场；只有明确需要中断前台任务时才调用 `terminal.runtime.interrupt`。

执行记录和输出均为内存数据：应用重启后去重保证不延续，输出也可能早于执行记录被覆盖。记录达到 1024 条时返回 unavailable；当前版本没有自动淘汰或持久化恢复。
