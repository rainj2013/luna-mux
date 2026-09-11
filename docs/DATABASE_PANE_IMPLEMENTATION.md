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

## 会话级写授权与控件错误回传（2026-09-10）

L2：前端数据库组件、样式与本地化；新增一个纯函数分类器。授权状态只存在于 React 组件内，不进入 `controls.current`、持久化、Control contract 或 MCP 目录，因此 MCP 无法读取、授予或切换授权。

边界：允许修改 `app/frontend/src/components/DatabasePane.tsx`、新增 `app/frontend/src/database-write-kind.ts` 与其测试、`styles.css` 及两套 locale。不改 Rust、Tauri command、数据库驱动、凭据存储或公共 JSON。

数据归属：写授权是"本次连接会话"的前端临时状态，唯一来源为 `DatabasePane` 的 `writeGrants { dml, ddl }`；`runtimeId` 变化（重连/切换连接）时重置。DDL 与 DML 分开授权，因为 DDL 影响面更大且不可逆。

调用链：用户在 SQL 页签尾部点击 DML/DDL 开关（`PencilLine`/`Hammer`）授予授权 → `execute`/`addColumn`/`addIndex`/`dropColumn`/`dropIndex`/`importData` 在动作前调用 `approveWrite(kind, ...)`，已授权则直接执行，未授权则回退到既有确认弹窗；`importSql` 需要同时持有 DML 与 DDL 授权。Agent 经由 UI bridge 的 `fill` + `click` 走同一组处理函数，因此人工授权后 Agent 同类 SQL 不再重复弹窗。

分类器：`databaseWriteKinds(sql)` 先屏蔽注释（`--`、`#`、`/* */`）与字符串字面量（含 `''`/`""`/反斜杠转义），再按 `;` 拆分逐条分类，任一语句为写即判定为写。`DML = insert|update|delete|replace`；`DDL = create|drop|alter|truncate|attach|vacuum|reindex`；`WITH`/`EXPLAIN` 语句做整词扫描以覆盖嵌套写。多语句按最宽类型判定，避免 `SELECT 1; DROP TABLE users` 这类批量语句绕过授权。注释屏蔽偏保守（`#` 按 MySQL 行注释处理），最坏情况是多一次确认，不会漏判写操作。

- [x] 会话级 DML/DDL 独立授权开关，重连自动重置；开关不进入 UI bridge 控件清单。
- [x] 所有写路径（执行 SQL、增删字段/索引、导入数据、导入 SQL）统一经 `approveWrite` 判定。
- [x] `runAction` 增加 `reportError` 参数；`execute`、`addColumn`、`addIndex`、`viewOverview`、`viewStructure`、`tableControls` 与翻页/页大小助手在失败时回传真实错误，Agent 不再只看到 `accepted: true`。
- [x] 只读校验统一到处理函数：`dropIndex`、`importSql`、`importData` 由原先只依赖按钮/菜单 `disabled` 改为在处理函数内检查 `readOnly` 并回传错误；`readOnly` 控件仅在未连接时注册到 UI bridge，连接期间 Agent 的 `fill` 会被拒绝（当时因控件未注册而返回「未知或禁用的数据库窗格控件」，2026-09-10 起改为显式返回 `database.readOnlyLocked`），不能清除已建立连接的只读标志（设置表单在连接期间本就是 `fieldset disabled`）。
- [x] 翻页守卫：记录最近一次执行语句是否含写操作，写语句禁用界面翻页/页大小重放，并且不注册 MCP `previousPage`/`nextPage` 控件，避免 `INSERT ... RETURNING` 之类被静默重复执行。

测试：`tests/database-write-kind.test.mts` 7 项通过，覆盖纯读、单条 DML/DDL、多语句混合、注释与字符串内的分号、大小写与 `WITH` 嵌套写。`node --experimental-strip-types --test tests/*.mts` 29 项通过；`npm run typecheck`、`npm run i18n:check`（833 keys）通过。

已知限制（本轮未改动，需单独决策）：MySQL/PostgreSQL 后端仍未强制 `read_only`（仅 SQLite 以 `SQLITE_OPEN_READ_ONLY` 打开），非 SQLite 的只读完全依赖前端标志；本轮已让前端标志在连接期间不可被 UI bridge 清除，但后端仍无第二道防线，如需强约束应在原生层拒绝或使用只读账号；人工授权后 Agent 同类写操作会静默执行，符合"一次授权覆盖本次会话"的设计但也意味着授权范围较大；窄窗格下新增两个授权按钮可能被挤压。以上留作后续验收项，真实数据库与实机交互仍由手工验收。

## 写确认弹窗说明 DML/DDL 范围与就地会话授权（2026-09-10）

L2：前端确认弹窗（App）与数据库组件、样式与本地化；未新增纯函数。授权状态归属不变，仍只存在于 `DatabasePane` 内，未新增 UI bridge 控件。

边界：允许修改 `app/frontend/src/App.tsx`（`ConfirmationOptions` 与 `ConfirmationDialog`）、`app/frontend/src/components/DatabasePane.tsx`、`styles.css` 及两套 locale。不改 Rust、Tauri command、数据库驱动、凭据存储或公共 JSON。

数据归属：写授权的唯一来源仍是 `DatabasePane` 的 `writeGrants { dml, ddl }`；弹窗只请求授权，不保存任何状态。

调用链：`execute` 在执行写语句前改用 `approveStatement(kinds)`，按「本次询问的缺口种类」（`kinds` 减去已授权）生成 `missing`，message/detail 依据 `missing` 选择；用户选择「本次会话都允许」时 `onGrant` 置位、弹窗仍以 `true` 结束，组件随即只对 `missing` 中的种类置位授权。`importSql` 仍先读取脚本文件，但弹窗只说明脚本会一次性执行，不再展示脚本内容。

弹窗契约：`ConfirmationOptions` 增加可选 `grantLabel`/`onGrant`（第三个按钮）。`ConfirmAction` 仍是 `(options) => Promise<boolean>`，其余调用方与 `SftpPane` 等既有 prop 类型不受影响。

文案：标题行按缺口种类写明 `即将执行数据写入（DML）` / `即将执行结构变更（DDL）` / 两者兼有（`database.confirmWrite` / `confirmDdl` / `confirmBoth`）；细节行固定用「单次允许：只执行这一次。/ 本次会话都允许：本次连接内的所有……都不再逐条确认，断开或重新连接后恢复。」把两个按钮的作用范围写清楚（`confirmWriteDetail` / `confirmDdlDetail` / `confirmBothDetail`）。弹窗不再展示 SQL 原文——一条语句配上「本次会话都允许」容易让人误以为授权范围只是这一条。

