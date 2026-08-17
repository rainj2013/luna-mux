# 终端架构

## 数据流

```text
Terminal Runtime
  -> backend (SSH russh / local portable-pty)
  -> Rust UTF-8 增量解码与 runtime event
  -> Tauri `terminal-runtime:event`
  -> React TerminalPane
  -> xterm.js
```

应用不嵌入 Terminal.app、Ghostty 或 Windows Terminal。每个终端 Pane 对应一个独立的 Terminal Runtime；当前 SSH 实现由一条 Rust SSH connection 和 PTY channel 承载，因此断开其中一个不会影响其他 Pane。

## PTY 与终端运行时

Rust 后端通过 `russh` 请求 `xterm-256color` PTY，并将窗口尺寸变化直接发送给远端。连接支持密码、键盘交互、私钥、SSH Agent、主机密钥确认、一级跳板机和可配置保活。

本地 PTY 采用 `portable-pty` 0.9：Windows 使用 ConPTY，macOS 使用 Unix PTY。M2.1 的 Windows 探针已验证 Unicode 输出、缩放、Ctrl-C 输入和进程清理；macOS 仍需在目标平台验证。ConPTY 启动时可能发送 `ESC[6n` 光标位置查询，正式的 xterm.js `TerminalPane` 会负责响应，PTY 后端不自行实现终端仿真。

在 Luna Mux 领域模型中，Mux Session 表示持久化的项目容器，Terminal Runtime 表示窗格中的一次终端运行。应用退出时会停止当前端口转发并关闭全部 Runtime；Mux Session 和窗格配置保留。运行期间，Runtime 由 Rust 后端持有，不依赖前端组件是否重新渲染。

## 输出与背压

SSH 和本地 PTY 数据块在 Rust 端使用增量 UTF-8 解码器处理，跨数据块的多字节字符不会被截断。Runtime 输出使用有界环形缓冲区和 UTF-8 字节游标；`TerminalPane` 挂载时可从游标增量追赶。xterm 写入队列积压时，前端通过流控暂停后端读取；积压恢复后再继续，避免大量输出长期占用 WebView 事件循环。

运行中的 Pane 会跨 Session 和 Session 内视图切换保持同一个 xterm 实例，非当前工作区只隐藏并停用 WebGL 渲染。布局重排等确实需要卸载组件的情况使用 xterm 官方序列化格式暂存正常屏幕、备用屏幕和滚动缓冲区，并从最后完成渲染的 UTF-8 字节游标继续追赶；不能只重放有界 PTY 原始输出，因为全屏 TUI 的清屏和光标控制序列无法重建已经丢失的滚动历史。删除 Pane 或 Session 时同步丢弃对应快照。

终端启用 WebGL 插件；初始化失败时自动使用 xterm 默认渲染器。透明背景由 xterm 的透明画布与终端容器背景图组合实现，不改变整个系统窗口透明度。

## 安全边界

WebView 只能通过 Tauri 命令调用显式注册的 Rust 能力。外部链接仅允许 HTTP/HTTPS，文件选择使用原生对话框，凭据保存在 macOS 钥匙串或 Windows 凭据管理器。终端和文件传输不启动 Node.js 子进程或辅助进程。
