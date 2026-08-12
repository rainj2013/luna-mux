# 终端架构

## 数据流

```text
远端 PTY
  -> russh channel
  -> Rust UTF-8 增量解码
  -> Tauri app:event
  -> React TerminalPane
  -> xterm.js
```

应用不嵌入 Terminal.app、Ghostty 或 Windows Terminal。每个终端 Pane 对应一个独立的 Terminal Runtime；当前 SSH 实现由一条 Rust SSH connection 和 PTY channel 承载，因此断开其中一个不会影响其他 Pane。

## PTY 与 Terminal Runtime

Rust 后端通过 `russh` 请求 `xterm-256color` PTY，并将窗口尺寸变化直接发送给远端。连接支持密码、键盘交互、私钥、SSH Agent、Host Key 确认、一级跳板机和可配置 keepalive。

在 Luna Mux 领域模型中，Mux Session 表示持久化的项目容器，Terminal Runtime 表示 Pane 中的一次终端运行。应用退出时会停止当前端口转发并关闭全部 Runtime；Mux Session 和 Pane 配置保留。运行期间，Runtime 由 Rust 后端持有，不依赖前端组件是否重新渲染。

## 输出与背压

SSH 数据块在 Rust 端使用增量 UTF-8 解码器处理，跨数据块的多字节字符不会被截断。xterm 写入队列积压时，前端通过 flow 控制暂停后端读取；积压恢复后再继续，避免大量输出长期占用 WebView 事件循环。

终端启用 WebGL addon；初始化失败时自动使用 xterm 默认 renderer。透明背景由 xterm 的透明画布与终端容器背景图组合实现，不改变整个系统窗口透明度。

## 安全边界

WebView 只能通过 Tauri command 调用显式注册的 Rust 能力。外部链接仅允许 HTTP/HTTPS，文件选择使用原生对话框，凭据保存在 macOS Keychain 或 Windows Credential Manager。终端和文件传输不启动 Node.js 子进程或 sidecar。
