# SSH Client 项目进度与风险记录

最后更新：2026-07-25

## 项目目标

构建一个支持 Windows 11 和 macOS 的桌面 SSH 客户端，主要能力包括：

- SSH 连接管理
- 密码和私钥认证
- 动态凭据输入及系统安全存储
- SSH 终端和多会话标签页
- 本地、远端双栏 SFTP 文件浏览
- 文件及目录上传、下载
- 传输进度、冲突处理、取消、重试和历史记录

当前代码已经达到功能较完整的 MVP 阶段，并完成单级跳板机的部分真实联调；完整认证/传输矩阵和 Windows/macOS 双平台验收仍未完成。

## 当前架构

应用采用 Electron、React 和 TypeScript：

```text
React renderer
    |
    | window.api / IPC
    v
Electron preload
    |
    v
Electron main process
    |-- SessionManager  -> ssh2 shell / SFTP
    |-- TransferManager -> 文件和目录传输队列
    `-- AppDatabase     -> node:sqlite / safeStorage
```

主要模块：

- `src/renderer/src/App.tsx`：连接、多标签会话、认证、主机指纹、冲突弹窗和传输面板
- `src/renderer/src/components/TerminalPane.tsx`：xterm 终端实例、输入、输出和尺寸同步
- `src/renderer/src/components/SftpPane.tsx`：本地/远端目录浏览、选择及拖放传输
- `src/preload/index.ts`：受控 IPC API
- `src/main/index.ts`：窗口生命周期和 IPC 注册
- `src/main/session-manager.ts`：SSH 连接、shell、SFTP 和主机指纹验证
- `src/main/transfer-manager.ts`：并发队列、递归传输、冲突处理、进度及取消
- `src/main/database.ts`：连接、凭据、known hosts、设置和传输历史
- `src/shared/types.ts`：主进程、preload 和 renderer 共用类型

## 已完成功能

### 连接与认证

- 新增、编辑、删除和搜索 SSH 连接
- 左侧连接管理支持折叠/展开，并持久化折叠状态
- 支持选择一个直连配置作为单级跳板机
- 跳板机和目标机器分别输入、保存凭据并验证主机指纹
- 通过跳板机的 `forwardOut` 通道连接目标机器，终端和 SFTP 均绑定目标机器
- 支持密码认证和 OpenSSH 私钥认证
- 私钥口令动态输入
- 使用 Electron `safeStorage` 加密保存凭据
- 支持主动清除已保存凭据
- 记录并校验服务器 SHA-256 主机指纹
- 首次连接及主机指纹变化时要求用户确认

### 会话与终端

- 多连接标签页
- 连接、断开及状态显示
- xterm 终端输入输出
- 终端尺寸变化同步到远端 PTY
- xterm.js WebGL GPU 渲染，初始化失败或上下文丢失时自动回退
- 支持全局终端字体、字号、文字颜色、背景色、背景颜色不透明度和背景图片设置，保存后不重连即可应用
- 背景图片支持覆盖、完整显示、拉伸和平铺；只持久化本地路径，主进程限制格式、文件大小和显示分辨率
- SSH 输出按 16 ms / 64 KiB 批处理，降低大量输出时的 IPC 开销
- stdout/stderr 使用流式 UTF-8 解码，避免多字节字符跨包乱码
- 应用重启后恢复已打开的连接标签，但不会自动重连

终端的数据链路、性能取舍和跨平台验收要求见 `docs/TERMINAL_ARCHITECTURE.md`。

### SFTP 与传输

- 浏览本地目录和远端目录；目录展示优先通过独立 SSH exec channel 执行结构化 `os.scandir()`，实际传输仍使用 SFTP
- 文件列表只读取名称和类型，不展示大小、修改时间；本地目录不再为每个条目并发 `lstat`，远端 exec 也不再逐项 `stat`
- 远端不支持 `python3` 或 exec 时自动回退 SFTP `readdir`
- 远端目录请求具有 12 秒超时、一次超时重试、10 秒缓存、同路径请求去重、悬停预取和过期响应隔离
- 文件夹优先排序和多选
- 通过按钮或拖放上传、下载
- 递归扫描及传输目录
- 最多三个传输任务并发运行
- 展示扫描、等待、运行、冲突、完成、失败、取消和中断状态
- 支持覆盖、跳过和自动重命名三种冲突策略
- 支持将冲突策略应用到当前批次
- 支持取消运行中的任务和重试失败任务
- 上传完成后自动刷新当前远端目录，下载完成后自动刷新当前本地目录；同批多文件完成事件合并刷新
- 传输记录持久化到 SQLite
- 应用异常退出后将未完成任务标记为中断

### 桌面与安全基础

- renderer 开启 `contextIsolation`
- 应用设置提供四种内置图标，默认使用 SSH 终端图；macOS 运行时更新 Dock，Windows 更新窗口/任务栏，安装包静态图标使用默认图
- renderer 关闭 Node 集成
- BrowserWindow 开启 sandbox
- preload 只暴露声明过的业务 API
- 阻止 renderer 任意页面跳转
- 外部 HTTP/HTTPS 链接交给系统浏览器
- 配置 Windows NSIS 和 macOS DMG/ZIP 构建目标

## 已完成验证

2026-07-25 在 macOS 开发机上完成以下静态验证：

```text
npm run typecheck  通过
npm test           通过，5 个测试文件、14 个测试用例
npm run build      通过，main/preload/renderer 均成功生成产物
npm audit --omit=dev  生产依赖 0 个已知漏洞
```

全量 `npm audit` 报告 16 个 high，均来自 `electron-builder` 使用的开发/打包依赖链（`brace-expansion`/`minimatch` 及其上游）。审计建议的自动修复会将 `electron-builder` 从 26.x 降至 25.x，属于破坏性变更，因此未执行 `npm audit fix --force`；发布工具链升级时需重新评估。

同日已执行 `npm run dev`，Electron 主进程、preload、renderer 开发服务和 macOS 桌面窗口均成功启动，启动阶段未出现运行时错误。renderer 开发服务监听 `http://localhost:5173/`。