- [x] 写语句确认弹窗按 DML / DDL / 两者同时，明说正在执行哪类变更与「单次允许」/「本次会话都允许」各自的范围。
- [x] 弹窗内直接授予会话级授权，范围严格等于本次询问的缺口种类（DML 与 DDL 同时缺失时一起授权，并在文案中写明）。
- [x] 导入 SQL 的确认弹窗说明脚本一次性执行、不会逐条确认（`confirmImport` / `confirmImportDetail`）；行导入写明目标表（`confirmImportData`，带 `{{name}}`）。
- [x] 授权开关与确认按钮都不进入 `controls.current`，Agent 仍无法读取、授予或切换授权。
- [x] 窗格工具栏的授权开关由无标签图标改为带文字的开关（`写入权限` + `数据写入` / `结构变更`，`ShieldCheck` 图标 + `aria-pressed`），窄于 430px 时才回退为图标 + `title`/`aria-label`。（2026-09-10 合并权限组后标签变为 `访问权限` + `只读` / `允许 DML` / `允许 DDL`，见下节。）

测试：`node --experimental-strip-types --test tests/*.mts` 33 项通过；`npm run typecheck`、`npm run i18n:check`、`npm run web:build` 通过。删除了随 SQL 预览一起加入的 `app/frontend/src/database-sql-preview.ts` 与其测试（`tests/database-sql-preview.test.mts`，4 项）。

已知限制：确认弹窗及其中的授权按钮不进入 UI bridge，Agent 触发的写操作在未授权时仍只弹窗等待，无法自行授予；授权仍是「按种类」而非「按语句」，一旦授予，同类写操作在本次连接内都会静默执行；本轮未做截图式视觉验收（仓库无前端组件测试环境），三个按钮在窄窗格下的换行由 `flex-wrap` 兜底，需实机确认。

## 数据库窗格自定义右键菜单（2026-09-10）

背景：WebView 的原生菜单（Reload/Back/Inspect 及平台编辑菜单）此前只在 Windows 被拦截（`window.api.platform !== 'win32'` 直接返回），macOS/Linux 上会在没有自绘菜单的窗格（数据库窗格、SFTP 窗格）漏出。

L1/L2：前端数据库组件、全局右键拦截与本地化；新增一个纯函数。

边界：允许修改 `app/frontend/src/App.tsx`（全局拦截去掉平台判断）、`app/frontend/src/components/DatabasePane.tsx`、新增 `app/frontend/src/database-field-edit.ts` 与其测试、两套 locale。不改 Rust、Tauri command、驱动或凭据存储。

调用链：`App` 在 document 上拦截 `contextmenu`：目标是文本输入（`TEXT_ENTRY_SELECTOR`，即 textarea、contenteditable 与文本/搜索/网址/电话/邮箱/密码/数字 input）时直接放行，保留系统编辑菜单；其余目标（带 Reload/Back/Inspect 的页面菜单）一律 `preventDefault`。数据库窗格在根节点处理 `contextmenu`，命中 `input`/`textarea` 且支持选区（`selectionStart !== null`）时 `preventDefault` + `stopPropagation`，用既有 `DatabaseMenu`（portal、方向键导航、Esc/外部点击关闭、关闭后回焦）在鼠标位置打开自绘菜单。菜单项：SQL 编辑器为「执行选中内容 / 复制 / 粘贴 / 全选」，其它文本框为「复制 / 粘贴 / 全选」；数字、日期、复选框类输入不提供菜单。

粘贴：`window.api.system.readClipboard()` 读取文本，`pasteAtCaret(value, start, end, text)` 计算新值与光标位；受控输入忽略直接赋值，因此写入走原型上的 `value` setter 再派发 `input` 事件，把变更交回 React 状态。选区偏移在菜单打开时捕获，避免焦点转移后错位。

- [x] 数据库窗格右键不再出现 WebView 菜单，改为自绘菜单；终端窗格保持其原有菜单。
- [x] 文本输入的右键保留系统编辑菜单（复制/粘贴），只屏蔽带 Reload 的页面菜单。
- [x] SQL 编辑器支持「执行选中内容」，无选区或未连接时禁用。
- [x] 复制/粘贴/全选在数据库窗格文本框中可用，粘贴保持「替换选区」语义。
- [x] 数字、日期、复选框输入不弹菜单，也不会触发 `setSelectionRange` 异常。

测试：新增 `tests/database-field-edit.test.mts` 4 项（插入、替换、反向选区、越界钳制、NaN、空粘贴）；`node --experimental-strip-types --test tests/*.mts` 37 项通过；`npm run typecheck`、`npm run i18n:check`（842 keys）、`npm run web:build` 通过。

已知限制：数据库窗格的自绘菜单会覆盖其文本框的系统编辑菜单（缺少撤销/剪切/拼写检查），若更希望这些字段直接用系统菜单，删掉窗格根节点的 `onContextMenu` 即可；本轮未做截图式视觉验收，菜单定位与窄窗格表现需实机确认。

## 提示消息改为悬浮自动消失（2026-09-10）

背景：提示（「本次连接内结构变更不再逐条确认」「已复制单元格内容」「数据库连接正常，可保存」「已保存」等）原先与 `error` 共用同一条 `.database-feedback` 状态栏，并在两处渲染：连接管理表单里紧跟 `.database-form-scroll` 之后（`managementOnly` 生效时可见），以及 `.database-main` 末尾（仅 `!managementOnly` 可见）。两者都参与 flex 布局（`flex: none`），会挤压内容高度与表单底部的操作栏，而且常驻不消失。

L1：单个前端组件与样式；状态仍属于 `DatabasePane`，不涉及共享契约、持久化或 UI bridge。

边界：允许修改 `app/frontend/src/components/DatabasePane.tsx` 与 `app/frontend/src/styles.css`。不改 Rust、Tauri command、驱动、凭据存储、locale 或桥接控件清单。

调用链：`notice` 仍由各处理函数写入，但改为经 `showNotice(text)` 设置——除文本外额外递增 `noticeRevision`，使得重复出现同一句文案时定时器会重新计时。`useEffect([notice, noticeRevision])` 在文案存在时启动 `DATABASE_NOTICE_MS = 5000` 的定时器清空 `notice`，组件卸载或文案变化时清理。`error` 不走定时器：错误需要用户确认，保持常驻直到下一次 `runAction` 或手动关闭。

呈现：两处渲染合并为 `.database-pane` 根节点上的唯一 `feedback`，`.database-pane` 增加 `position: relative`，`.database-feedback` 改为 `position: absolute; left: 50%; bottom: 64px; translateX(-50%)` 的悬浮卡片（`max-width: min(560px, calc(100% - 24px))`、边框、阴影、`z-index: 30`），不再占据布局高度。`bottom: 64px` 让浮层落在窗格底部之上：连接管理弹窗底部是 `.database-form-actions` 操作栏（约 60px 高），查询视图底部是 `.database-pagination` 分页条（约 39px 高），浮层不会盖住「测试连接 / 保存 / 取消」或分页按钮。右侧关闭按钮保留。

