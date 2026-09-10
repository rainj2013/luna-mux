# 数据库窗格实施计划

## 启动入口与连接管理统一（2026-09-10）

- 边界：App 启动弹窗、数据库 React UI 和样式；连接仍由 databaseProfiles 保存，运行时由 databaseRuntimes 管理。不改 Rust、MCP 契约或持久化格式。
- 调用链：启动器读取已保存 Profile → 选择连接创建数据库窗格并传入初始 Profile → 现有连接 API。管理弹窗独立于窗格，不注册 Pane UI bridge，也不因打开管理界面创建 Runtime。
- 兼容：已有数据库窗格仍能选择连接；初始连接只作为新建窗格的前端启动参数，不保存凭据，不引入新的恢复/自动重连策略。
- [x] 分类标题使用 13px 半粗体、18px Lucide 图标、数量标记及右侧折叠箭头。
- [x] 数据库分类直接展示已保存连接，提供与 SSH 对齐的管理/空列表入口；加载失败显示错误并保留管理入口。
- [x] 独立连接管理弹窗与工作窗格分离；管理弹窗复用连接表单，支持分组和上下移排序。保存保留原列表位置。管理弹窗的只读选择传递到新窗格。
- [x] 连接失败可重试，重连读取最新 Profile；已删除连接不会创建 Runtime。取消密码输入不连接，保存过凭据的连接沿用原生凭据读取。React StrictMode 不重复建立连接，关闭窗格后到达的连接结果会被断开。
- [x] 验证：npm run check、npm test（234 passed / 5 ignored）、最终 typecheck、i18n:check（781 keys）、web:build 通过。38 项模拟断言通过，覆盖主流程、只读/更新/删除/重试/清理、密码取消、已保存凭据、空列表和列表加载失败；键盘 Enter 展开、仅右侧箭头旋转已检查。
- [x] 已检查实际组件的暗色折叠、暗色数据库展开、浅色 SSH 展开截图；截图包含测试数据，非真实数据库。临时 React 预览在词条热更新时出现过 I18nProvider 上下文错误，完整重载后主流程通过。
- [ ] 真实数据库认证、TLS、跨平台桌面连接流程仍由手工验收覆盖；不以模拟 API 检查替代。

## 桌面 UI 一致性修订（2026-09-10）

- [x] 新增窗格弹窗分类顺序调整为：本地终端（始终展开）、SSH 远程环境（默认折叠）、数据库连接（默认折叠）。折叠标题支持键盘展开，沿用应用主题与焦点样式；每次重新打开弹窗恢复默认折叠。类型及双语词条检查通过。
- [x] 清理数据库页叠加的 CSS 覆盖，只使用已有主题变量、系统字体、Lucide 图标及表单/按钮样式；入口复用窗格启动器的列表项。
- [x] 连接页改成连接列表与设置表单；数据库类型用单选控件展示。移除多余的选择连接下拉框、字符图标、装饰性英文标题和大卡片。
- [x] 未连接时只显示连接管理；连接后提供对象、SQL、表结构与设置；断开后返回连接页。连接期间设置锁定，避免修改表单却继续使用旧 Runtime。
- [x] 导入导出与新增字段/索引操作按需展开；查询结果采用真正的表格，表头与单元格对齐。修复保存连接未复用 ID、SQLite 测试连接调用错误接口的问题。测试连接复用现有 Runtime，以只读配置执行 SELECT 1，并在 finally 中断开。
- [x] 新增中英文 UI 文案；将桌面控件一致性和 UI 验收要求写入开发规范。
- [x] 验证：typecheck、i18n:check（771 keys）、web:build 通过。独立 React 组件网页预览使用模拟 API，18 项断言通过：未连接/保存/测试/临时 Runtime 断开/连接/表格结果/结构操作收起/已连接设置锁定/断开/编辑保留 ID/驱动默认端口/脏表单禁用，以及明暗主题在 1100、650、360 px 下无横向溢出。额外检查单选组方向键选择与可见焦点通过，浏览器错误列表为空。
- [ ] 截图视觉验收未完成：agent_browser 截图先返回连接读取错误（os error 10060），再次返回 `agent-browser command timed out after 10000ms`。以上 DOM、尺寸及交互检查不替代截图验收；未验证真实数据库认证、TLS 或系统密码对话框。