首次启动曾出现空白窗口。根因是项目启用 `"type": "module"` 后，preload 被构建为 `index.mjs`，而 sandbox preload 将其按 CommonJS 执行，触发 `Cannot use import statement outside a module`。这会导致 `window.api` 未注入并使 React 应用崩溃。现已将 preload 明确构建为 `out/preload/index.cjs`，同时更新 BrowserWindow 加载路径，保留 `sandbox`、`contextIsolation` 和禁用 Node 集成等安全设置。修复后已通过 renderer 调试协议确认 `window.api` 存在、React 根节点已挂载且完整 UI 文本可见。

工作区已调整为“终端”和“文件传输”两个视图页签。终端默认独占主要工作区；文件传输页签集中显示本地/远端双栏 SFTP、传输队列和传输记录。终端组件在切换时保持挂载，避免丢失滚屏内容。macOS 标题栏现在通过 preload 暴露的平台信息应用专属间距，已在运行时确认交通灯区域左侧留白为 84px。

单级跳板机功能已完成真实联调：应用分别完成跳板机和目标机器的密码认证与主机指纹验证，通过跳板机 `forwardOut` 建立目标 SSH Client，最终会话进入 `connected`；在同一会话上调用 SFTP `readdir('.')` 成功返回目标机器的文件和目录。验证过程中未记录或输出两层密码。

远端文件浏览性能已定位：该服务器首次建立 SFTP subsystem/解析 home 约 6.4 秒，稳定状态下每次 SFTP `readdir` 仍约 1.75 秒，而终端 `ls` 流畅。现已改为 SSH exec + Python JSON 目录展示，并保留 SFTP 兼容回退。renderer 还增加请求竞态保护、缓存、预取和传输完成自动刷新；真实目录间连续导航的优化后延迟仍需在用户下次输入跳板机密码后复测。

终端性能优化已完成运行时探测：Electron 的 Chromium 支持 WebGL2，`@xterm/addon-webgl` 成功加载并创建 2 个 canvas，未触发回退。输出合批和 UTF-8 跨数据块解码已有自动化测试。详细设计及基准限制见 `docs/TERMINAL_ARCHITECTURE.md`。

当前单元测试只覆盖：

- 远端文件自动重命名
- Windows 本地路径自动重命名
- 连接必填字段和端口校验
- 跳板机与目标机器分别认证、`forwardOut` 目标和最终 shell 归属
- 终端输出 16 ms 合批、达到阈值立即刷新及关闭刷新
- UTF-8 多字节字符跨 SSH 数据块解码

尚未完成：

- 对已启动的 Electron 窗口进行完整 UI 走查和截图验收
- 直连、私钥、加密私钥和 keyboard-interactive 的完整真实认证矩阵
- 主机指纹首次确认及变更测试
- 真实 SFTP 上传、下载、取消和冲突测试
- Windows 实机运行及安装包测试
- macOS DMG、签名、公证和 Gatekeeper 测试

## 已知风险

