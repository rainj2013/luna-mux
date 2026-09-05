# 二期：终端屏幕与模式感知交互

## 架构与范围（L3）

用户已授权实施二期。延续一期唯一控制入口及同 Session 权限：MCP → ControlService → InteractionManager / TerminalBackend → Runtime OutputBuffer。每个 OutputBuffer 在截断原始输出前增量维护一个终端解析器；屏幕为输出派生状态，不依赖 React / xterm 挂载，不持久化第二份历史。前端的 xterm 仍负责显示，不向后端回传快照。

使用固定版本 vt100 解析器处理 ANSI、光标、宽字符和备用屏幕；[上游接口说明](https://docs.rs/vt100/0.16.2/vt100/struct.Parser.html)。屏幕模型与输出在同一锁内更新，快照带输出游标。一个仅记录打印/编辑活动的 vte 扫描器维护每行写入游标，不保存第二份屏幕。UTF-8 C1 先规范化为七位序列，OSC/DCS/SOS/PM/APC 字符串在进入解析器前有界丢弃。限制解析尺寸和快照字节数；超限明确报告不完整，不把截断/不完整屏幕用于自动提示判断。

新增 screen.snapshot 控制操作和生成的 Runtime DTO；Backend 新方法提供默认 unsupported，旧扩展实现仍可编译。更新 Control JSON、MCP schema、Runtime JSON / 生成脚本及 TypeScript/Rust 消费者。无数据库或前端 UI 迁移。

按键支持方向键、Home/End、翻页、Delete，并根据 application cursor 模式编码。新增 autoPaste 输入，根据 bracketed paste 模式编码；多行且模式未开启时拒绝自动粘贴，避免逐行意外执行。一期 paste 的显式布尔参数保持不变。模式在获得输入协调锁后读取；重试先查去重记录，不因模式变化重发或重编码已发送的请求。

CLI 规则为调用参数，不建立持久化配置表或全局进程识别器。提供 mysql / redis / pager / custom 预设选择及受限自定义规则，输出 prompt / continuation / pager / unknown、规则来源。识别当前光标行，基于显式配置作观察，不推断命令成功、退出码或 Shell 已就绪。支持 wait.prompt，使 Agent 可等待屏幕上的提示状态；记录光标行实际写入/内容变更的 UTF-8 游标（cursorLineCursor），仅匹配起始游标之后更新的光标行；不使用整屏 hash，避免同样提示重绘漏报或其他行刷新误报。

## 实施步骤与验收

1. 接入有界 vt100 屏幕模型及生成 DTO；覆盖覆盖写、清屏、跨块 ANSI、宽字符、光标、主/备用屏幕、模式开关、尺寸变化、输出环覆盖和容量限制。
2. 本地/SSH 后端创建时同步尺寸、成功 resize 后更新模型，Composite 统一转发。原始输出字节和事件不变；解析器不得回应终端查询、执行 OSC 副作用或泄露标题/链接/剪贴板内容。
3. 新增 screen.snapshot，返回行文本、光标、终端模式、游标、完整性和可选 CLI 观察；限制返回量，校验授权与 schema。
4. 扩展 interact 语义输入与屏幕提示等待；保持一期 JSON 兼容、去重、取消、用户接管和输入锁行为。验证模式切换、自动多行粘贴拒绝、提示覆盖规则、分页/续行、旧提示符及模式变化后的重复请求。
5. 更新中英文设计与工具用法，运行窄测试、真实 macOS PTY 及全量 check/test/runtime/native-core。按开发规范完成独立上下文审查并修复有证据的问题。

## 平台与非目标

处理现有 Runtime 的 UTF-8 输出，所有平台复用同一解析器。输入只编码终端序列，不追加 Shell 语法，不依赖 stdin EOF。macOS 本地实测；PowerShell 5.1/7、WSL、SSH 未提供实机环境时明确记录风险。

不包含第三期 Shell integration、可靠退出码和条件命令执行；不实现浏览器、图像/像素快照、终端查询应答、完整 xterm 图形协议或数据库连接器。屏幕仅为终端文本模型，可能与 xterm 在复杂字形和 resize 重排细节上不同。

## 验证结果

2026-09-05，步骤 1–5 已完成。

| 验证 | 结果 |
| --- | --- |
| `npm run check` | 通过，含生成契约校验、TypeScript typecheck、终端输入测试及原生 cargo check |
| `npm test` | 217 passed，0 failed，4 ignored；原有需浏览器环境的测试保持忽略 |
| `npm run runtime:test` | 9 passed，含新增屏幕 DTO camelCase/round-trip 测试 |
| `npm run native-core:test` | 12 passed |
| `git diff --check` | 通过 |
| 真实 macOS PTY | resize、主/备用屏幕切换、application cursor 和 bracketed paste 模式；前台 raw-mode 程序实际收到 ArrowUp 的 SS3 字节和 15 字节自动括号粘贴，最终回到主屏幕 |
| 8 KiB 连续 LF 压力回归 | 本机 debug 构建约 82 ms（24×80）、280 ms（256×512）；行比较有固定单元访问预算，耗尽后清除未知新旧标记，新打印可恢复 |

回归覆盖：ANSI 覆盖/清屏/跨块、宽字符与一列网格、C1 控制字符串与 ESC 打断、环覆盖后屏幕保留、相同提示重绘/其他行更新/OSC-only 更新、U+FFFD 与零宽字符、提示正常/续行/分页及规则覆盖、模式切换后幂等重试、自动多行粘贴拒绝、原输入协调/取消/背压、授权和审计脱敏。

独立上下文审查发现的提示新旧判断、控制字符串处理、CPU 放大、宽/零宽字符边界均已修复并补回归；最终复审确认审查范围内没有剩余具体阻断项。未覆盖 PowerShell 5.1/7、WSL 和真实 SSH/MySQL/Redis CLI 实机；本机没有这些 CLI 可执行文件。预设规则使用输出 fixture 验证，不能视为真实服务器端到端测试。SSH resize 在途输出时序及复杂 xterm 扩展兼容仍保留为已知风险。原生构建保留已有未使用代码与 russh future-incompatibility 提示。

## 使用示例

重新构建并重启应用后，Agent 可通过以下 MCP 工具使用二期能力；本期没有新增 GUI 按钮。

读取屏幕并按 MySQL 规则观察当前提示（`terminal.runtime.screen.snapshot`）：

```json
{
  "resourceId": "<runtime-id>",
  "arguments": { "cli": { "profile": "mysql" }, "maxBytes": 65536 }
}
```

返回 `screen.lines`、`cursorRow` / `cursorCol`、`cursorLine`、`cursorLineCursor`、`modes` 等，以及 `cli.state`、规则 `source` 和 `matchedRule`。识别规则由调用者选择，不自动探测进程。mysql 预设区分正常提示和 `->`、引号、注释续行；redis 预设覆盖地址/端口、库编号与未连接提示；pager 预设覆盖常见 `--More--`、`(END)` 和 `:`。不同工具版本或自定义提示可覆盖规则。

输入并等待正常/续行提示（`terminal.runtime.interact`）：

```json
{
  "resourceId": "<runtime-id>",
  "idempotencyKey": "query-screen-001",
  "arguments": {
    "input": { "type": "text", "text": "SELECT 1", "submit": true },
    "wait": {
      "prompt": { "cli": { "profile": "mysql" }, "states": ["prompt", "continuation"] }
    },
    "timeoutMs": 1000
  }
}
```

`observation.reason = prompt` 时，查看 `observation.prompt.state` 区分正常提示与续行；执行语义仍以程序输出为准。超时沿用一期 execution.wait 继续等待。

模式感知的输入示例（放入 interact 的 `arguments.input`）：

```json
{ "type": "key", "key": "ArrowDown" }
```

```json
{ "type": "autoPaste", "text": "SELECT\n  1;" }
```

自动多行粘贴只有 bracketed paste 模式开启才会发送，不自动附加 Enter。显式 `paste` 仍保留一期行为。Home/End 和方向键自动选择 CSI/SS3；PageUp/PageDown、Insert/Delete 可直接命名。Ctrl-C 使用原 interrupt。

自定义提示规则示例：

```json
{
  "profile": "custom",
  "rules": [
    { "state": "continuation", "pattern": "\\s*more>\\s*" },
    { "state": "prompt", "pattern": "db\\[[0-9]+\\]>\\s*" }
  ]
}
```

规则整体匹配完整光标行，不包含 ANSI，末尾空白单元已裁掉。`rules` 替换预设而非追加，第一条匹配规则生效。规则正文不写入审计日志。

## 有界性及已知限制

- 模型最多 256×512 单元、零 scrollback；超限留下 `sizeLimited`，不会把受损屏幕当成可信提示。
- vt100 的宽字符运算要求内部至少两列；真实一列终端不被改变，但模型使用两列并置 `sizeLimited`，禁用基于该模型的自动提示判断。替换字符及没有实际改动单元的零宽字符不会更新提示新旧标记。
- 输出快照限制的是行文本和 cursorLine 的 UTF-8 字节，不含 JSON 元数据开销；`truncated` 时不作提示识别。
- 每个输出块的行变化比较最多访问 262144 个单元。超出预算仍正常渲染，但无法证明新旧关系的行游标清零；`cursorLineCursor = 0` 表示未追踪/未知，不能满足新的 prompt 等待。后续实际打印会恢复该行追踪；可继续读取原始输出，不会因性能降级误把旧提示当成新提示。
- 快照为文本模型，复杂字形、部分 xterm 扩展序列和 resize 重排并非像素级一致。没有远端原子屏幕检查点，SSH resize 与在途输出之间可能短暂错位，应等待重绘后重新读取。
- 光标坐标按解析器的零起点单元位置返回；`cursorCol == cols` 表示末列之后的待换行位置。
- 原始输出读取接口保持不变；快照不保存或执行 OSC 标题、剪贴板或链接目标，C1 控制字符串同样被处理。解析器不响应终端查询，避免与前端 xterm 重复应答。