本轮范围为数据库前端交互与外观；不改 Rust/公共 API，不恢复 SSH 自动联动。历史任务状态保持独立，不能由本轮 UI 检查推导全部数据库能力已验收。

状态：进行中。历史进度记录不代表整体已验收；Agent 窗格操作正在按 UI 控件方案接入。

## 决策
- Agent 接入改为数据库窗格 UI 控件操作：snapshot / fill / click；使用当前 React 窗格状态和同一组人机操作处理函数，不直接调用数据库驱动或读取凭据。
- 权限复用 Control Service 的 Pane grants（由 Luna MCP 按 Session 分配）；仅已打开的 database Pane 可接收请求。控件使用明确的 ref，不支持任意脚本/选择器，也不提供密码或确认按钮的 ref。
- 调用链：MCP → ControlService（资源授权、参数校验、脱敏审计、幂等）→ 有界 UI 请求桥 → React 控件 → 现有 Tauri API。快照读取当前 UI，点击回传 accepted，执行状态和结果通过随后快照观察；accepted 不代表 SQL 成功。
- 兼容：新增操作和事件，保留既有 API；跨平台沿用现有 MCP 传输，不新增 shell、PTY 或浏览器生命周期。
- 验收：无权限/非数据库 Pane/未挂载 Pane 拒绝；未知、禁用、密码和确认控件拒绝；同一幂等键不重复点击；请求取消/超时/卸载清理；SQL 不进入审计；前端状态与按钮处理函数只有一个来源。
- 测试：桥接请求取消、超时、错误窗格回复；Control 授权/幂等/审计；前端控件输入/点击/禁用/确认隔离；npm run check、npm test、相关契约测试与独立 review。真实桌面交互由用户验收。
- 驱动：SQLite、MySQL/MariaDB、PostgreSQL；目标为全部使用内置 Rust 驱动，不捆绑服务端或命令行客户端。
- SQLite：打开本地文件、最近文件、只读模式、备份/另存为。
- 认证：密码 + TLS；密码仅存系统 Keychain。
- SSH：用户手动建立 SSH 端口转发后，数据库窗格连接本地端口。
- Agent：读操作直接执行；危险写操作确认。
- 结构：字段、索引、外键可视化修改；复杂对象先只读。
- 导入导出：CSV、JSON、SQL。
- 事务：查询自动提交，批量编辑手动事务。
- 每个数据库窗格独立连接。

## 任务
- [x] T1 扩展领域模型、SQLite 迁移、数据库 Profile 持久化
- [x] T2 引入数据库驱动抽象与运行时连接管理（SQLite 已可连接；网络协议驱动待接入）
- [x] T3 实现 Tauri commands、事件、前端 API 类型
- [x] T4 实现数据库 Pane UI（连接、对象树、SQL、结果）
- [x] T5 实现结构查看/修改、导入导出、手动 SSH 端口转发
- [x] T6 扩展 Control/MCP 数据库目录与 Agent 资源边界、审计脱敏
- [x] T7 测试、跨平台检查、文档收尾（真实 DB/SSH 环境除外）

## 进度记录

### T1（已完成）
- `MuxPaneKind` 新增 `Database`。
- 新增 `DatabaseDriver`、`DatabaseProfile`、`DatabaseProfileInput`。
- SQLite 初始化 `database_profiles` 与凭据引用表，并提供 Profile 列表、保存、删除持久化方法。
- 验证：数据库相关测试通过（11 个）。

### T2（已完成）
- 新增 `database_runtime` 模块，定义 SQLite/MySQL/MariaDB/PostgreSQL 驱动枚举、连接配置、Runtime 状态摘要及统一生命周期管理器。
- SQLite 使用内置 rusqlite 支持读写/只读本地文件；网络驱动已接入 Runtime，历史记录中的“驱动尚未接入”描述已被后续 T2 实现取代。
- Runtime 管理器支持 connect/list/disconnect/remove，错误状态可观测且不泄露凭据。
- 验证：`cargo check --manifest-path app/native/Cargo.toml` 通过。

### T3（已完成）
- `AppState` 初始化并持有 `DatabaseRuntimeManager`，应用退出时随生命周期清理。
- 注册数据库 Profile 与 Runtime 六个 Tauri 命令：列表、保存、删除、连接列表、连接、断开。
- 前端 `AppApi` 与 Tauri API 增加数据库 Profile/Runtime 类型和调用方法。
- 验证：`cargo check --manifest-path app/native/Cargo.toml` 通过。