- [x] 提示消息 5 秒后自动消失，也可手动关闭；重复同一句提示会重新计时。
- [x] 提示消息悬浮在窗格底部上层，不再挤压查询/表单区域高度。
- [x] 连接管理弹窗只渲染一份提示（原先表单内那份已删除），测试连接结果仍然可见。

测试：本次为纯前端交互与样式调整，无新增单测；`npm run typecheck`、`npm run i18n:check`、`npm run web:build` 通过，`node --experimental-strip-types --test tests/*.mts` 33 项通过。

已知限制：错误提示仍需手动关闭（若希望同样自动消失需另行确认）；`bottom: 64px` 是按当前操作栏高度取的固定值，若底栏高度变化需同步调整；未做截图式视觉验收，需实机确认。

## 结果表格多选与「复制为 INSERT」（2026-09-10）

背景：数据/查询结果表格此前只能双击单元格复制单个值（`DatabaseResultGrid.onCopy`），没有行选择，也没有右键菜单。

L1：新增一个纯函数模块、结果表格组件的交互与样式，以及两套 locale；不涉及 Rust、Tauri command、驱动、持久化或 UI bridge 控件清单（右键菜单仍只由人工触发，Agent 看不到也点不到）。

边界：允许新增 `app/frontend/src/database-insert-sql.ts` 与其测试，修改 `app/frontend/src/components/DatabaseUi.tsx`（`DatabaseResultGrid`、`DatabaseResultSelection`）、`app/frontend/src/components/DatabasePane.tsx`（菜单项与剪贴板写入）、`styles.css` 及两套 locale。

数据归属：结果数据仍来自 `DatabaseQueryResult`；选择状态是 `DatabaseResultGrid` 的内部 UI 状态（`selectedRows: Set<number>` + `anchorRow`），换结果对象时重置，不进入任何 manager、快照或 MCP 返回值。

交互：左键点行选中该行；`Cmd/Ctrl` 点击切换单行；`Shift` 点击按锚点扩选区间。右键时若指针所在行已在选择内则保留整段选择，否则把该行变成新的选择；随后按鼠标位置弹出该窗格自有的 `DatabaseMenu`。菜单项：

- `复制单元格`（`database.copyCell`）——仅在右键落在数据单元格（非行号列）时出现，沿用原有 `copyCell`。
- `复制 {{count}} 行为 INSERT（{{table}}）`（`database.copyInsertRows`）——表名取当前左侧选中的表，写在标签里以免误解；未选表时该项禁用并提示先选表（`database.copyInsertNoTable`）。复制成功后提示「已复制 {{count}} 行 INSERT 语句」（`database.copiedInsert`）。

