# 终端架构与性能设计

最后更新：2026-07-25

## 目标与边界

应用内终端需要同时支持 Windows 和 macOS，并复用应用已经建立的 SSH、跳板机、主机指纹及双层凭据认证。终端只负责交互显示；远端 shell 和命令仍运行在目标服务器。

不直接嵌入 Terminal.app、Ghostty、Windows Terminal 等外部应用。它们没有统一、稳定的 Electron 嵌入接口，强行嵌入原生窗口会引入两套平台代码，并造成焦点、输入法、缩放、GPU 上下文和会话复用问题。未来可独立增加“在外部终端打开”，但它属于新的系统 SSH 会话，不能假定复用应用内密码或现有 SSH channel。

## 数据链路

```text
远端 PTY (xterm-256color)
    -> ssh2 ClientChannel
    -> UTF-8 StringDecoder
    -> 16 ms / 64 KiB 输出批次
    -> Electron IPC app:event
    -> xterm.js parser/write queue
    -> WebGL renderer
       `-> WebGL 不可用或 context lost 时回退默认 DOM renderer
```

输入方向保持低延迟：`xterm.onData` 经单向 IPC 直接写入 ssh2 shell。窗口变化由 `FitAddon` 计算行列数，再调用远端 PTY `setWindow`。

## 已实现优化

### WebGL 渲染

renderer 使用官方 `@xterm/addon-webgl`。加载成功后由 Chromium 使用 GPU 合成；Windows 通常经 ANGLE 映射到 Direct3D，macOS 由 Chromium 映射到系统图形后端。

WebGL 初始化失败不能阻止终端打开。插件抛错时保留 xterm 默认 renderer；运行中发生 context loss 时释放 WebGL 插件并回退。终端容器的 `data-renderer` 会记录 `webgl` 或 `dom`，用于调试协议和自动化验收，不显示给用户。

### SSH 输出批处理

此前每个 ssh2 `data` 事件都会产生一次 Electron IPC。大量日志输出时，小数据块会造成主进程、序列化和 renderer 事件循环开销。

现在默认累计 16 ms 后合并发送；累计内容达到 64 KiB 时立即发送，限制延迟和内存占用。交互命令的额外等待上限约为一帧，高吞吐输出则显著减少 IPC 次数。xterm.js 自己的 `write()` 队列继续负责解析端背压，不由 React state 承载终端内容。

### UTF-8 流式解码

不能对每个 SSH Buffer 单独调用 `toString('utf8')`，因为一个中文字符可能跨越两个网络数据块。主进程为 stdout 和 stderr 分别维护 Node `StringDecoder`，只在字节完整后输出字符；会话关闭时结束 decoder 并刷新最后一个批次。

### 高水位背压

renderer 统计已交给 `term.write()` 但尚未完成解析的数据量。待处理内容达到 1 MiB 时通过 IPC 暂停对应 ssh2 shell，回落到 256 KiB 后恢复；组件销毁时也会解除暂停，避免 session 永久停住。这形成从 xterm 写队列到 SSH channel 的高低水位流控。

### 外观设置与背景图

终端外观设置全局保存到 SQLite，包括字体族、10–30 px 字号、文字颜色、背景颜色、背景颜色不透明度、背景图路径和图片显示方式。修改保存后直接更新现有 xterm 实例的 `fontFamily`、`fontSize` 和 `theme`，随后重新执行 `FitAddon.fit()`，不关闭 SSH channel，也不丢失终端滚屏内容。

xterm 启用 `allowTransparency`，背景颜色使用 RGBA 绘制；背景图片放在 xterm canvas 下方的终端容器上，因此文字和光标保持不透明。WebGL addon 支持透明纹理，初始化失败时仍沿用现有 DOM renderer 回退策略。这里的透明度表示背景颜色遮罩的透明度，不会让整个 Electron 窗口透出桌面。

数据库只保存本地图片路径。主进程通过受控 IPC 读取 PNG、JPEG、WebP、BMP 或 GIF，拒绝超过 25 MB 的文件，并将显示副本限制在 3840×2160 内后转成 data URL 交给 sandbox renderer。原图移动或删除后设置仍保留，但启动时不再显示图片；应用不会把原图复制进用户数据目录。

## 性能预期与验收

常规 shell、vim、tmux、top 等交互应维持低延迟。WebGL 主要改善快速滚屏及高频全屏重绘；IPC 批处理主要改善大量短数据块。Electron 和 JavaScript 解析仍有固定成本，不承诺达到 Ghostty 等原生终端的极限吞吐。

发布前在 Windows 11 和 macOS 分别验证：

- `data-renderer` 首选值为 `webgl`，禁用 GPU 时能回退为 `dom`。
- 快速连续输入没有可感知延迟，窗口缩放后的行列数正确。
- 中文输出在刻意拆分 UTF-8 字节后不出现替换字符。
- 连续日志和全屏程序运行时，renderer 不冻结且内存保持稳定。
- GPU context loss 后终端仍可继续输入和显示。

性能测试应记录相同机器、窗口尺寸和输入数据下的总耗时、主进程 CPU、renderer CPU、峰值内存及 IPC 事件数量，比较默认 renderer、WebGL 以及启用输出批处理前后的结果。

2026-07-25 在 macOS Electron 开发实例中完成运行时探测：Chromium 可创建 WebGL2 context，`WebglAddon` 成功加载并创建 2 个 canvas，未触发默认 renderer 回退。一次性向 xterm 写入约 1.8 MB 字符串会长时间占用 renderer；这种调用绕过真实 SSH 分块和本实现的主进程合批，不能作为有效吞吐结论。后续基准必须分块写入、模拟实际到达节奏，并为单轮测试设置取消和超时。

## 后续增强

- 增加终端搜索、链接识别、Unicode 11、预设主题和快捷键设置。
- 增加可重复的吞吐基准工具，避免使用无限输出命令导致不可控负载。
- 将“在外部终端打开”作为独立功能设计，优先支持私钥和 SSH config/ProxyJump，不在命令行参数中传递密码。