## 当前实现边界
当前已具备 Profile/Runtime 的原生与前端 API 入口；数据库 Pane UI、查询和 MCP 能力仍由后续任务实现。

### T3（已完成）
- `AppState` 初始化并持有数据库 Runtime 管理器。
- 注册 Profile 列表/保存/删除、Runtime 列表/连接/断开、SQL 执行 Tauri commands。
- 前端 `types.ts` 与 `tauri-api.ts` 提供数据库 Profile、Runtime、连接配置和分页结果类型。
- SQL 执行强制非空 SQL，并限制默认返回 1000 行（后端上限 10000）；返回列名、行数据、影响行数和截断标记。
- 验证：`npm run typecheck`、`cargo check --manifest-path app/native/Cargo.toml` 通过。

### 后续任务状态
历史记录：此处描述的是早期阶段，已被下方 T4-T7 进度记录取代。

### T4（部分完成）
- 新增 `DatabasePane` React 组件：连接 Profile 选择、SQLite Runtime 连接/断开、SQL 编辑与执行、结果展示。
- MuxLayout 在 `kind=database` 时渲染数据库组件；普通 Terminal Pane 保持原流程。
- `MuxPaneKind` 前端类型扩展为 terminal/browser/database。
- 验证：`npm run typecheck`、`cargo check` 通过。
- 历史记录：Pane Launcher、Profile、对象树和导入导出已在后续任务补齐。

### T4（已完成 MVP）
- 数据库 Pane 支持 Profile 选择、SQLite 连接/断开、SQL 编辑执行、分页结果展示和表/视图列表。
- 点击对象树名称可生成基础查询语句。
- 验证：`npm run typecheck`、`cargo check --manifest-path app/native/Cargo.toml` 通过。

### 当前剩余实现
- 历史记录：网络驱动和导入导出已在后续任务实现；SSH 自动编排按用户决定取消。

### T5（SQLite MVP 已完成）
- 数据库 Pane 支持 Profile 新增/编辑/删除，保存 SQLite 文件路径。
- 支持表/视图枚举、表字段结构查看、增加字段（名称/类型白名单校验）。
- 支持 SQL 结果 CSV 导出。
- 验证：`npm run typecheck`、`cargo check --manifest-path app/native/Cargo.toml` 通过。

### 当前剩余
历史记录：网络 Runtime、JSON/SQL 导入、索引/外键查看及 UI 控制桥已在后续任务实现。

### T5（继续完成）
- 查询结果新增 JSON 导出，前端提供下载按钮；CSV/JSON 均复用后端行数上限和结果截断语义。
- 验证：`npm run typecheck`、`cargo check --manifest-path app/native/Cargo.toml` 通过。

### T6（基础契约完成）
- Control contract 新增 `databaseProfile`、`databaseRuntime` 资源类型，为 Agent 数据库权限隔离提供资源边界。
- 数据库命令仍通过 Tauri UI 入口；危险写操作确认策略和 MCP 工具目录尚待接入。
- 验证：`npm run typecheck` 通过。

### T6（继续完成）
- Control Service 新增 `databases.list` 只读操作，返回脱敏数据库 Profile 摘要。
- 数据库 Profile 使用独立 `DatabaseProfile` 资源授权边界。
- 验证：`cargo check`、`npm run typecheck` 通过。

### T5（继续完成）
- 新增 SQL 脚本导入：前端文件选择、后端事务执行、失败自动回滚，并刷新对象列表。
- 新增 `database_runtime_import_sql` 命令及前端 API。
- 验证：`npm run typecheck`、`cargo check` 通过。

### T4（继续完成）
- Pane Launcher 新增“打开数据库窗格”入口，可创建并持久化 `kind=database` Pane，加入当前 Session 布局。
- 验证：`npm run typecheck` 通过。

### 网络连接诊断
- 新增 `database_connection_test`：SQLite 检查文件存在；网络数据库以 5 秒超时测试 TCP 主机/端口可达性。
- 可用于诊断直连或 SSH 转发后的端口；不替代数据库认证/查询测试。
- 验证：`npm run typecheck`、`cargo check` 通过。