### P0：真实联调前应修复

#### 1. 多个并发传输冲突可能永久卡住（已解决）

`TransferManager` 允许三个任务并发，每个任务都可能进入冲突等待；renderer 只保存一个冲突弹窗状态。多个冲突接连发生时，后来的冲突会覆盖前一个，先前任务的 Promise 将一直得不到处理。

涉及位置：

- `src/main/transfer-manager.ts`：`concurrency`、`askConflict()` 和 `conflicts`
- `src/renderer/src/App.tsx`：单一 `conflict` state

处理结果：主进程使用 FIFO 串行展示冲突，同一时间只有一个活动提示；“应用到当前批次”会自动释放已经等待及后续出现的同批冲突。取消活动冲突会继续调度下一项，批次结束后清理策略缓存。自动化测试覆盖两个并发冲突只提示一次。

#### 2. 应用重启后无法正确重试传输（新任务已解决）

传输表只保存运行时 `sessionId`。应用重启后旧会话不存在，但 `retry()` 仍复用旧 `sessionId`，因此中断任务会以“SSH 会话未连接”失败。

涉及位置：

- `src/main/database.ts`：`transfers.sessionId`
- `src/main/transfer-manager.ts`：`retry()`

处理结果：数据库 v2 migration 为传输增加 `bookmarkId`；重试时验证当前 session 属于原连接并更新 `sessionId`。renderer 会引导用户先连接对应连接。v2 以前已经存在的旧记录无法反推出连接，`bookmarkId` 为空时会明确提示无法自动重试。

### P1：跨平台和常用流程风险

#### 3. macOS 标题栏控件可能与品牌区域重叠（已解决）

macOS 交通灯坐标从 `x=16` 开始，CSS 中品牌内容同样从左侧 16px 开始。使用 `titleBarStyle: hidden` 时两者很可能重叠。

涉及位置：

- `src/main/index.ts`：`trafficLightPosition`
- `src/renderer/src/styles.css`：`.titlebar`

处理结果：preload 暴露平台信息，renderer 根节点增加平台 class，并只在 macOS 将标题栏左内边距调整为 84px。运行时样式已确认；仍需在 Intel 和 Apple Silicon 机器上截图验收。

#### 4. 本地文件浏览器在磁盘根目录的“上级目录”计算错误（已解决）