SQL 生成（`database-insert-sql.ts`）：`databaseInsertSql(table, columns, rows, driver)` 生成多行 `VALUES` 的 `INSERT INTO`，标识符按驱动加引号（MySQL 用反引号，SQLite/PostgreSQL 用双引号，内部引号翻倍）；字面量按 JSON 类型还原——`null`/`undefined` 与 BLOB（SQLite 只回传 `{ bytes: n }`，没有内容）→ `NULL`，布尔 → `TRUE`/`FALSE`，数字 → 原样，其余 → 单引号包裹并把 `'` 翻倍（MySQL 额外把 `\` 翻倍，因为默认 `sql_mode` 下反斜杠是转义符）。每 200 行拆成一条语句（`DATABASE_INSERT_ROWS_PER_STATEMENT`），避免超长多行 `VALUES` 撞上引擎限制。

- [x] 结果表格支持行选择，并用 `Cmd/Ctrl`/`Shift` 扩选多行，选中行有高亮样式。
- [x] 右键弹出自绘菜单，可复制单元格，也可把 1..n 行复制成一条多行 `INSERT` 语句。
- [x] 生成 SQL 的标识符与字面量按驱动转义；SQLite 整数以字符串回传，因此数值型字段会写成带引号的字面量（SQLite 类型亲和性会照常转换）。
- [x] 未选择表、结果为空或行数为 0 时不生成 SQL；BLOB 因内容未回传只能还原为 `NULL`。

测试：新增 `tests/database-insert-sql.test.mts` 9 项（字面量类型、单引号/反斜杠转义、标识符转义、短行补 `NULL`、空输入、200 行分片）；`node --experimental-strip-types --test tests/*.mts` 42 项通过；`npm run typecheck`、`npm run i18n:check`（851 keys）、`npm run web:build` 通过。另用 `sqlite3` 对生成语句做了落库回读验证（引号、反斜杠、换行、`NULL`、布尔均按预期还原）。

已知限制：行选择是鼠标交互，未加键盘导航或快捷键；`INSERT` 的目标表取自左侧当前选中的表，查询视图里手写 SQL 连表时该表名可能不是用户想要的（标签里已标出表名）；BLOB 只写成 `NULL`；大量行的复制会拆分多条语句，粘贴时按顺序执行即可。

## 权限开关合并为「只读 / 允许 DML / 允许 DDL」（2026-09-10）

背景：只读此前是连接表单里的复选框，两个写入授权开关在窗格工具栏的页签尾部，三者分处两地；用户要求合并到同一处，只读与两个写入授权互斥，并再次确认不需要后端限制。

L2：前端数据库组件、样式与本地化。未改 Rust、Tauri command、数据库驱动、凭据存储或公共 JSON；`readOnly` 仍只是 `DatabasePane` 的组件状态，不进入持久化以外的任何管理器。

边界：修改 `app/frontend/src/components/DatabasePane.tsx`、`styles.css` 及两套 locale；`docs` 之外的契约无变化。

数据归属：`readOnly` 与 `writeGrants { dml, ddl }` 都是「本次连接」的组件状态，唯一来源是 `DatabasePane`。窗格把只读写进 pane target（`databasePaneTarget(profileId, readOnly)`），因此重新打开窗格仍记得上次的选择；`onProfileSelected(profile, readOnly)` → `saveDatabasePaneProfile` 负责更新 target，`writeGrants` 仍不持久化。

交互：窗格头部（`.database-toolbar`）新增一个 `访问权限` 组，包含三个带文字的开关：

- `只读`（`Lock`，`aria-pressed`）——开启时清空两个写入授权；与两个写入授权互斥。
- `允许 DML` / `允许 DDL`（`ShieldCheck`）——点击仍先弹出「本次会话允许……」的确认；在只读状态下点击会先关闭只读再授权。

互斥规则与连接重建：`applyReadOnly(next)` 统一处理只读切换——写入状态、清空或保留授权、持久化 targetId、提示消息，然后判断是否需要重建连接。只有 SQLite 需要重建（只读标志会带进 `SQLITE_OPEN_READ_ONLY` 的打开模式）：已连接且驱动为 SQLite 时 `disconnect()` + `connect(next, true)`；已连接且是 MySQL/PostgreSQL 时只切换前端门禁，不重建、不清空结果。`connect(nextReadOnly, restart)` 的两个参数用于绕过 React 尚未提交的旧状态闭包与「已连接就直接返回」的保护。提示文案区分是否重建（`readOnlyNotice` / `readOnlyReconnected` / `writeNotice` / `writeReconnected`）。

连接表单：移除只读复选框（`.database-options` 现在只在非 SQLite 时渲染 TLS 一项），新建 / 编辑连接不再携带只读；打开窗格后在头部选择。`testConnection` 仍固定以 `readOnly: true` 试探。

UI bridge 边界：`readOnly` 控件仍只在未连接时注册进 `controls.current`，`fill` 在连接期间显式抛出 `database.readOnlyLocked`，因此 Agent 依旧无法清除已建立连接的只读标志，也无法借只读开关触发重建。

- [x] 只读与两个写入授权在同一组内，只读开启即清空授权；点击任一写入授权会先关闭只读。
- [x] 只读在连接前可选，连接后切换会按驱动决定是否重建（仅 SQLite 重建），并给出对应提示。
- [x] 只读选择写入 pane target，重开窗格保留；连接表单不再有只读复选框。
- [x] 窄窗格（≤430px）隐藏开关图标只保留文字，避免两个 `ShieldCheck` 无法区分；`只读` / `允许 DML` / `允许 DDL` 标签始终可见。

测试：`node --experimental-strip-types --test tests/*.mts` 42 项通过；`npm run typecheck`、`npm run i18n:check`（859 keys）、`npm run web:build` 通过。纯函数（分类器、INSERT 生成、字段编辑）无改动。

已知限制：MySQL/PostgreSQL 的只读仍只是前端门禁（沿用既有决定，不加后端限制）；SQLite 连接期间切换只读会断开并重连，当前表选择、结果与页码会清空，提示消息里已写明；窗格头部在极窄宽度下依赖标题省略号让位，需实机确认。

## 结果表格支持按住拖动选择多行（2026-09-10）

背景：多选此前只有 `Cmd/Ctrl` 点选和 `Shift` 点选两种方式，用户要求能按住鼠标拖动连续选择，方便一次复制多行 INSERT。

L1：单个前端组件与一条样式；不涉及数据、契约、持久化或 UI bridge（右键菜单与选择状态都不进入 MCP 控件清单）。

边界：修改 `app/frontend/src/components/DatabaseUi.tsx`（`DatabaseResultGrid`）与 `styles.css`。

交互：在行上按下左键即把该行设为锚点并只选中它；按住拖动时按指针所在行扩展为「锚点..当前行」的连续区间，可反向拖动。拖动期间在网格边缘 24px 内会按次滚动，便于一次选中屏幕外的行；指针移出网格仍有效（`pointermove`/`pointerup`/`pointercancel` 监听在 window 上，组件卸载时在 effect 清理中解绑）。`Shift`/`Cmd`/`Ctrl` 点选规则不变：这些修饰键按下时 `startRowDrag` 直接返回，锚点不被改写，因此 `Shift` 扩选仍以之前的锚点为准。

文本选择：拖动开始时会同步给滚动容器加 `.selecting`（`user-select: none` + 默认光标），并在每次 `pointermove` 清掉浏览器已产生的选区，因此拖动不会再和原生文本选择互相打架。代价是数据单元格不再能用鼠标拖选文本（双击复制单元格和右键「复制单元格」仍在）。

尾随点击：拖动跨行后 `pointerup` 常落在别的行上，`click` 冒泡到的是公共祖先而不是 `<tr>`；若确实回到起始行，`end()` 置位的 `suppressClick` 会让那次 `click` 不把选择收敛成单行，下一次 `pointerdown` 复位。

- [x] 左键按住拖动可连续选择多行，支持反向拖动、跨屏滚动、在网格外继续拖动。
- [x] `Shift` / `Cmd` / `Ctrl` 点选与右键菜单行为不变，拖动结束的尾随点击不会重置选择。
- [x] 拖动期间不触发原生文本选择（`.selecting`）。

测试：`npm run typecheck`、`npm run web:build`、`node --experimental-strip-types --test tests/*.mts`（42 项）通过。仓库没有前端组件测试环境，`DatabaseResultGrid` 的拖动手势只能手工验收（指针命中行、边缘滚动、跨行拖动后的右键复制范围）。

已知限制：边缘滚动按 `pointermove` 步进，指针停在边缘不动时不会持续滚动；拖动期间指针悬停在表头或空白处时保持上一段选择不收缩；虚拟滚动只渲染可见行，但选择按行号存储，滚动后区间仍然完整。

## 导出菜单增加 INSERT SQL、导入项标签写清范围（2026-09-10）

背景：导出菜单只有 CSV / JSON，用户问为什么没有 SQL；同时「导入 SQL 脚本」和「导入到所选表」在菜单上看不出差别。

L1：前端数据库组件的导出/导入入口与本地化，复用已有的 `databaseInsertSql`；不涉及 Rust、Tauri command、契约或持久化。

导出：新增「导出 INSERT SQL」（`database.exportSql`），与右键「复制为 INSERT」共用 `databaseInsertSql(table, columns, rows, driver)`，因此标识符与字面量的转义规则完全一致（MySQL 反引号 + 反斜杠转义，SQLite/PostgreSQL 双引号），每 200 行拆成一条语句（`DATABASE_INSERT_ROWS_PER_STATEMENT`）。文件名为 `<所选表>.sql`，未选表时该项禁用——INSERT 必须有目标表名。生成文本下载走 `downloadDatabaseFile(text, filename, mime)`，原来的 `downloadDatabaseResult` 也改为调用它。

导出范围：三种格式导出的都是「当前屏幕上结果集快照」（数据页=当前页，查询页=当前查询结果页），不会重新执行 SQL，也不会翻页拉全表。原因与既有的复制一致：重跑可能重复执行写语句，整表导出需要另行设计分页拉取与进度反馈。

导入标签：`导入 SQL 脚本（任意语句）` / `导入到所选表（CSV / JSON）`，把两者的作用域直接写在菜单里。两者的实际差别（也是行为边界）：

- 导入到所选表：必须先在左侧选中一张表，文件按 CSV（首行表头）或 JSON（对象数组）解析成列与行，由原生 `import_rows` 在一个事务内逐行 `INSERT INTO "表" (列...) VALUES (...)`；不能建表、不能改结构，任何一行失败整批回滚，只支持 SQLite。
- 导入 SQL 脚本：文件是任意 SQL 文本，由原生 `import_sql` 在一个事务里 `execute_batch` 执行，可以包含建表/改结构/多语句，不针对特定表；因为可能同时含 DML 与 DDL，需要两项写授权（或弹窗单次确认），同样只支持 SQLite。

- [x] 导出菜单支持 INSERT SQL，转义与「复制为 INSERT」一致。
- [x] 未选表时「导出 INSERT SQL」禁用；导出内容仍不触发重新执行查询。
- [x] 两个导入项标签写明「任意语句」与「CSV / JSON」的范围差别。

测试：`npm run typecheck`、`npm run i18n:check`（860 keys）、`npm run web:build`、`node --experimental-strip-types --test tests/*.mts`（42 项）通过；INSERT 生成本身由 `tests/database-insert-sql.test.mts` 覆盖。

已知限制：导出只包含当前页（数据页默认 50 行），大表导出需要先调页大小或另做整表导出；CSV 导入解析很朴素（不处理引号内的逗号与换行）；导入两项仍只在 SQLite 连接下出现。

## 整库 SQL 导入 / 导出（2026-09-10）

背景：用户要求导入导出直接针对整个库（SQL 里自带表名）、只保留 SQL 一种格式、考虑大批量数据的性能；随后又明确入口位置——导入 / 导出放到左侧栏（顶部一个整库按钮、单表右键导出该表），SQL 查询结果的导出留在查询页。

L2：新增 Tauri command 与原生的 SQL dump 生成器，替换 `invoke_handler` 中原来的字符串版 `database_runtime_import_sql`；前端调整数据库窗格的入口布局。按 `docs/AI_DEVELOPMENT_GUIDELINES.md` 走完整流程。

契约：

- `database_runtime_export_sql(runtimeId, path, table?) -> { tables, rows, bytes }`：`table` 为空时 dump 整个库，给出表名时只 dump 这一张表或视图（连同它的索引与触发器）。返回表数、行数与文件字节数。
- `database_runtime_import_sql_file(runtimeId, path)`：从磁盘读整份 SQL 并在一个事务里执行，失败整份回滚。
- 两个命令都走 `database_worker`（阻塞线程池），只传文件路径，SQL 文本不进 IPC。

导出实现（`export_sql` + `write_sql_dump` / `sqlite_dump_objects`）：

- 在同一个读事务里生成整份文件，保证所有表来自同一快照，写入过程中不会被并发写入撕裂。
- `BufWriter::with_capacity(1 << 20, File::create(path))`：内存占用是固定缓冲区，与库大小无关；`bytes` 取自写完后的 `metadata().len()`。
- 顺序：先所有 `CREATE TABLE`，再各表数据，最后索引 / 触发器 / 视图。数据先于索引落库，恢复时不必为每行付索引维护成本。
- 数据按表分批：`INSERT INTO "表" ("列", ...) VALUES (...), (...);`，每 200 行一条语句（`SQL_DUMP_ROWS_PER_STATEMENT`），把语句数量与解析开销压到两个数量级以下，同时远低于任何 SQLite 构建的语句长度上限。
- 对象来源是 `sqlite_master`，过滤 `sql IS NOT NULL`（自动索引没有 DDL）与 `name NOT LIKE 'sqlite\_%'`（`sqlite_sequence`、`sqlite_stat*` 等内部表）。单表导出按 `name = ?1 OR (tbl_name = ?1 AND type IN ('index','trigger'))` 取该对象及其附属对象；名字不存在时报「数据库中找不到该表或视图」，不会静默产出空文件。视图只有 DDL，没有数据。
- `sqlite_sequence` 不导出也没关系：AUTOINCREMENT 计数会在恢复后由 `max(rowid)` 重建。

字面量编码（`write_sql_value`）：

- `NULL` → `NULL`；INTEGER → 十进制；REAL → `{:?}`（最短且可精确回读的表示），非有限值（NaN / ±Inf）无法用 SQL 字面量表达，写成 `NULL`。
- TEXT → 单引号包裹，`'` 双写转义，按字节写出。TEXT 在 SQLite 里可以存非法 UTF-8 字节，这种值写成 `CAST(X'..' AS TEXT)`，既保住原始字节又让 dump 文件本身仍是合法 UTF-8。
- BLOB → `X'..'` 十六进制（`write_sql_hex`）。
- 标识符（表名 / 列名）统一 `"` 包裹并把 `"` 双写（`sql_dump_identifier`）。

导入实现（`import_sql_file` → `import_sql`）：整个文件 `read_to_string` 后 `execute_batch`，全程一个事务，任何一条语句失败整份回滚（既有语义，未改动）。文件里自带 `BEGIN` / `COMMIT` 时无法再被外层事务包住，rusqlite 报错会映射成「脚本自带 BEGIN/COMMIT，无法整体执行：请移除脚本里的事务语句后重试」；路径不存在时报「无法读取 SQL 文件：…」。连接没有开启 `PRAGMA foreign_keys`，dump 的表顺序不会因为外键引用关系而失败。

前端入口（`DatabasePane`）：

- 左侧栏顶部（搜索框下方，仅连接后显示）一个「导入 / 导出」按钮，打开整库菜单：`导入整库 SQL 文件…` / `导出整库为 SQL 文件…`。
- 左侧栏表列表单项右键（`onContextMenu`，与连接列表右键同一套 `openMenu` + `DatabaseMenu`）只有一项：`导出该表为 SQL 文件…`。
- 查询页动作栏的「导出结果」导出刚执行那条 SQL 的结果快照为 INSERT SQL，不依赖左侧表选择；简单 `FROM` 查询使用来源表作为目标，无法推断时使用 `query_result`。
- 数据页工具栏不再有导入 / 导出（保留刷新与分页）；隐藏的 `<input type="file">`、CSV / JSON 解析（`importData`）已删除，导入改由原生按路径读取，导出默认文件名取连接名或表名并过滤文件系统非法字符。
- 结果提示复用悬浮提示条（5 秒自动消失、可手动关闭）：整库导出显示张数 / 行数 / 大小 / 路径，单表导出显示表名 / 行数 / 大小 / 路径，导入显示文件路径。
- `controls.current` 里的 `exportCsv` / `exportJson` 移除：新的导入导出需要用户先选文件路径，零参数的 UI bridge 控件表达不了，故不注册替代项。

为什么当时只有 SQLite（后续已补齐，见文末「MySQL / MariaDB / PostgreSQL 也支持整库 SQL 导入导出」）：dump 的 DDL 必须逐字可靠，而 DDL 与字面量语义都是驱动专属的——SQLite 可以直接从 `sqlite_master.sql` 拿到原始 `CREATE` 语句，MySQL / MariaDB 需要 `SHOW CREATE TABLE`，PostgreSQL 没有等价语句，只能从 `pg_catalog` / `information_schema` 重新拼装（类型、默认值、序列、约束、权限）。值编码同理：MySQL 的 `Value` 与 PostgreSQL 的文本协议各有转义和类型规则，写错会产出无法恢复或静默丢数据的脚本。因此首版只落在 SQLite（本机唯一在用的驱动），其他驱动的导出 / 导入在原生侧返回「SQL 导出目前仅支持 SQLite 连接」，界面上的菜单项同时禁用，避免给出「看起来能用但结果是错的」的入口。补 MySQL（`SHOW CREATE TABLE` 可直接复用）与 PostgreSQL（需 catalog 拼装 + `COPY` 流式读）是后续独立工作。

测试：`app/native/src/database_runtime.rs` 新增 4 项——`sqlite_dump_round_trips_schema_and_values_into_a_new_database`（多表 / 索引 / 引号与换行 / 非法 UTF-8 TEXT / REAL / BLOB / NULL，校验 `bytes` 等于文件大小、dump 文本含表名与 `CAST(X'FF41' AS TEXT)`，再导入到另一个库逐行比对）、`sqlite_table_dump_keeps_only_the_requested_table_and_its_indexes`（只含目标表与其索引，其他表不出现，找不到表报错）、`sqlite_dump_of_an_empty_database_restores_without_error`、`sqlite_import_from_file_is_all_or_nothing`（中途出错整批回滚、自带 BEGIN/COMMIT 的提示、路径不存在的提示）。

- [x] 左侧栏顶部整库「导入 / 导出」按钮；单表右键导出该表；查询页保留 INSERT SQL 结果导出。
- [x] 生成的 SQL 自带表名，导入不需要选择表。
- [x] 导出走原生流式写入单事务快照，IPC 不承载数据，内存占用与库大小无关。
- [x] 只保留 SQL 通道，CSV / JSON 导出与「导入到所选表」已从导入导出界面移除（Rust 侧的 `export_csv` / `export_json` / `import_rows` 命令此前就不被窗格使用，本次未改动其注册状态）。
- [x] `npm run typecheck`、`npm run i18n:check`（864 keys）、`npm run web:build`、`node --experimental-strip-types --test tests/*.mts`（42 项）通过；`cargo test database` 4 项新增用例通过。

已知限制：MySQL / MariaDB / PostgreSQL 的整库与单表导入导出尚未实现（原因见上），菜单项禁用并给出明确文案；导入要把整份文件读成字符串，内存占用与文件大小同阶（导出侧是平坦的），超大脚本的流式导入未做；dump 不包含 `sqlite_sequence` 等内部表与没有 DDL 的自动索引；恢复顺序按 `sqlite_master.rowid`，不按外键拓扑排序（当前连接不强制外键，若将来开启 `PRAGMA foreign_keys`，父子表的插入顺序可能失败）；REAL 的 NaN / ±Inf 会退化成 `NULL`；BLOB 在结果表格里只显示字节数，导出文件里才是真实内容；无法从查询推断来源表时，查询结果 INSERT 使用 `query_result` 作为目标表，导入前可在文件中替换目标表名。

## 表列表折叠按钮移到主区域工具条（2026-09-10）

背景：数据库窗格左侧表列表的收起 / 展开按钮原本在侧栏自己的标题栏里（收起后侧栏缩成 38px 窄条，按钮留在窄条里）。用户要求改成和「折叠会话 / 窗格列表」一致的位置——即入口放在主区域那一侧。

L1：纯前端布局与样式，不涉及数据、契约、持久化、UI bridge。

改动（`DatabasePane.tsx` / `styles.css`）：

- 收起 / 展开按钮移到主区域顶部工具条（`database-tabs`）最左侧，图标沿用 `PanelLeftClose` / `PanelLeftOpen`，`aria-expanded` 跟随状态；侧栏标题栏只剩「刷新对象」按钮（连接列表状态下仍是「新建连接」）。
- 折叠后侧栏不再缩成 38px 窄条，而是与工作区侧栏一致地整体隐藏：`.database-body.sidebar-collapsed` 单列 + `.database-sidebar { display: none }`，原来 `.database-sidebar.collapsed` 与 `≤520px` 容器查询里的窄条规则一并删除。
- 新增 `.database-sidebar-toggle` 覆盖 `.database-tabs button` 的页签样式（无下边框、28px 方形、圆角 hover），避免看起来像一个页签。

- [x] 折叠按钮在主区域工具条最左侧，与工作区侧栏的收起 / 展开行为一致。
- [x] 折叠后表列表整体隐藏，主区域占满宽度；窄容器（≤520px）下同样生效。
- [x] `npm run typecheck`、`npm run i18n:check`（864 keys）、`npm run web:build` 通过。

已知限制：折叠状态下侧栏里的「刷新对象」按钮不可见，需要先展开（数据页仍可用自身的刷新按钮刷新当前表）；折叠状态仍是组件内 state，不跨会话持久化。

## 只读改成默认，写入授权跨过授权本身触发的重连（2026-09-10）

背景：用户反馈两点——权限组应当默认只读；点击写入授权时提示会重连，但重连之后授权丢了，还得再点一次。

根因：SQLite 的只读标志会带进 `SQLITE_OPEN_READ_ONLY` 的打开模式，所以「只读 → 写」必须重建运行时（这是保留的行为）。而运行时 id 变化时有一个 effect 会把两个写入授权清空（`useEffect(..., [runtimeId])`，注释写着「断开或重新连接都会撤销授权」）。授权触发的重连会改 runtimeId，于是刚点的授权被这个 effect 立刻撤销，用户必须再点一次。

L1：前端数据库窗格的状态与文案；不涉及 Rust、Tauri command、契约或持久化格式（pane target 的查询串多写一个显式值）。

改动：

- 默认只读：`DatabasePane` 的 `initialReadOnly` 默认值改为 `true`，`App.openDatabasePane(profile, title, readOnly = true)` 同步；连接表单的 `newProfile()` 也重置为只读。新建的数据库窗格与从连接库打开的都从只读开始，要写就得先授权。
- pane target 恒写标志：`databasePaneTarget(id, readOnly = true)` 现在总是输出 `?readOnly=true|false`，`parseDatabasePaneTarget` 在缺少该参数时按只读处理（老窗格恢复后只读，安全优先）。`tests/database-pane-target.test.mts` 补了显式 `readOnly=false` 的用例。
- 授权生命周期改为「跟随连接」：删掉按 `runtimeId` 清空授权的 effect，改成在 `disconnect()` 里清空。这样显式断开、以及「切回只读」都会撤销授权，但授权自己触发的那次重连不会——`toggleWriteGrant` 在 `applyReadOnly(false)`（含重连）返回之后再写授权状态，因此重连后授权仍在。
- 文案：`confirmWriteDetail` / `confirmDdlDetail` / `confirmBothDetail` / `confirmGrantDml` / `confirmGrantDdl` 里的「断开或重新连接后恢复」改为「断开连接后恢复」（与新的生命周期一致）。新增 `grantDmlReconnected` / `grantDdlReconnected`，在「授权 + 顺带重连」这一条路径上提示「已重新建立连接，本次连接内数据写入/结构变更不再逐条确认」，取代原来的「已切换为可写模式」提示。

- [x] 权限组默认只读（新窗格、连接库打开、连接表单重置都从只读开始）。
- [x] 在只读的 SQLite 连接上点「允许 DML / 允许 DDL」：确认一次 → 自动重连 → 授权保持有效，不需要再点第二次。
- [x] 显式断开、切回只读仍会撤销授权；文案与新语义一致。
- [x] `npm run typecheck`、`npm run i18n:check`（866 keys）、`npm run web:build`、`npm run check`、`node --experimental-strip-types --test tests/*.mts`（42 项）通过。

已知限制：SQLite 上「只读 → 写」的重连会重置当前视图、选中表与结果（原有行为，未改动），但保留刚授予的写入权限；MySQL / PostgreSQL 不重建连接，因此没有这个提示。老窗格的 target 里没有 `readOnly` 参数时恢复为只读，需要写就手动关一次。

## MySQL / MariaDB / PostgreSQL 也支持整库 SQL 导入导出（2026-09-11）

背景：上一版把 SQL 导入 / 导出限制在 SQLite，界面菜单项同时对其他驱动禁用。用户明确要求 MySQL 等驱动同样支持。

边界与数据归属：仍由 `DatabaseRuntimeManager` 拥有生命周期，只有「网络 Runtime 没有常驻句柄」这一点被显式处理——网络驱动把 `DatabaseConnectionConfig` 存在 `DatabaseHandle::Network` 里，查询是每次新建连接。导入 / 导出因此为本次调用建立一条短连接，用完即断开，不引入新的常驻状态；`network_config(id)` 是读取它的唯一入口，不再有第二份连接缓存。

调用链：菜单项（未变）→ `database_runtime_export_sql` / `database_runtime_import_sql_file`（未变）→ `DatabaseRuntimeManager::{export_sql, import_sql}`。这两个方法先探测句柄种类：SQLite 走原有的本地实现，网络驱动 `block_on` 到 `export_network_sql` / `import_network_sql`。命令签名与前端契约不变，`table` 仍表示「只导出这一张表 / 视图」。

MySQL / MariaDB：

- 结构来自 `SHOW CREATE TABLE`（视图用 `SHOW CREATE VIEW`），因此索引、字符集、`AUTO_INCREMENT`、分区等都由服务器原样给出，不需要拼装。表先建、数据后插、视图最后建（视图可能引用任意表）。
- 对象列表来自 `information_schema.tables`（`table_schema = DATABASE()`），单表导出在 SQL 里过滤，找不到对象返回「数据库中找不到该表或视图」且不落文件。
- 数据用 `SELECT *` 的文本协议流式读，按 `SQL_DUMP_ROWS_PER_STATEMENT`（200）行一条 `INSERT`。值编码按列元数据分三类：数值列（含 `YEAR`、`DECIMAL`）写裸字面量；时间列（`DATE` / `DATETIME` / `TIMESTAMP` / `TIME` 及其 `2` 变体）写引号字符串——它们和数值列一样是 charset 63，只看 charset 会把日期写成十六进制；其余 charset 63 的列（`BLOB` / `BINARY` / `VARBINARY` / `BIT` / `GEOMETRY`）写 `X'..'`；普通文本写单引号字符串。文本里出现反斜杠、控制字符或非法 UTF-8 时也改用 `X'..'`，这样在 `NO_BACKSLASH_ESCAPES` 开启和关闭的服务器上都按字节还原。
- 导入整段脚本作为一次请求发给服务器（`CLIENT_MULTI_STATEMENTS` 由驱动恒定协商），分号、字符串与注释由服务器解析，不做客户端切分。已知限制：dump 不含触发器、存储过程与事件（触发器体需要 `DELIMITER`，普通脚本体无法还原），文件头已注明。

PostgreSQL：

- PostgreSQL 没有 `SHOW CREATE TABLE`，结构由 catalog 拼装：列用 `pg_attribute` + `format_type` + `pg_attrdef`（保留原始类型拼写、`NOT NULL`、非序列默认值）；主键 / 唯一 / 检查约束与索引分别来自 `pg_constraint`、`pg_index`（排除约束背后的索引，避免重复创建）；视图用 `pg_get_viewdef`。
- `nextval(...)` 默认值的列改写成 `serial` / `bigserial` / `smallserial`；identity 列统一写成 `GENERATED BY DEFAULT AS IDENTITY`（`ALWAYS` 会拒绝 dump 里的显式值），两类列在数据之后用 `setval(pg_get_serial_sequence(...), MAX(...), COUNT(...) > 0)` 重新对齐序列。
- 写入顺序：建表 → 数据 → 索引 → 主键 / 唯一 / 检查约束 → 序列对齐 → 外键 → 视图。外键放在最后，恢复时不必为每行做校验，也不受建表顺序影响。
- 值走 simple query 的文本形式，全部写成引号字面量，由列类型强制转换；`bytea`（文本形如 `\x...`）和含反斜杠的值用 `E'...'`，在 `standard_conforming_strings` 开或关的服务器上都一致。已知限制：不含触发器、函数、属主与授权，文件头已注明。
- `execute_network_inner` 的 PostgreSQL 连接分支改为复用新的 `postgres_client` helper，两条路径（查询与 dump）共用同一次 TLS / 超时逻辑；原 30 秒查询超时只约束交互式查询，dump 不受它限制。

前端：`transferMenu` / `tableMenu` 去掉 `driver !== 'sqlite'`，菜单项只对「未连接」和导入时的「只读」禁用。

测试：`app/native/src/database_runtime.rs` 新增 8 项——4 项纯函数（MySQL 值按列种类编码、时间值文本、PostgreSQL 字面量与 `bytea`、列定义 / serial / identity 拼装），4 项走假服务器跑真实驱动（`mysql_dump_writes_schema_and_batched_rows`、`mysql_dump_of_one_table_filters_the_query_and_reports_missing_tables`、`network_import_sends_the_script_as_a_single_request`、`postgres_dump_writes_tables_data_constraints_and_sequences`、`postgres_import_sends_the_script_as_one_batch`）。

- [x] MySQL / MariaDB / PostgreSQL 的整库与单表 SQL 导出可用，导入仍只接受 SQL 文件。
- [x] 网络驱动的 dump 走单条短连接 + 1 MB 缓冲流式写入，不经过 webview，内存占用与库大小无关。
- [x] 找不到表 / 视图时不落文件并报错；导出文件大小取自落盘结果。
- [x] `cargo test database_runtime` 30 项通过；`npm run typecheck`、`npm run i18n:check`（866 keys）、`npm run web:build`、`node --experimental-strip-types --test tests/*.mts`（42 项）通过。

已知限制：MySQL 触发器 / 存储过程 / 事件与 PostgreSQL 触发器 / 函数 / 属主 / 授权不在 dump 内，文件头已注明；两类 dump 都不做跨表一致性快照（未使用 `START TRANSACTION WITH CONSISTENT SNAPSHOT`），导出期间发生的写入可能造成表间不一致。

## 导入 / 导出改为两个小图标按钮，文案不再强调「整库」（2026-09-11）

背景：用户反馈导入 / 导出不是常用操作，sidebar 里不该占一整行大按钮；另外导入导出本来就只针对「当前连接 + 文件」，不需要在文案里反复强调「整库」。

改动（`app/frontend/src/components/DatabasePane.tsx`）：

- 侧边栏搜索框下方那一行 `database-transfer-actions` 大按钮删除，改为侧边栏标题栏（与「刷新对象」并列）两个 14px 图标按钮：`Upload` 导入、`Download` 导出，沿用该处已有的 `.icon-button` 尺寸与 hover 样式。导入仍是写操作，只读连接下禁用，禁用时 tooltip 用 `database.readOnly` 说明原因。
- 原来那个按钮打开的 `transferMenu()` 随之删除：菜单里只有「导入 / 导出」两项，直接点更少一次操作。
- 文案去掉「整库」：`importSql`「导入 SQL 文件…」、`exportDatabase`「导出为 SQL 文件…」、`imported`「已导入 SQL：{{路径}}」、`confirmImport`「即将导入 SQL 文件，是否继续？」，英文同步；`database.importExport`（原大按钮标签）删除，i18n 从 866 条降到 865 条。
- `confirmImportDetail` 顺带修正为驱动中立：SQLite 会把整份文件放在一个事务里执行、出错整体回滚，其他驱动按语句依次执行（原文写死了「整体在一个事务里执行」，对 MySQL / PostgreSQL 不成立）。
- CSS 里 `.database-transfer-actions` 的两条规则删除；`.database-labeled-action` 保留（工具栏的「管理连接 / 断开 / 导出结果」仍在用）。

- [x] 侧边栏不再有整行大按钮，导入 / 导出是两个小图标按钮，位置与「刷新对象」一致。
- [x] 界面文案中不再出现「整库 / whole database」，表右键仍是「导出该表为 SQL 文件…」。
- [x] `npm run typecheck`、`npm run i18n:check`（865 keys）、`npm run web:build`、`node --experimental-strip-types --test tests/*.mts`（42 项）通过。

已知限制：图标按钮依赖 tooltip 表达含义，只读连接下 WebKit 对禁用按钮不显示 tooltip，此时用户需要从权限区的「只读」高亮判断原因。

## 自绘文件选择器，替换系统弹窗（2026-09-11）

背景：用户反馈导出时用的是系统保存面板（macOS 会带一个没用的 Tags 字段），而且路径选择不够直观、各平台体验不一致，希望自己写一个。

改动：

- 新增 `app/frontend/src/components/FileDialog.tsx`：一个自绘的文件 / 目录选择器，三种模式 `open` / `save` / `directory`。界面是「路径输入框 + 面包屑 + 文件夹列表 + 文件名输入 + 取消 / 确定」，与界面其余部分同一套样式，因此 macOS / Windows / Linux 表现一致，也不会再出现系统面板专属的控件。
- 目录列表复用已有的 `files.home` / `files.listLocal` / `files.parentLocal` 原生命令（原本给 SFTP 面板左侧使用），没有新增原生接口；排序为文件夹在前，名称用 `localeCompare`。列表支持单击选中、双击进入 / 确认、搜索框过滤当前目录、显示隐藏文件开关、`.sql` 之类的扩展名筛选下拉；在 `directory` 模式下选中文件夹再点「选择」即选中该文件夹，否则选中当前目录。
- 记住上一次打开的目录（模块级 `lastDirectory`），与系统保存面板的习惯一致；如果记住的目录已经不存在，依次回退到调用方建议目录、home 目录。
- 自绘对话框通过 portal 挂到 `.app-shell` 内，而不是 `document.body`：主题变量定义在 `.app-shell` 上，挂到 body 会丢样式。
- 纯路径逻辑（分隔符判断、面包屑拆分、拼接、扩展名、保存时补扩展名）拆到 `app/frontend/src/file-dialog-path.ts`，并有 `tests/file-dialog-path.test.mts`（6 项，覆盖 POSIX / Windows 盘符 / UNC）。
- 接入位置：数据库窗格的「导入 SQL / 导出 SQL / 选择 SQLite 文件」（`DatabasePane.tsx`），以及会话根目录、部署本地目录、SSH 私钥的选择（`App.tsx`）。
- 随之删除只服务系统弹窗的 `files.chooseDatabaseFile` / `chooseSqlFile` / `chooseSqlSavePath` / `chooseLocalDirectory` / `choosePrivateKey`（前端）与原生命令 `files_choose_local_directory` / `files_choose_private_key`（含 `lib.rs` 注册）；`@tauri-apps/plugin-dialog` 的 **JS** 依赖不再被引用，但 Rust 侧 `tauri-plugin-dialog` 仍用于连接归档导入等场景。
- 新增 17 条文案：`fileDialog.*`（12 条 + 2 条文件类型标签）与 `database.chooseDatabaseFile`、`database.exportSqlFileTitle`、`database.importSqlFileTitle`，i18n 从 865 条升到 882 条。

- [x] 数据库窗格的导入 / 导出 / SQLite 文件选择，以及部署目录、会话根目录、私钥选择都不再调用系统弹窗。
- [x] `npm run typecheck`、`npm run i18n:check`（882 keys）、`npm run web:build`、`node --experimental-strip-types --test tests/*.mts`（48 项）、`npm run check`、`cargo test --manifest-path app/native/Cargo.toml`（258 项通过）均通过。

已知限制：符号链接一律当成文件处理，不会进入（原生 `files_list_local` 把 symlink 与目录分开上报）；`fileDialog.*` 只有中英两份文案；Windows 下切换盘符需要直接在路径框里输入 `D:\`。