### 网络驱动连接测试（已完成）
- 新增内置 `mysql_async` 与 `tokio-postgres` 客户端连接测试；MySQL/MariaDB 执行 `SELECT 1`，PostgreSQL 执行 `SELECT 1`。
- 前端增加测试连接按钮，密码仅通过临时输入传递，不写入 Profile 或日志。
- 当前 Runtime 查询仍仅支持 SQLite；网络协议查询池接入是下一步。

### 网络 Runtime 查询（已完成基础版）
- Runtime 现在保存网络连接配置，并通过 `mysql_async` 执行 MySQL/MariaDB 查询，通过 `tokio-postgres` 执行 PostgreSQL 查询。
- 支持密码、端口、数据库名和 TLS（MySQL Rustls；PostgreSQL 系统根证书 Rustls）。
- 查询结果统一映射到列/行/截断结构，沿用 1-10000 行上限。
- 验证：`cargo check`、`npm run typecheck` 通过；未连接真实数据库，需手工验收认证、类型映射和 TLS。

### Profile 管理（已完成）
- 数据库连接支持完整排序校验、事务批量更新、分组移动。
- 新增 Tauri/API：`database_profiles_reorder`、`database_profiles_move_to_group`。
- 验证：`cargo check`、`npm run typecheck` 通过。

### Runtime 认证握手（已完成基础版）
- 创建 MySQL/MariaDB/PostgreSQL Runtime 前执行真实 `SELECT 1` 握手。
- 认证或网络失败时返回 Error，不保存假 Ready Runtime。
- 查询阶段复用已验证配置；密码只存在运行时内存。
- 验证：`cargo check` 通过。

### SQLite 结构修改（已完成基础版）
- 增加字段和删除字段均提供原生命令/API，字段名校验防止注入。
- 删除字段由调用方在高风险确认后执行；失败返回 SQLite 原生错误。
- 验证：`cargo check`、`npm run typecheck` 通过。

### MCP 数据库目录（已完成基础版）
- Luna MCP 工具目录和资源需求判断已加入 `databases.list`。
- Agent 可发现并读取数据库 Profile 脱敏摘要；密码不会通过 MCP 返回。
- 数据库 SQL、DDL 和导入导出通过 UI 控制桥间接操作；MCP 不直接获得驱动或凭据，写操作仍由桌面 UI 确认。

### 验证修复
- Control contract 已同步 `databases.list` 操作，目录契约测试恢复通过。
- `npm run check` 全部通过；完整 Rust 测试中的数据库与控制目录测试通过。

### 结构编辑 UI（已完成基础版）
- 选中表后展示字段、类型、主键和非空信息。
- 提供新增字段和删除字段按钮，危险操作二次确认并刷新结构。
- 验证：`npm run typecheck`、`cargo check` 通过。

### 索引管理（已完成基础版）
- SQLite 支持索引列表、单列索引创建（可选 UNIQUE）和删除。
- 索引/表/列名称统一白名单校验；新增和删除由 UI 高风险确认策略保护。
- 验证：`npm run typecheck`、`cargo check` 通过。

### 多数据库 Profile UI（已完成）
- Profile 编辑区支持 SQLite、MySQL/MariaDB、PostgreSQL 类型选择。
- 网络连接支持主机、端口、用户名；SQLite 支持文件路径。
- 选择已有 Profile 会回填配置，保存时复用原 Profile ID。
- 验证：`npm run typecheck`、`cargo check` 通过。

### 凭据管理（已完成基础版）
- 数据库 Profile 支持保存/删除系统 Keychain 凭据引用。
- 前端连接时可选择保存密码；密码仍仅在当前 Runtime 或系统凭据库中存在。
- SQLite 数据库只保存 profileId/account 引用，不保存密码明文。
- Runtime 连接可按 profileId 从系统 Keychain 自动读取已保存密码，前端不会回读密码。
- 验证：`cargo check`、`npm run typecheck` 通过。

### Runtime 自动化测试（已完成）
- 新增 SQLite Runtime 测试，覆盖查询结果上限/截断、SQL 导入事务回滚、成功导入和断开生命周期。
- 测试通过：`database_runtime::tests::sqlite_query_is_bounded_and_import_rolls_back`。

## 最终交付范围（2026-09-09）