当前字符串算法会将 macOS `/` 计算为空字符串，将 Windows `C:\` 计算为 `C:`。继续读取时可能报错或进入非预期目录，对 UNC 路径的支持也不可靠。

涉及位置：`src/renderer/src/components/SftpPane.tsx` 的 `parentPath()`。

处理结果：本地父路径已移到主进程，按平台使用 `path.posix`/`path.win32`，并限制在 `path.parse().root` 内。测试覆盖 `/`、`C:\` 和 UNC 共享根目录。

#### 5. 后台标签连接成功后可能不加载远端目录（已解决）

连接完成时如果标签不可见，SFTP effect 不会读取远端目录；之后仅切换 `visible` 也不会触发 effect，用户会看到空列表直到手动刷新。

涉及位置：`src/renderer/src/components/SftpPane.tsx` 的远端目录加载 effect。

处理结果：远端目录加载 effect 已监听 `visible`，切换到文件传输页签时会触发加载。

#### 6. 快速重复连接会产生孤立会话（已解决）

连接请求返回前，现有标签的状态和 `sessionId` 尚未完全更新。用户快速重复点击时可能创建多个 SSH Client，标签最终只关联其中一个。断线及失败的会话也不会从 `SessionManager.sessions` 删除。

涉及位置：

- `src/renderer/src/App.tsx`：`connect()`
- `src/main/session-manager.ts`：`connect()`、`disconnect()` 和 `close` 事件

处理结果：主进程按连接复用正在建立或已连接的 session，renderer 在请求前立即进入 `connecting`。断开、失败和跳板机转发异常统一释放 Map、host-key callback、shell、SFTP、目标 Client 和跳板 Client。

#### 7. 并发创建远端目录存在竞态（已解决）

多个上传任务创建相同远端目录时，可能同时确认目录不存在并执行 `mkdir`。其中一个成功后，另一个可能因“已存在”而将整个任务标记为失败。

涉及位置：`src/main/transfer-manager.ts` 的 `ensureRemoteDirectory()`。

处理结果：`mkdir` 失败后重新 `lstat`，如果其他任务已经创建同名目录则视为成功；目标不是目录时仍返回原错误。

#### 8. 部分 SFTP 服务不接受 `/` 作为初始目录（已解决）

真实连接中观察到服务器对 `readdir('/')` 返回 `No such file`。当前界面将远端初始路径固定为 `/`，没有查询 SFTP 会话实际 home 目录，因此在 chroot、虚拟根目录或受限 SFTP 服务上可能无法显示初始文件列表。

涉及位置：`src/renderer/src/components/SftpPane.tsx` 的 `remotePath` 初始值，以及 `SessionManager` 缺少 `realpath('.')` 查询。

处理结果：优先通过 SSH exec 读取 `$HOME`；受限 shell 下回退 SFTP `realpath('.')`，renderer 使用返回路径作为初始目录，最终失败时才尝试 `/`。

### P2：可靠性和产品完整度

#### 9. 跳过冲突后完成任务的进度可能不足 100%（已解决）

`bytesTotal` 包含所有源文件，但跳过的文件不会增加 `bytesTransferred`。任务最终会显示已完成，但进度可能低于 100%。

处理结果：跳过文件时将该文件大小计入已处理字节，任务完成进度与总大小一致。

#### 10. 覆盖传输失败可能留下损坏文件（文件传输已解决）

上传和下载都直接以 `w` 打开目标文件。传输中断时，原文件可能已被截断，并留下不完整目标。

处理结果：上传和下载先写目标同目录的唯一 `.part` 文件，传输成功后再提交；失败或取消会清理临时文件。OpenSSH 服务优先使用 POSIX rename，其他 SFTP 服务使用备份、重命名和失败恢复流程。自动化测试确认中途失败时本地旧文件保持不变。目录作为多个文件的集合仍不是整体事务，失败时可能留下此前已完成的文件。

#### 11. 符号链接会被静默跳过（已解决静默行为）

扫描阶段将符号链接计为 0 字节，传输阶段直接跳过，但任务仍可能显示完成，用户无法得知链接没有传输。

处理结果：当前产品策略仍是不传输符号链接，但扫描阶段会明确将任务标记为失败并显示对应路径，不再以“已完成”静默跳过。

#### 12. 凭据在认证成功前保存（已解决）

勾选“记住凭据”后，凭据会在 SSH 认证结果确认前写入数据库。输入错误密码可能覆盖此前正确凭据。

处理结果：跳板机和目标机器的凭据均只在各自 SSH Client 触发 `ready` 后保存，错误密码不会提前覆盖已保存凭据。

#### 13. 数据库没有 schema migration 机制（已建立基础机制）

项目此前只使用 `CREATE TABLE IF NOT EXISTS`。增加跳板机字段时已引入 `PRAGMA user_version` migration；传输连接 ID 使用版本 2 migration，并有从 v1 数据库升级及读写新字段的自动化测试。

后续要求：所有 schema 变更继续使用顺序 migration，不得假设用户数据库为空。

#### 14. 自动化覆盖不足

自动化覆盖已增加到 6 个测试文件、16 个用例，覆盖跳板连接、SFTP 单例初始化、重复连接、host-key 定时器隔离、传输冲突批次、失败覆盖恢复、数据库 v2 migration、路径、终端输出和终端设置校验。但真实网络错误和跨平台行为仍有覆盖缺口。

优先补充：

- Session 状态机和重复连接测试
- host key 接受、拒绝、变化及超时测试
- 传输冲突队列和批次策略测试
- 取消、断线、部分写入和重试测试
- 数据库 migration 测试
- macOS、Windows 路径测试

#### 15. 发布签名尚未配置

当前有打包目标，但没有 Windows 代码签名及 macOS Developer ID、公证配置。正式分发时会遇到 SmartScreen 或 Gatekeeper 提示。

#### 16. 开发/打包依赖存在审计告警

全量 `npm audit` 报告 16 个 high，集中在 `electron-builder` 的归档、glob 和 Windows 安装器依赖链；`npm audit --omit=dev` 为 0。当前自动强制修复会降级主版本工具链，不能在没有打包回归的情况下直接采用。

建议：跟踪 `electron-builder` 上游修复版本，升级后重新执行 macOS DMG/ZIP 与 Windows NSIS 打包验收。在此之前不要使用不可信输入驱动本地打包脚本。

#### 17. SSH-exec 首次目录读取失败会错误地永久回退 SFTP（已解决）

`listRemote()` 在尚未确定模式时捕获所有 exec 错误并将整个 session 固定为 SFTP。路径不存在、权限不足或瞬时超时并不代表服务器不支持 `python3`/exec，却会让后续目录浏览重新承受已测得约 1.75 秒一次的 SFTP 延迟。

处理结果：只有 exec channel 不可用或退出码 127 才固定使用 SFTP；超时只对当次请求回退，路径、权限和脚本错误直接返回，不再改变 session 模式。

#### 18. SFTP 初始化并发和操作超时不足（已解决）

多个浏览或传输任务首次同时调用 `getSftp()` 时会分别创建 subsystem，后完成的 wrapper 覆盖先完成者。传输使用的 `lstat`、`readdir`、`mkdir` 等 Promise 没有本地超时，服务端不回调时任务可长期停留在扫描或运行状态。

处理结果：并发调用共享同一个 in-flight SFTP Promise，12 秒初始化超时后的迟到 wrapper 会立即关闭。传输元数据操作使用 20 秒超时，文件流 60 秒无进展会销毁读写流并失败。

#### 19. 目录扫描请求没有并发上限（已解决）

上传和下载的大小扫描递归使用 `Promise.all`。大目录会在单个任务内并发发出大量本地 stat 或 SFTP 请求，`TransferManager` 的三个任务并发限制并不能限制这些子请求。

处理结果：递归大小扫描改为顺序遍历，所有传输任务共享最多 8 个并发远端元数据操作的 semaphore。

#### 20. 主机指纹提示存在覆盖和定时器串扰（已解决）

renderer 只保存一个主机指纹弹窗，并发连接陌生主机时后一个提示会覆盖前一个。主进程的 60 秒定时器只按 `sessionId` 查找当前 callback；跳板机提示被接受后，它的旧定时器可能误拒绝同一 session 后续的目标机提示。

处理结果：renderer 使用 FIFO 保存并发提示；主进程定时器只在 Map 中仍是其创建的同一 pending 对象时才拒绝。测试覆盖旧跳板机定时器不会拒绝后续目标机提示。

#### 21. renderer 缺少 Content Security Policy（已解决）

当前 HTML 没有 CSP，Electron 开发运行时会输出安全警告。虽然已启用 sandbox、context isolation 并关闭 Node 集成，但一旦未来出现内容注入，缺少 CSP 会扩大 renderer 可执行脚本和联网范围。

处理结果：HTML 已限制默认、脚本、样式、图片、字体和连接来源；不允许 `unsafe-eval`，开发环境只额外允许本机 Vite WebSocket。重启后的 Electron 不再输出 CSP 安全警告。

#### 22. 终端高吞吐没有端到端背压（已解决基础流控）

WebGL 和 16 ms IPC 合批降低了正常输出成本，但 ssh2 stream 不会依据 xterm.js 写队列的消费速度暂停。远端持续高速输出时，renderer 待解析数据仍可能增长并造成长时间占用。

处理结果：xterm 待解析内容达到 1 MiB 时暂停对应 SSH shell，回落至 256 KiB 时恢复，组件销毁也会解除暂停。真实持续输出基准和 Windows/macOS 长时间稳定性仍需验收。

## 建议优化顺序

1. 使用真实跳板连接复测 SSH-exec 目录延迟、上传/下载、覆盖恢复和自动刷新。
2. 在 macOS 走查冲突队列、主机指纹队列、终端背压和窗口布局。
3. 使用可控 SSH 测试服务器覆盖密码、私钥、host key、取消和断线恢复。
4. 在 Windows 11 验证盘符、UNC、原子替换、多选拖放和 NSIS 安装。
5. 升级存在审计告警的打包依赖，并完成签名、公证、安装升级测试。

## 下一次继续开发时的检查清单

- 阅读本文件并确认风险优先级是否变化。
- 执行 `npm ci`。
- 执行 `npm run typecheck`、`npm test` 和 `npm run build`。
- 检查 `git status`，避免覆盖其他机器上尚未提交的修改。
- 修改数据库前先设计 migration，不直接假设用户数据库为空。
- 修复核心竞态后再开始大规模 UI 调整。

## 环境备注

- 当前开发机通过 `fnm` 安装了 Node.js `v24.18.0`，但默认 shell 没有自动加载 Node；命令可通过 `fnm exec` 运行。
- 本次 `npm ci` 使用 npm `11.16.0`，依赖安装成功。
- Electron 运行时已在首次启动时成功下载，`npm run dev` 可以正常打开 macOS 桌面窗口。npm 仍提示 SSH 等部分依赖存在尚未显式放行的可选安装脚本；当前运行使用其可用实现，真实 SSH 联调时需要继续观察。
- `node_modules/` 和 `out/` 已在 `.gitignore` 中，不属于应提交内容。