- Pane、Profile、分组排序、独立 Runtime、Keychain 凭据引用和退出清理已接入。
- 内置 SQLite、MySQL/MariaDB、PostgreSQL Rust 驱动；支持密码、TLS、查询结果分页上限及错误状态。
- SQL 编辑执行、写操作确认、表/视图枚举、SQLite 字段与单列索引查看和修改已接入。
- CSV、JSON 导出和事务化 SQL 脚本导入已接入。
- Control/MCP 提供脱敏 `databases.list` 目录能力，密码不会返回或写入日志。
- Agent 可通过 `database.pane.snapshot`、`database.pane.fill`、`database.pane.click` 操作已挂载数据库窗格 UI；请求只允许显式控件 ref，不能读取凭据或调用任意脚本。
- 数据库窗格 UI 桥为有界请求（单 Pane 单并发、10 秒超时、64 个全局上限、512 KiB 响应上限），挂载 token 绑定 Pane，卸载和超时会清理请求。
- SSH 隧道不由数据库窗格自动管理；用户可通过现有 SSH 窗格/隧道功能建立端口转发，再将数据库主机设为本地地址。

未覆盖的高级能力包括 CSV/JSON 文件导入、外键及多列索引图形化编辑、视图/触发器/存储过程 DDL，以及真实 SSH/TLS 数据库端到端验证。这些属于后续增强项，不影响当前窗格的常见连接、查询、结构查看、导出和 SQL 导入流程。



## 2026-09-10 连接表单与卡死修复（本轮）

边界：L2，修改数据库窗格/新增窗格 UI、数据库 Tauri 命令和既有 Runtime manager。Profile 仍由应用数据库管理，密码只进入表单内存与系统凭据库，Runtime 仍由唯一 manager 管理。调用链：React 表单 → 异步 Tauri 命令 → blocking worker → Runtime/驱动 → 返回结果或有界超时错误；测试不创建持久 Runtime、不保存表单。保留现有 API 名称与配置格式，不涉及终端、PTY、SSH 隧道或 shell 行为。

验收与任务：
- [x] 分类保留图标和较大字重，列表条目去图标、缩进、小字号，明暗主题检查。
- [x] 表单内输入/替换/清除密码，保存使用系统凭据库，切换连接不串密码，不回读、不进入 Pane/MCP 快照。
- [x] 未保存且无名称时可测试有效端点；使用当前端口/用户名/TLS/密码，测试不保存、不创建文件或 Runtime；修改端点不得自动复用旧密码。
- [x] 网络操作移出 UI 线程，修复 MySQL 连接池关闭等待自身连接，连接测试 8 秒、查询 30 秒有界；查询不得持有全局 Runtime 锁等待网络。
- [x] 原生回归覆盖成功握手/查询清理、握手停滞/超时、并发目录响应和 SQLite 测试副作用；实际组件检查未保存测试、凭据保存/错误与重试。
- [x] npm run check、npm test、web:build 和独立 Review；真实远端数据库及 macOS 留待手工验收并记录。

本轮进度：实际组件浏览器检查 32 项通过（Mock API，真实 React 组件），包含未保存测试、密码不回读、修改端点禁用旧凭据、保存失败重试且不重复创建、清除失败不保存新端点、清空密码、切换驱动清理、无密码弹窗、工作窗格连接及卸载清理。明暗主题与 600px 窄布局已查看截图；npm run check、web:build 已通过。完整原生测试 242 passed / 5 ignored / 0 failed；本轮新增 8 项回归覆盖 MySQL/MariaDB 成功 SELECT 与关闭、握手超时、查询超时/并发目录不阻塞与关闭、PostgreSQL 查询超时关闭、SQLite 测试无副作用、PostgreSQL 驱动名兼容与密码不序列化、后台线程隔离、原生凭据目标校验。

独立 Review：依据开发规范第 6 节，使用独立上下文对比本轮基线审查。无新增阻塞问题；审查指出的原生凭据目标验证、PostgreSQL 查询超时关闭证据已补测试。真实操作系统凭据库读写、远端数据库/TLS 与 macOS 仍需手工验收，不将 Mock 结果当作真机成功证据。

修复原因：同步 Tauri 命令等待网络阻塞 UI；MySQL 在连接仍被借出时等待连接池关闭；网络查询持有 Runtime 全局锁。现改为后台 worker、单次直连及有界关闭、网络等待前释放全局锁。测试只验证当前配置，不登记 Runtime。PostgreSQL 同时接受前端 postgresql 和旧 postgres 驱动名，启用 TLS 时要求 TLS。

本地调试版已通过 cargo build --manifest-path app/native/Cargo.toml --bin luna-mux 重新编译，产物 app/native/target/debug/luna-mux.exe。前端预览热更新不替换已运行进程中的 Rust 代码，真机复测需启动本次构建。

## 2026-09-10 数据库整体交互优化

边界：前端 DatabasePane、App 接入及主题样式/本地化；复用现有 Profile API、Runtime API、窗格指针拖拽规则、菜单样式及应用确认弹窗。不改变数据库协议、密码存储或原生生命周期。数据顺序仍以 profiles.reorder 返回为准；失败恢复列表，拖拽可取消且卸载清理监听。UI Bridge 保留既有 ref 名称和密码隔离。

- [x] 连接管理：搜索/真实分组、拖拽排序替代上移下移、行内更多菜单（编辑/复制/删除），移除重复状态和新建入口。
- [x] 表单：紧凑布局、固定底部测试/保存/保存并连接；切换或关闭保护未保存内容；使用现有确认弹窗。
- [x] 数据工作区：点表直接分页看数据，独立 SQL 草稿，结构按需加载，刷新与空/错误/加载反馈；禁用不支持的操作，结构编辑表单可取消。
- [x] 查询/导出：执行所选 SQL、Tab 缩进、统一菜单；导出当前结果而不重复执行 SQL，复制单元格；危险操作仍保留确认。
- [x] 实际组件交互回归、明暗主题/窄布局截图、typecheck/i18n/build、完整要求检查和独立 Review，更新完成记录。

完成记录：
- 排序复用窗格列表的指针交互：拖动手柄、前后插入指示、Escape/失焦取消；支持 Alt + 上下箭头。当前为组内排序，跨组调整使用连接表单的分组字段；搜索时禁用排序，避免对过滤后的片段误排。失败恢复原顺序。
- 连接操作收进悬停/键盘聚焦时出现的更多菜单，也支持右键。复制生成未保存副本，不复制密码；删除使用应用确认弹窗。SQLite 可调用现有系统文件选择器，新增 files.chooseDatabaseFile 前端适配，复用已安装的 dialog 插件与权限，无原生改动。
- 点表直接展示每页 100 行；分页与 SQL 查询分别保留结果，表浏览不改 SQL 草稿，分页行号连续。未选表时禁用数据/结构页；结构按需加载并按驱动能力显示操作。SQLite 字段和索引使用可取消的行内编辑表单。
- SQL 支持选中执行、Ctrl/Cmd + Enter、Tab 缩进，Shift + Tab 可离开编辑器。导出当前页面/查询的结果快照，不重新执行 SQL；双击复制单元格。查询耗时、加载、错误重试提示及空结果均有对应状态。

验证结果：
- 实际 React 组件 + Mock API 共 **74 项交互检查通过**：连接管理 28、查询/分页/导出及 Agent UI 桥 22、结构编辑 11、分页行号/加载与错误反馈 5、分组/复制/删除/菜单及窄布局 8。包含保存和凭据失败后重试、不丢草稿、取消破坏性操作、导出不重复查询。
- 已检查深浅主题、600px/480px 布局及连接管理、数据、SQL、结构视图截图；窄布局滚动表单时底部操作保持可见，使用应用现有 Windows 滚动条样式。
- `npm run check` 通过（含 TypeScript、818 个中英文词条、契约检查、8 项终端输入测试与 cargo check）；`npm run web:build` 通过；`npm test`：242 passed / 5 ignored / 0 failed。
- 独立审查发现并修复三项 P2：提示条关闭按钮误触表单提交、Shift + Tab 被拦截、Agent 从数据页执行后快照仍返回旧表结果；上述均有实际组件回归，最终复审无新增阻塞问题。
- 临时浏览器验收页和脚本已归档到本机临时目录后从工程移除。真实远端数据库、macOS、系统文件选择器及桌面下载落盘仍需真机验收；Mock 验证不作为这些场景成功的证据。本轮前端由现有本地预览热更新，未改动原生数据库驱动。


## 2026-09-10 圆角、SQL 默认分页与列备注

边界：复用现有 sqlparser、Runtime manager、Tauri execute 命令和主题变量；execute 新增可选 pageOffset，结果新增可选 hasMore，列结构新增可选 comment。缺失新字段的旧调用与 JSON 可兼容，不改连接持久化、密码、SSH 或终端行为。

- [x] SQL 查询移到首个标签；工具栏、侧栏、工作区、标签、编辑器和表格复用应用圆角。
- [x] SQL 默认每页 500 条，通过 offset 真正读取后续数据。MySQL/MariaDB 首屏 LIMIT 0, 500、下一页 LIMIT 500, 500；PostgreSQL/SQLite 使用 LIMIT 500 OFFSET n。保持用户显式 LIMIT 总量和初始 OFFSET。
- [x] 翻页使用上一次执行的 SQL，新查询页码归零；翻页失败保留原页。数据预览固定 LIMIT 500，仅展示前 500 条；导出当前查询页/预览结果。
- [x] MySQL/MariaDB COLUMN_COMMENT、PostgreSQL col_description + attnum 列备注接入；SQLite 无原生备注时显示空值，支持长备注换行和悬停查看。
- [x] 数据库端给普通查询加 LIMIT；不改写写入、多语句、SELECT INTO、写入 CTE 等语句。任意 SQL 都等到数据库执行完成，流式结果仅保留上限内行，不提前断开吞掉后续错误。SQLite VALUES 等不能追加 LIMIT 的语法保持原文，禁用自动翻页。
- [x] 验证、最终构建和预览更新。

独立审查已修复两项问题：截断后过早断开会漏掉后续 SQL 错误/事务完成；SQLite VALUES/复合查询末尾 VALUES 不能直接附加 LIMIT。最终复审无其余已确认阻断。

验证记录：完整原生测试 251 passed / 5 ignored / 0 failed；最后 SQLite VALUES 修正后，17 项数据库专项复测全部通过。实际 React 组件 Mock API 17 项检查通过，覆盖首个 SQL 标签、每页 500 条/offset 0、500、1000、末页禁用、翻页失败保留、执行 SQL 与草稿隔离、固定表预览上限、备注和圆角。已查看深色结构、浅色 SQL、600px/480px 窄布局截图。npm run check（821 个词条）、最终 typecheck、web:build 和原生调试版 cargo build 通过。调试预览已更新为本次原生构建。

普通单条 SELECT/只读 WITH 可自动分页；显式 LIMIT/OFFSET 保留总量。动态 LIMIT、特殊 FETCH 或无法可靠改写的 SQL 保留原文、不自动翻页，结果存储仍有行数上限并等待执行完成。真实 MySQL/MariaDB/PostgreSQL 服务、TLS、macOS 的列备注和查询仍需手工验收；协议夹具和 Mock 检查不等于这些真机验证。

## 历史数据库窗格重连修复

L2：前端窗格路由和连接引用恢复，复用 muxPanes.save 的 targetId 保存 database-profile:<id>，只保存公开 Profile ID，不保存密码；不变更数据库 Schema、原生 SSH/PTY 或驱动。侧栏重连按 kind 分流，DatabasePane 负责自身 DB Runtime；分屏和重启恢复保留连接引用。旧窗格没有引用时不根据名称猜测连接，展示已有连接列表，用户选择后保存引用。检查新旧窗格、已删除连接、重连和分屏，验证不会调用终端 Runtime API。

- [x] 按窗格类型重连，数据库双击不进入 SSH/本地终端；重启和关闭同样按类型处理，清除旧误启动留下的终端错误状态。
- [x] 保存/恢复 Profile 引用及只读模式，分屏保留引用；旧空引用手选一次后保存，已删除引用不会连接其他同名服务。
- [x] 组件回归、要求检查、独立审查及文档记录。

验证：实际 React StrictMode + 异步 Mock API 15 项检查通过，覆盖旧空引用手选、重新挂载恢复、只读模式、已删除连接、关联保存失败与重试、重复重连和卸载清理。直接执行 App 真实函数体并注入 Mock 协作者的 9 项检查通过，覆盖 DB/SSH/本地分流、终端启动防护、重启、引用保存及分屏。新增 `tests/database-pane-target.test.mts` 的 3 项测试通过（运行 `node --experimental-strip-types --test tests/database-pane-target.test.mts`），覆盖特殊字符 ID、旧空引用、非 DB 引用和损坏编码。

组件检查发现并修复保存关联后父层更新 initialProfile 导致重复连接的竞态：手动连接开始即标记已尝试初始连接，并检查已拥有的 Runtime。已删除引用改选其他连接时仍需点击连接，不隐式连接。独立复审无新增确定问题。

`npm run check`、最终 `npm run typecheck`、`npm run web:build` 通过；`npm test` 为 251 passed / 5 ignored / 0 failed。本轮未修改原生层，正在运行的调试预览通过 Vite 热更新获取前端修复；真实远端数据库及关闭应用后重开仍需手工验收，Mock 检查不作为这些真机场景的证据。临时组件验收页归档后移出工程。

## 两列布局恢复、数据库图标与表数据分页

L2：前端布局恢复、数据库组件和本地化。布局仍以 Session.layout 为来源，缺失/不完整布局使用已有 twoColumns 预设，完整自定义布局保留。DB 图标按 kind 判断。预览与 SQL 共用 databaseRuntimes.execute 的 pageOffset → 原生 execute_page → 方言 LIMIT/OFFSET，每次只传回一页，不全量读取后切片；页码属于组件，成功后更新，失败保留当前页。旧 API/JSON 无变化，不改凭据、SSH、PTY 或驱动。新增翻页按钮同时接入现有 UI bridge。验证布局恢复/最小化、真实组件分页/失败/切表/只读和原生分页测试。

- [x] 修复缺失/不完整布局的两列兜底，并统一侧栏数据库图标；调整分隔线时保存完整布局树，保留最小化窗格的位置和比例。
- [x] 表预览每页 500 条真实查询、前后翻页、刷新当前页；SQL 和预览页码互相独立。翻页失败保留旧页，切表归零，导出仅使用当前页。
- [x] 回归、构建、独立审查和预览验证。

验证：`tests/mux-layout.test.mts` 8 项通过，覆盖混合类型、漏 DB 引用、完整自定义布局、最小化后调整为 70/30 再恢复、隐藏分支路径映射、空列表和重复/失效引用；连接引用原有 3 项通过。实际 DatabasePane + Mock API 32 项行为检查通过（分页 18、bridge/方言标识符/导出 9、审查回归 5），覆盖 offset 0/500/1000、末页、失败、切表、草稿隔离和当前页导出。提取真实 MuxLayout 渲染函数及侧栏图标表达式的隔离夹具验证了两列几何及 DB 图标；终端/DB 内容为占位组件，不是真实终端验收。已检查布局、600px 深色与 480px 浅色表分页截图。

独立审查发现并修复：bridge fill selectedTable 沿用旧表页码；最小化期间 resize 丢失隐藏节点导致恢复时比例重置。新增回归通过，最终复审无新增具体问题。

`npm run check`、最终 typecheck/web:build 通过；`npm test` 251 passed / 5 ignored / 0 failed（包含实际 SQLite 多页数据读取和 MySQL/PostgreSQL LIMIT/OFFSET 方言测试）。本轮无原生代码修改；调试预览进程正常，Vite 已提供最新分页和布局代码。真实远端 DB、macOS 及应用完整退出重开仍需手工验收。特殊 SQL 的既有自动翻页限制保持不变；不支持安全改写的语句不会在内存中伪装分页。临时浏览器夹具及脚本归档后从工程移除。

## 数据库可选页大小与紧凑结果布局

本节取代上文历史实现中的固定每页 500 行：SQL 查询与表数据浏览现在分别默认每页 50 行，并可选择 50/100/200。React 将页大小和偏移同时传给既有 `database_runtime_execute`；command 在存在 `pageOffset` 时仍把 `maxRows` 传入 `execute_page`，Runtime 的方言分页重写、结果上限和 `hasMore` 判断使用同一个页大小。未传 `maxRows` 的旧调用仍默认 500，公共字段、持久化和 MCP 契约不变。

紧凑窗格阈值提升到 560px，SQL 编辑器压缩后为结果区保留 190px，并由右侧工作区整体滚动；页大小控件使用应用标准控件圆角。数据库专项测试 17 项通过，新增 SQLite 50 行两页实读及 MySQL/PostgreSQL 50/100 行方言断言；`npm run check` 与完整原生测试（251 passed / 5 ignored / 0 failed）通过。真实远端数据库和截图式视觉验收仍需手工确认。
