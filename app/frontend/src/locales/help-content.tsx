import type { ReactNode } from 'react'
import { BookOpen, FolderOpen, Globe2, LayoutGrid, Network, Rocket, Server, Settings, ShieldCheck, Sparkles, SquareTerminal, TriangleAlert, WandSparkles, type LucideIcon } from 'lucide-react'
import { PRODUCT_INFO } from '../product-info'

export interface HelpSection {
  id: string
  group: string
  title: string
  icon: LucideIcon
  searchText: string
  content: ReactNode
}

export function createChineseHelpSections(commandKey: string): HelpSection[] {
  return [
    {
      id: 'start', group: '开始使用', title: '快速开始', icon: BookOpen,
      searchText: '快速开始 第一次 项目 会话 窗格 终端 agent 浏览器 ssh 工作区',
      content: <>
        <h2>快速开始</h2>
        <p>{PRODUCT_INFO.displayName} 以项目会话组织终端、Coding Agent、SSH 连接和受管浏览器。一个会话会保存项目根目录、窗格定义、拆分布局和浏览器配置，让同一个项目的工作环境可以持续使用。</p>
        <h3>第一次使用</h3>
        <ol>
          <li>点击侧栏顶部的加号创建项目会话，填写名称，并按需选择项目根目录。</li>
          <li>在会话行点击加号添加窗格，选择本机 Shell、WSL、SSH 目标或 Coding Agent 启动配置。</li>
          <li>在窗格中运行命令，或让 Agent 执行任务；需要并行上下文时，继续添加窗格或拆分当前窗格。</li>
          <li>Agent 验证网页时会使用当前会话的受管浏览器，Luna Mux 会按需准备 Chrome 环境。</li>
        </ol>
        <h3>顶部视图</h3>
        <table className="help-detail-table"><tbody>
          <tr><th scope="row">窗格</th><td>显示本地终端、SSH 终端和 Agent 终端组成的拆分工作区，是日常操作的主视图。</td></tr>
          <tr><th scope="row">Agent</th><td>查看当前活跃 Agent 的所属窗格、目标环境、Hook、Luna MCP 和 Browser MCP 状态。</td></tr>
          <tr><th scope="row">浏览器</th><td>启动、聚焦、重启或停止当前会话的受管 Chrome，并查看 Profile、进程和启动诊断。</td></tr>
          <tr><th scope="row">文件</th><td>聚焦已连接 SSH 窗格后出现，用于浏览本地和远端目录、传输文件和打开部署工具。</td></tr>
        </tbody></table>
      </>
    },
    {
      id: 'sessions', group: '开始使用', title: '会话与侧栏', icon: LayoutGrid,
      searchText: '会话 session 项目 根目录 侧栏 展开 折叠 切换 重命名 删除 右键 宽度 窗格 提醒 状态',
      content: <>
        <h2>项目会话与侧栏</h2>
        <p>会话是一个项目的工作容器。侧栏用于创建、切换、展开、编辑会话，并快速定位会话里的窗格。</p>
        <h3>会话保存内容</h3>
        <ul>
          <li><strong>名称和项目根目录：</strong>名称显示在侧栏；项目根目录会作为新建本地终端和本地 Agent 的初始工作目录。</li>
          <li><strong>窗格和布局：</strong>窗格名称、目标、Agent 启动类型、拆分方向和比例会随会话保存，应用重启后恢复工作区结构。</li>
          <li><strong>浏览器环境：</strong>每个会话自动配备一个独立 Browser Resource 和持久化 Profile，用于同会话 Agent 的网页验证。</li>
        </ul>
        <h3>侧栏操作</h3>
        <ul>
          <li>点击会话可切换、展开或折叠；多个会话可以同时保持展开。</li>
          <li>点击窗格会切换到它所属的会话，聚焦该窗格，并回到“窗格”视图。</li>
          <li>会话行加号用于添加窗格；右键会话可编辑或删除；右键窗格可重命名；窗格行末按钮会关闭并移除该窗格。</li>
          <li>拖动侧栏右边缘调整宽度，双击边缘恢复默认宽度；工具栏左侧按钮可折叠整个侧栏。</li>
        </ul>
        <h3>状态和提醒</h3>
        <ul>
          <li>窗格名称前的圆点显示终端连接状态：绿色为已连接，闪烁黄色为连接中，红色为错误，灰色为未运行。</li>
          <li>Agent 需要确认或输入时，窗格图标与名称使用警示色，同时终端面板出现橙色边框；错误使用红色，未读完成事件使用蓝色。</li>
          <li>进入窗格会标记完成或错误事件为已读；权限确认、输入和中断等实际操作会推进仍在等待的 Agent 状态。</li>
        </ul>
        <div className="help-warning"><TriangleAlert size={15} aria-hidden="true" /><div><strong>删除会话</strong><p>删除会话会关闭其中的终端、Agent 和受管浏览器，并删除窗格与布局定义。已保存的 SSH 目标会继续保留在资源库中。</p></div></div>
      </>
    },
    {
      id: 'terminal', group: '开始使用', title: '窗格与终端', icon: SquareTerminal,
      searchText: '窗格 pane 终端 本地 powershell pwsh wsl macos ssh 分屏 拆分 布局 最大化 重启 搜索 复制 粘贴 快捷键 背景图',
      content: <>
        <h2>窗格、布局与终端</h2>
        <p>窗格承载一个本地、WSL、SSH 或 Agent 运行时。拆分布局让同一会话中的多个上下文并排工作。</p>
        <h3>添加窗格</h3>
        <ol>
          <li>在会话行点击加号，按需填写窗格名称。</li>
          <li>选择普通“终端”或一个 Coding Agent 启动配置。</li>
          <li>选择运行目标：Windows 提供 PowerShell 7 和已安装的 WSL 发行版；macOS 使用登录 Shell；远程目标来自 SSH 目标资源库。</li>
        </ol>
        <p>普通终端里也可以手动运行 <code>codex</code> 或 <code>claude</code>，Luna Mux 会在检测到活动后把它识别为 Agent。</p>
        <h3>拆分和布局</h3>
        <ul>
          <li>窗格标题栏的左右拆分和上下拆分按钮会复制当前目标定义，创建一个新的同类窗格。</li>
          <li>拖动窗格之间的分隔线调整比例，比例会随会话保存。</li>
          <li>最大化按钮临时只显示一个窗格，再次点击恢复完整布局。</li>
          <li>两个以上窗格时，顶部布局菜单可改为横向、纵向或每行两列排列。</li>
          <li>重启会关闭当前运行时并按原目标重新启动；关闭窗格会同时移除它的持久化定义。</li>
        </ul>
        <h3>终端操作</h3>
        <ul>
          <li>拖选文本后按 <kbd>{commandKey}+C</kbd> 复制；按 <kbd>{commandKey}+V</kbd> 粘贴。没有选区时，Ctrl+C 会发送给终端进程。</li>
          <li>按 <kbd>{commandKey}+F</kbd> 搜索当前终端滚屏，使用上下按钮切换结果，按 Esc 关闭。</li>
          <li>按住 <kbd>{commandKey}</kbd> 并左键点击或直接双击终端中的 HTTP/HTTPS 链接，可用系统默认浏览器打开。</li>
          <li>本地终端停止或 SSH 断开后，空状态按钮可重新启动或重连。</li>
        </ul>
        <h3>常用快捷键</h3>
        <table className="help-shortcut-table"><tbody>
          <tr><th scope="row">添加窗格</th><td><kbd>{commandKey}+Shift+T</kbd></td></tr>
          <tr><th scope="row">向右拆分</th><td><kbd>{commandKey}+Shift+D</kbd></td></tr>
          <tr><th scope="row">向下拆分</th><td><kbd>{commandKey}+Alt+Shift+D</kbd></td></tr>
          <tr><th scope="row">重启当前窗格</th><td><kbd>{commandKey}+Shift+R</kbd></td></tr>
          <tr><th scope="row">关闭当前窗格</th><td><kbd>{commandKey}+W</kbd></td></tr>
          <tr><th scope="row">切换窗格</th><td><kbd>Ctrl+Tab</kbd> / <kbd>Ctrl+Shift+Tab</kbd></td></tr>
          <tr><th scope="row">打开帮助</th><td><kbd>F1</kbd></td></tr>
        </tbody></table>
      </>
    },
    {
      id: 'agents', group: 'Agent 与自动化', title: 'Agent 工作流', icon: Sparkles,
      searchText: 'agent codex claude code 启动 手动 hook mcp luna browser 状态 提醒 等待 授权 环境 集成 adapter 多窗格 协作 输出 指令 bash 远程 agent ssh 注入',
      content: <>
        <h2>Coding Agent 工作流</h2>
        <p>Luna Mux 把 Codex 和 Claude Code 放进会话窗格中运行，并为每个运行时注入 Hook、Luna MCP 和 Browser MCP 配置。这样 Agent 可以感知同会话窗格、请求确认、读取受管浏览器状态，并把等待或完成状态反馈到界面。</p>
        <h3>启动方式</h3>
        <ol>
          <li>创建终端窗格，或在添加窗格时选择 Coding Agent 启动配置。</li>
          <li>在窗格中运行 <code>codex</code> 或 <code>claude</code>；受管启动会直接进入对应 Agent，普通终端也会在活动出现后识别。</li>
          <li>Agent 退出后，重新启动会建立新的运行时身份，状态和授权会重新绑定到当前窗格。</li>
        </ol>
        <h3>提醒如何工作</h3>
        <ul>
          <li>Hook 报告输入、权限确认、完成或错误时，侧栏、窗格名称和终端边框会同步显示提醒。</li>
          <li>橙色表示需要介入，红色表示错误，蓝色表示未读完成信息；终端连接圆点继续显示连接状态。</li>
          <li>进入窗格可阅读最新事件；提交输入、确认权限、按 Esc 或 Ctrl+C 会推动 Agent 继续运行或中断。</li>
        </ul>
        <h3>同会话多 Agent 协作</h3>
        <p>项目会话是协作边界。同一会话中的 Agent 可以发现会话内所有窗格和当前运行时，并通过 Luna MCP 与其他终端或 Agent 协作。</p>
        <ul>
          <li><code>terminal.runtimes.list</code> 返回终端运行时和所属窗格，目标可以是 Agent，也可以是普通 Bash、PowerShell 或 SSH Shell。</li>
          <li><code>terminal.runtime.output.read</code> 按游标增量读取有界输出；<code>terminal.runtime.write</code> 向目标 PTY 写入文本或命令。</li>
          <li><code>agents.list</code> 和 <code>agents.get_status</code> 提供结构化 Agent 状态；<code>agents.send_task</code> 向目标 Agent 发送任务文本。</li>
          <li>读取输出、写入 PTY、发送任务和中断前台进程使用 Luna Mux 的会话级控制授权；关闭 Runtime 等高影响动作会按操作策略确认。</li>
        </ul>
        <h3>Agent 视图</h3>
        <p>“Agent”页用于查看当前会话的 Agent 运行环境和集成状态，适合排查 Hook、MCP、目标环境和浏览器能力。</p>
        <ul>
          <li>“会话环境”显示项目根目录，以及 Browser MCP 当前是否已就绪或可按需启动。</li>
          <li>“活跃 Agent 运行环境”显示每个 Agent 的适配器、所属窗格、启动方式和本地/SSH 目标。</li>
          <li>Hook 和 Luna MCP 列显示结构化集成是否已经连接和配置。</li>
          <li>筛选框可按 Agent、窗格或目标快速定位活跃运行时；输入框可向选中的 Agent 发送文本。</li>
        </ul>
        <h3>运行时集成</h3>
        <p>Luna Mux 按窗格运行时写入临时 Hook、Luna MCP 与 Browser MCP 环境变量。配置随运行时生效，并在运行时结束后失效。</p>
        <p>远程 Agent 集成通过“设置 → SSH → 远程 Agent 集成”开启。开启后，新建或重启的 SSH 窗格会在远端探测 Codex、Claude Code 和网络工具，上传运行时 helper 到 <code>~/.luna-mux/runtime/&lt;runtime-id&gt;</code>，临时调整当前 Shell 的 PATH，并建立绑定远端回环地址的反向转发。</p>
        <p>远程 Browser MCP 会通过当前 SSH 连接把 MCP 请求送回本机，并操作本机会话的 Browser Resource。Hook 转发需要 curl 或 wget；Browser MCP 需要 socat、nc/ncat 或 bash TCP 支持。正常断开时，运行时目录会通过 SFTP 清理。</p>
      </>
    },
    {
      id: 'browser', group: 'Agent 与自动化', title: '浏览器自动化', icon: Globe2,
      searchText: '浏览器 browser chrome cdp profile agent-browser mcp 自动 按需 启动 停止 重启 打开 tab 页面 可用 agent 退出 profile 保留',
      content: <>
        <h2>受管浏览器与网页验证</h2>
        <p>每个项目会话自动拥有一个 Browser Resource。它为同会话 Agent 提供独立 Chrome 进程、持久化 Profile 和 agent-browser MCP 连接。</p>
        <h3>自动使用</h3>
        <ol>
          <li>Agent 第一次调用注入的 <code>agent_browser</code> 工具时，Luna Mux 会解析当前会话的 Browser Resource。</li>
          <li>Chrome 处于停止状态时，Luna Mux 会按需启动独立 Chrome 进程并连接 CDP。</li>
          <li>普通导航复用当前绑定页面；需要并行页面上下文时，Agent 可以创建额外标签页。</li>
          <li>停止浏览器或退出 Luna Mux 会关闭 Chrome 进程；登录状态和站点数据保留在该会话的独立 Profile 中。</li>
        </ol>
        <h3>浏览器页操作</h3>
        <table className="help-detail-table"><tbody>
          <tr><th scope="row">启动</th><td>立即启动当前会话的 Chrome，适合先登录网站或预热验证环境。</td></tr>
          <tr><th scope="row">打开</th><td>把已运行的受管 Chrome 窗口切到前台。</td></tr>
          <tr><th scope="row">重启</th><td>关闭当前进程后，使用同一个 Profile 和会话 CDP 配置重新启动。</td></tr>
          <tr><th scope="row">停止</th><td>关闭 Chrome 进程，并保留会话 Profile 供下次启动使用。</td></tr>
        </tbody></table>
        <p>诊断区显示运行状态、PID、CDP 地址、Profile 路径和启动错误。安装 Chrome 后，可点击重新检测刷新可用状态。</p>
        <div className="help-warning"><TriangleAlert size={15} aria-hidden="true" /><div><strong>Agent 浏览器自动化</strong><p>Agent 可以通过注入的 <code>agent_browser</code> 工具按需启动并操作当前会话的受管 Chrome；“浏览器”页提供同一个 Browser Resource 的人工启动、聚焦、重启和诊断入口。</p></div></div>
      </>
    },
    {
      id: 'connections', group: 'SSH', title: 'SSH 目标与认证', icon: Server,
      searchText: 'ssh 目标 连接 资源库 密码 私钥 agent 跳板机 分组 收藏 排序 搜索 keepalive 保活 config 导入 导出 luna remote 凭据 指纹',
      content: <>
        <h2>SSH 目标与认证</h2>
        <p>SSH 目标是创建远程窗格时复用的连接配置。可在“添加窗格”中直接选择目标，也可进入侧栏底部的 SSH 目标资源库集中维护。</p>
        <h3>资源库操作</h3>
        <ul>
          <li>双击目标会把它作为新窗格加入当前会话；单击会选中目标并显示详情。</li>
          <li>右键目标可编辑、复制或删除；目标和分组可拖动排序，也可把目标拖入其他分组。</li>
          <li>收藏目标会显示星标；搜索匹配名称、主机、用户名、分组和备注。</li>
          <li>可导入 OpenSSH Config、Luna Mux 连接备份或 Luna Remote 数据库，也可导出连接备份。连接备份包含目标和分组信息。</li>
        </ul>
        <h3>认证方式</h3>
        <table className="help-detail-table"><tbody>
          <tr><th scope="row">密码</th><td>连接时输入服务器密码，也支持 keyboard-interactive；勾选记住后保存到系统安全存储。</td></tr>
          <tr><th scope="row">私钥</th><td>选择 OpenSSH 私钥文件；加密私钥会提示口令，口令可单独保存到系统安全存储。</td></tr>
          <tr><th scope="row">SSH Agent</th><td>使用 Luna Mux 启动时继承的 Agent。macOS/Linux 依赖有效的 <code>SSH_AUTH_SOCK</code>，Windows 使用 OpenSSH Authentication Agent。</td></tr>
          <tr><th scope="row">跳板机</th><td>先连接一个已保存的直连目标，再由它连接最终主机。当前支持单级跳板，两个目标分别认证。</td></tr>
        </tbody></table>
        <h3>首次连接和保活</h3>
        <p>首次连接会显示服务器 SHA-256 指纹，核对后再信任。已保存指纹发生变化时，先向服务器管理员确认密钥变更。保活间隔和最大无响应次数可减少空闲网关断线。</p>
      </>
    },
    {
      id: 'files', group: 'SSH', title: 'SFTP 与传输', icon: FolderOpen,
      searchText: 'sftp 文件 本地 远端 上传 下载 拖放 多选 隐藏 收藏 预览 重命名 删除 冲突 队列 重试 进度',
      content: <>
        <h2>SFTP 文件管理与传输</h2>
        <p>聚焦一个已连接的 SSH 窗格后，顶部会出现“文件”视图。左栏浏览当前电脑，右栏浏览该 SSH 连接的远端文件系统。</p>
        <h3>浏览和文件操作</h3>
        <ul>
          <li>使用后退、前进、上级目录、刷新或路径输入框导航。筛选作用于当前目录；眼睛按钮切换隐藏文件；星标收藏当前路径。</li>
          <li>双击目录进入，双击文本文件预览内容。大文件可查看开头或末尾最多 1 MiB。</li>
          <li>工具栏支持新建目录、重命名和递归删除。F2 重命名单个选中项，Delete 删除选中项。</li>
          <li>按住 <kbd>{commandKey}</kbd> 单击增减选择，Shift 单击范围选择，<kbd>{commandKey}+A</kbd> 全选当前列表。</li>
        </ul>
        <h3>上传和下载</h3>
        <ul>
          <li>选中项目后点击中间箭头，或在左右栏之间拖放。也可以从系统文件管理器直接拖入远端栏上传。</li>
          <li>目录会递归扫描并创建所需父目录。传输面板显示队列、总进度、速度、预计时间和失败原因。</li>
          <li>同名冲突可选择覆盖、跳过或自动重命名，并可把选择应用到当前批次。</li>
          <li>失败、中断或取消的任务保留在历史中；恢复对应 SSH 连接后可重试。已完成记录可统一清除。</li>
        </ul>
      </>
    },
    {
      id: 'deployment', group: 'SSH', title: '部署', icon: Rocket,
      searchText: '部署 发布 profile 配置 本地目录 远端目录 预览 diff 新增 变化 相同 仅远端 删除 多余文件 单向同步 rsync',
      content: <>
        <h2>单向部署</h2>
        <p>部署用于把固定本地目录反复发布到当前 SSH 目标的远端目录。入口位于“文件”视图工具栏，适合站点静态资源、构建产物或配置目录的手动发布。</p>
        <h3>使用流程</h3>
        <ol>
          <li>保持目标 SSH 窗格已连接，打开部署并新建配置。</li>
          <li>填写配置名称、本地目录和远端绝对路径；按需启用“删除远端多余文件”。</li>
          <li>点击“保存并预览”。应用会递归扫描两端并显示差异。</li>
          <li>检查新增、变化、相同和仅远端项目，确认目标路径后开始部署。</li>
          <li>新增和变化文件进入普通传输队列；全部上传成功后，执行已确认的远端清理。</li>
        </ol>
        <h3>比较规则与限制</h3>
        <ul>
          <li>相同性依据文件大小和修改时间，允许约 2 秒误差。</li>
          <li>变化文件按整文件上传；空目录随包含文件的父目录创建。</li>
          <li>遇到本地或远端符号链接时会停止并提示，由你确认后调整目录内容。</li>
          <li>远端用户需要读取、创建和覆盖权限；启用清理时还需要删除权限。</li>
        </ul>
        <div className="help-warning"><TriangleAlert size={15} aria-hidden="true" /><div><strong>先核对远端目录</strong><p>“删除远端多余文件”会删除预览中只存在于远端的文件。开始部署前请确认远端根目录和差异列表。</p></div></div>
      </>
    },
    {
      id: 'tunnels', group: 'SSH', title: '端口转发', icon: Network,
      searchText: '端口转发 tunnel 隧道 local 本地 remote 远端 dynamic socks5 监听 地址 端口 目标 localhost 127.0.0.1 0.0.0.0',
      content: <>
        <h2>SSH 端口转发</h2>
        <p>聚焦已连接的 SSH 窗格后点击顶部“端口转发”。配置按 SSH 目标保存，运行实例跟随当前 SSH 连接。</p>
        <table className="help-detail-table"><tbody>
          <tr><th scope="row">本地转发</th><td>本机监听端口，流量经 SSH 服务器访问目标。目标地址从 SSH 服务器的网络视角解析。</td></tr>
          <tr><th scope="row">远端转发</th><td>SSH 服务器监听端口，流量经隧道返回当前电脑可访问的目标。是否允许由服务器 sshd 策略决定。</td></tr>
          <tr><th scope="row">SOCKS5</th><td>在本机创建动态代理。把应用代理指向显示的本机监听地址和端口。</td></tr>
        </tbody></table>
        <h3>配置和启动</h3>
        <ol>
          <li>新建配置，选择类型，填写监听地址、监听端口以及需要时的目标地址和端口。</li>
          <li>监听端口填 <code>0</code> 可让系统自动分配；启动后状态中显示实际端口。</li>
          <li>通常使用 <code>127.0.0.1</code> 供本机访问；需要局域网访问时使用 <code>0.0.0.0</code> 并配置防火墙。</li>
          <li>绿色运行状态表示监听已建立；客户端连接时会验证目标服务可达性。</li>
        </ol>
        <h3>示例</h3>
        <p>远端 Web 服务监听 <code>127.0.0.1:6000</code> 时，可建立本地转发：本机 <code>127.0.0.1:16000</code> → 当前 SSH 服务器 <code>127.0.0.1:6000</code>，然后访问 <code>http://127.0.0.1:16000/</code>。</p>
      </>
    },
    {
      id: 'ai-command', group: '终端', title: 'AI 命令助手', icon: WandSparkles,
      searchText: 'AI 命令 助手 API base url key 模型 provider thinking shell 上下文 脱敏 历史 原始 请求 风险 执行',
      content: <>
        <h2>AI 命令助手</h2>
        <p>AI 命令助手为当前已连接的本地或 SSH 终端生成一条 Shell 命令。它使用“设置 → AI 命令助手”中的 OpenAI 兼容服务配置。</p>
        <h3>配置服务</h3>
        <ol>
          <li>填写 API Base URL、模型和 API Key。Base URL 可指向 <code>/v1</code>，也可填写完整 chat-completions 端点。</li>
          <li>模型厂商通常保持自动识别；需要适配思考参数时再手动选择。</li>
          <li>选择默认目标 Shell 和思考模式，点击测试连接后保存。API Key 存入系统安全存储。</li>
        </ol>
        <h3>生成和使用</h3>
        <ol>
          <li>在本地或 SSH 终端工具栏打开 AI 命令，确认目标 Shell，描述任务。本地终端会根据 macOS Shell、PowerShell 或 WSL 自动选择初始类型。</li>
          <li>可附带终端最近 100 行，最多 16000 字符。自动脱敏会遮盖常见邮箱、手机号和证件号。</li>
          <li>检查生成的命令、解释、假设、警告和风险等级，并可直接编辑命令。</li>
          <li>“复制”写入剪贴板；“填入终端”只输入文字；“执行”会再次确认，高风险命令还要求输入确认文字。</li>
        </ol>
        <h3>历史和诊断</h3>
        <p>最近 10 条成功建议保存在本机。原始数据页显示最近一次请求与响应，便于排查服务错误；API Key 会隐藏，请求正文会包含你选择发送的终端上下文。</p>
        <div className="help-warning"><TriangleAlert size={15} aria-hidden="true" /><div><strong>命令必须人工审核</strong><p>执行删除、覆盖、权限、进程、软件包和数据库命令前，请再次确认目标主机、路径和不可逆影响。</p></div></div>
      </>
    },
    {
      id: 'settings', group: '设置与支持', title: '设置', icon: Settings,
      searchText: '设置 外观 主题 深色 浅色 语言 图标 终端 字体 字号 颜色 透明度 背景图 跨窗格 ssh 远程 agent 集成 注入 ai 命令助手 工具 诊断 导出',
      content: <>
        <h2>应用设置</h2>
        <p>设置分为“通用”“SSH”和“工具”。通用包含界面与终端外观；SSH 包含远程连接集成；工具包含 AI 命令助手等辅助功能。</p>
        <h3>界面</h3>
        <ul>
          <li>主题支持跟随系统、浅色和深色；语言支持简体中文和 English。选择时立即预览，点击保存后持久化。</li>
          <li>可选择 Luna Mux 提供的应用图标变体。</li>
          <li>“导出诊断”生成本机 JSON，包含版本、平台和运行诊断信息，用于排查问题。</li>
        </ul>
        <h3>终端</h3>
        <ul>
          <li>可选择内置 JetBrains Mono、系统等宽字体、已检测字体或手动输入字体名称，并调整字号和前景/背景颜色。</li>
          <li>背景透明度影响终端底色。背景图支持覆盖、完整显示、拉伸和平铺，并跨当前会话的整个终端工作区显示。</li>
          <li>修改会立即应用到已打开终端的预览状态；保存后用于现有和未来窗格。</li>
        </ul>
        <h3>SSH</h3>
        <ul>
          <li>“远程 Agent 集成”是高影响的可选功能。首次开启时会列出远端探测、上传、Shell 包装、反向转发和本地资源控制行为。</li>
          <li>此开关应用到之后新建或重启的 SSH 窗格。已连接的普通 SSH 终端保持当前运行状态。</li>
        </ul>
        <h3>工具：AI 命令助手</h3>
        <ul>
          <li>“AI 命令助手”页配置本地与 SSH 终端中的命令生成功能，包括 API 服务、模型、目标 Shell 和思考模式。</li>
        </ul>
        <p>诊断导出和连接备份聚焦配置与运行状态。分享前请检查生成文件，确认其中的信息适合发送给他人。</p>
      </>
    },
    {
      id: 'security', group: '设置与支持', title: '安全与常见排查', icon: ShieldCheck,
      searchText: '安全 排查 错误 agent hook mcp browser chrome cdp ssh 指纹 凭据 keychain credential manager 断线 超时 sftp 端口转发 诊断',
      content: <>
        <h2>安全与常见排查</h2>
        <h3>安全边界</h3>
        <ul>
          <li>密码、私钥口令和 AI API Key 使用 macOS Keychain 或 Windows Credential Manager；私钥文件本身由文件系统管理。</li>
          <li>每个 Agent 运行时获得临时身份。Hook、Luna MCP 和 Browser MCP 都绑定到当前会话与窗格，运行时退出后撤销。</li>
          <li>Browser Resource 使用会话独立 Profile。Profile 目录和 CDP 端点适合交给 Luna Mux 与 agent-browser 管理。</li>
        </ul>
        <h3>Agent 状态诊断</h3>
        <ul>
          <li>先查看 Agent 页的 Hook 列。显示“终端识别”时，重启该窗格中的 Agent 可重新注入结构化 Hook。</li>
          <li>Codex 提示 Hook 信任时，运行 <code>/hooks</code> 审核；用户配置中的 Hook 开关需要保持启用。</li>
          <li>手动启动 Agent 后，在 Luna Mux 当前终端运行时中运行命令，可让窗格和运行时身份正确关联。</li>
        </ul>
        <h3>浏览器启动诊断</h3>
        <ul>
          <li>浏览器页显示“未找到 Google Chrome”时，安装 Chrome 后点击重新检测。</li>
          <li>“CDP 端口未就绪”通常表示 Chrome 启动慢、Profile 锁定或进程异常。先停止再重启 Browser Resource，并查看页面错误详情。</li>
          <li>通过“浏览器”页启动和恢复 Chrome，可让 Profile、CDP 端口和 Agent 路由保持在同一会话下。</li>
        </ul>
        <h3>SSH、SFTP 和转发</h3>
        <ul>
          <li>连接超时时，检查主机、端口、网络、VPN、跳板机路径和服务器 sshd 策略。</li>
          <li>指纹变化时，先向管理员确认服务器密钥变更。SSH Agent 认证异常时，检查 Agent 服务和 <code>SSH_AUTH_SOCK</code>。</li>
          <li>SFTP 首次打开会初始化通道；断线后先重连对应窗格再重试传输。</li>
          <li>远端转发失败通常与 <code>AllowTcpForwarding</code> 或 <code>GatewayPorts</code> 有关；本地端口失败通常与端口占用和系统特权端口限制有关。</li>
        </ul>
      </>
    }
  ]
}

export function createEnglishHelpSections(commandKey: string): HelpSection[] {
  return [
    {
      id: 'start', group: 'Getting started', title: 'Quick start', icon: BookOpen,
      searchText: 'quick start first project session pane terminal agent browser ssh workspace',
      content: <>
        <h2>Quick start</h2>
        <p>{PRODUCT_INFO.displayName} organizes terminals, Coding Agents, SSH connections, and a managed browser around project Sessions. A Session stores the project root, Pane definitions, split layout, and browser configuration for one reusable workspace.</p>
        <h3>First workflow</h3>
        <ol>
          <li>Select the plus button at the top of the sidebar, create a project Session, and optionally choose its project root.</li>
          <li>Select the plus button on the Session row, then choose a local shell, WSL distribution, SSH target, or Coding Agent launch profile.</li>
          <li>Run commands in the Pane or give the Agent a task. Add or split Panes when you need parallel contexts.</li>
          <li>When an Agent verifies a web page, Luna Mux prepares the Session's managed Chrome environment on demand.</li>
        </ol>
        <h3>Workspace views</h3>
        <table className="help-detail-table"><tbody>
          <tr><th scope="row">Panes</th><td>The main split workspace containing local terminals, SSH terminals, and Agent terminals.</td></tr>
          <tr><th scope="row">Agents</th><td>Active Agent details such as owner Pane, target environment, Hooks, Luna MCP, and Browser MCP.</td></tr>
          <tr><th scope="row">Browser</th><td>Start, focus, restart, or stop the Session's managed Chrome and inspect its Profile, process, and startup diagnostics.</td></tr>
          <tr><th scope="row">Files</th><td>Appears for a focused connected SSH Pane and provides local/remote browsing, file transfer, and deployment tools.</td></tr>
        </tbody></table>
      </>
    },
    {
      id: 'sessions', group: 'Getting started', title: 'Sessions and sidebar', icon: LayoutGrid,
      searchText: 'session project root sidebar expand collapse switch rename delete context menu width pane notification status',
      content: <>
        <h2>Project Sessions and the sidebar</h2>
        <p>A Session is the workspace container for one project. The sidebar creates, switches, expands, edits, and navigates Sessions and their Panes.</p>
        <h3>What a Session stores</h3>
        <ul>
          <li><strong>Name and project root:</strong> the name appears in the sidebar, and the root becomes the initial directory for new local terminals and local Agents.</li>
          <li><strong>Panes and layout:</strong> Pane names, targets, Agent launch types, split directions, and ratios are saved with the Session and restored on app restart.</li>
          <li><strong>Browser environment:</strong> every Session receives an isolated Browser Resource and persistent Profile for web verification by Agents in that Session.</li>
        </ul>
        <h3>Sidebar operations</h3>
        <ul>
          <li>Select a Session to switch to it and expand or collapse it. Multiple Sessions may stay expanded.</li>
          <li>Selecting a Pane switches to its owning Session, focuses it, and returns to the Panes view.</li>
          <li>The Session plus button adds a Pane. Right-click a Session to edit or delete it; right-click a Pane to rename it. The close button closes and removes the Pane.</li>
          <li>Drag the sidebar edge to resize it, double-click the edge to reset it, or use the toolbar button to collapse the whole sidebar.</li>
        </ul>
        <h3>Status and attention</h3>
        <ul>
          <li>The dot before a Pane name shows terminal connection state: green is connected, pulsing yellow is connecting, red is an error, and gray is stopped.</li>
          <li>An Agent waiting for input or permission colors the Pane icon/name and draws an orange border around its terminal. Errors are red; unread completion is blue.</li>
          <li>Opening a Pane marks completion or error events as read; confirming permissions, submitting input, or interrupting the Agent advances actionable states.</li>
        </ul>
        <div className="help-warning"><TriangleAlert size={15} /><div><strong>Deleting a Session</strong><p>Deleting a Session closes its terminals, Agents, and managed browser, then removes Pane and layout definitions. Saved SSH targets remain in the target library.</p></div></div>
      </>
    },
    {
      id: 'terminal', group: 'Getting started', title: 'Panes and terminal', icon: SquareTerminal,
      searchText: 'pane terminal local powershell pwsh wsl macos ssh split layout maximize restart search copy paste shortcut background',
      content: <>
        <h2>Panes, layouts, and terminal use</h2>
        <p>A Pane hosts one local, WSL, SSH, or Agent runtime. Split layouts place multiple contexts from the same Session side by side.</p>
        <h3>Adding a Pane</h3>
        <ol>
          <li>Select the plus button on a Session and optionally enter a Pane name.</li>
          <li>Choose ordinary Terminal or a Coding Agent launch profile.</li>
          <li>Choose a target. Windows offers PowerShell 7 and installed WSL distributions; macOS uses the login shell; remote targets come from the SSH target library.</li>
        </ol>
        <p>You can also run <code>codex</code> or <code>claude</code> manually in an ordinary terminal. Luna Mux recognizes the Agent after activity is detected.</p>
        <h3>Splits and layout</h3>
        <ul>
          <li>The split-right and split-down buttons duplicate the current target definition into a new matching Pane.</li>
          <li>Drag dividers to resize. Ratios persist with the Session.</li>
          <li>Maximize temporarily shows one Pane; select it again to restore the full layout.</li>
          <li>With two or more Panes, the toolbar can arrange them horizontally, vertically, or in two columns per row.</li>
          <li>Restart closes and relaunches the current runtime. Close also removes the persistent Pane definition.</li>
        </ul>
        <h3>Terminal operations</h3>
        <ul>
          <li>Select text and press <kbd>{commandKey}+C</kbd> to copy; use <kbd>{commandKey}+V</kbd> to paste. With no selection, Ctrl+C is delivered to the process.</li>
          <li>Use <kbd>{commandKey}+F</kbd> to search scrollback, the arrows to move between matches, and Escape to close search.</li>
          <li><kbd>{commandKey}</kbd>+click or double-click an HTTP/HTTPS link to open it in the system browser.</li>
          <li>A stopped local runtime can be started and a disconnected SSH runtime can be reconnected from the empty state.</li>
        </ul>
        <h3>Shortcuts</h3>
        <table className="help-shortcut-table"><tbody>
          <tr><th scope="row">Add Pane</th><td><kbd>{commandKey}+Shift+T</kbd></td></tr>
          <tr><th scope="row">Split right</th><td><kbd>{commandKey}+Shift+D</kbd></td></tr>
          <tr><th scope="row">Split down</th><td><kbd>{commandKey}+Alt+Shift+D</kbd></td></tr>
          <tr><th scope="row">Restart Pane</th><td><kbd>{commandKey}+Shift+R</kbd></td></tr>
          <tr><th scope="row">Close Pane</th><td><kbd>{commandKey}+W</kbd></td></tr>
          <tr><th scope="row">Cycle Panes</th><td><kbd>Ctrl+Tab</kbd> / <kbd>Ctrl+Shift+Tab</kbd></td></tr>
          <tr><th scope="row">Open help</th><td><kbd>F1</kbd></td></tr>
        </tbody></table>
      </>
    },
    {
      id: 'agents', group: 'Agents and automation', title: 'Agent workflows', icon: Sparkles,
      searchText: 'agent codex claude code managed manual hook mcp luna browser status attention waiting permission environment integration adapter multi pane collaboration output command bash remote agent ssh injection',
      content: <>
        <h2>Coding Agent workflows</h2>
        <p>Luna Mux runs Codex and Claude Code inside Session Panes and injects Hooks, Luna MCP, and Browser MCP for each runtime. This lets Agents inspect same-Session Panes, request approvals, read managed browser status, and report waiting or completion state back to the interface.</p>
        <h3>Launching an Agent</h3>
        <ol>
          <li>Create a terminal Pane or choose a Coding Agent launch profile while adding a Pane.</li>
          <li>Run <code>codex</code> or <code>claude</code> in the Pane. Managed launch starts the selected Agent directly, and ordinary terminals are recognized after activity appears.</li>
          <li>After an Agent exits, starting it again creates a new runtime identity and binds state and authorization to the current Pane.</li>
        </ol>
        <h3>Attention behavior</h3>
        <ul>
          <li>When Hooks report input, permission, completion, or error, the sidebar, Pane name, and terminal border show attention together.</li>
          <li>Orange requires intervention, red is an error, and blue is unread completion. The connection dot continues to show terminal connection state.</li>
          <li>Open the Pane to read the latest event; submit input, approve permissions, press Escape, or press Ctrl+C to continue or interrupt the Agent.</li>
        </ul>
        <h3>Multi-Agent collaboration inside a Session</h3>
        <p>The project Session is the collaboration boundary. Agents in one Session can discover all of its Panes and current runtimes and collaborate with other terminals or Agents through Luna MCP.</p>
        <ul>
          <li><code>terminal.runtimes.list</code> returns terminal runtimes and owner Panes. A target may contain an Agent or an ordinary Bash, PowerShell, or SSH shell.</li>
          <li><code>terminal.runtime.output.read</code> incrementally reads bounded output by cursor, while <code>terminal.runtime.write</code> writes text or commands to the target PTY.</li>
          <li><code>agents.list</code> and <code>agents.get_status</code> expose structured Agent state; <code>agents.send_task</code> sends task text to the target Agent.</li>
          <li>Output reads, PTY writes, task delivery, and foreground-process interrupts use Luna Mux Session-level control authorization. High-impact actions such as closing a Runtime follow their operation policy.</li>
        </ul>
        <h3>Agents view</h3>
        <p>The Agents view shows the current Session's Agent runtime environment and integration status. Use it to inspect Hooks, MCP, target environments, and browser capability.</p>
        <ul>
          <li>Session environment shows the project root and whether Browser MCP is ready or available on demand.</li>
          <li>Active Agent environments show each Agent's adapter, owner Pane, launch mode, and local/SSH target.</li>
          <li>Hook and Luna MCP columns show structured integration connection and configuration state.</li>
          <li>The filter field locates active runtimes by Agent, Pane, or target; the input field sends text to the selected Agent.</li>
        </ul>
        <h3>Runtime integration</h3>
        <p>Luna Mux writes temporary Hook, Luna MCP, and Browser MCP environment variables for each Pane runtime. The configuration is active for that runtime and expires when the runtime ends.</p>
        <p>Remote Agent integration is enabled under Settings → SSH → Remote Agent integration. For newly created or restarted SSH Panes, Luna Mux probes Codex, Claude Code, and network tools on the remote host, uploads a runtime helper under <code>~/.luna-mux/runtime/&lt;runtime-id&gt;</code>, temporarily adjusts the current shell PATH, and creates reverse forwards bound to remote loopback.</p>
        <p>Remote Browser MCP sends MCP requests back through the current SSH connection and operates the local Session Browser Resource. Hook forwarding needs curl or wget; Browser MCP needs socat, nc/ncat, or bash TCP support. On a normal disconnect, the runtime directory is cleaned up over SFTP.</p>
      </>
    },
    {
      id: 'browser', group: 'Agents and automation', title: 'Browser automation', icon: Globe2,
      searchText: 'browser chrome cdp profile agent-browser mcp automatic on demand start stop restart focus tab page available agent exit profile persistence',
      content: <>
        <h2>Managed browser and web verification</h2>
        <p>Every Session automatically owns one Browser Resource. It gives Agents in that Session an isolated Chrome process, persistent Profile, and agent-browser MCP connection.</p>
        <h3>Automatic use</h3>
        <ol>
          <li>On the first injected <code>agent_browser</code> tool call, Luna Mux resolves the current Session's Browser Resource.</li>
          <li>When Chrome is stopped, Luna Mux starts an isolated Chrome process and connects its CDP endpoint.</li>
          <li>Normal navigation reuses the bound page. Agents can create extra tabs when parallel page context is needed.</li>
          <li>Stopping Chrome or exiting Luna Mux closes the process while preserving login state and site data in the Session's persistent Profile.</li>
        </ol>
        <h3>Browser view controls</h3>
        <table className="help-detail-table"><tbody>
          <tr><th scope="row">Start</th><td>Starts Chrome immediately, useful for signing in or warming up the verification environment.</td></tr>
          <tr><th scope="row">Focus</th><td>Brings the running managed Chrome window to the foreground.</td></tr>
          <tr><th scope="row">Restart</th><td>Relaunches with the same Profile and Session CDP configuration.</td></tr>
          <tr><th scope="row">Stop</th><td>Closes the Chrome process and keeps the Session Profile for the next start.</td></tr>
        </tbody></table>
        <p>Diagnostics show runtime state, PID, CDP endpoint, Profile path, and startup errors. After installing Chrome, use the refresh button to update availability.</p>
        <div className="help-warning"><TriangleAlert size={15} /><div><strong>Agent browser automation</strong><p>Agents can use the injected <code>agent_browser</code> tools to start and operate the current Session's managed Chrome on demand. The Browser view provides manual start, focus, restart, and diagnostics for the same Browser Resource.</p></div></div>
      </>
    },
    {
      id: 'connections', group: 'SSH', title: 'SSH targets and authentication', icon: Server,
      searchText: 'ssh target connection library password private key agent jump host group favorite sort search keepalive config import export luna remote credential fingerprint',
      content: <>
        <h2>SSH targets and authentication</h2>
        <p>An SSH target is a reusable connection definition for remote Panes. Choose one directly in Add Pane or manage targets in the SSH target library at the bottom of the sidebar.</p>
        <h3>Target library</h3>
        <ul>
          <li>Double-click a target to add it to the current Session. A single click selects it and shows details.</li>
          <li>Right-click to edit, duplicate, or delete. Drag targets and groups to reorder or regroup them.</li>
          <li>Search matches name, host, username, group, and notes. Favorites are marked with a star.</li>
          <li>Import OpenSSH Config, Luna Mux backups, or Luna Remote data, and export Luna Mux backups. Backups contain target and group data.</li>
        </ul>
        <h3>Authentication</h3>
        <table className="help-detail-table"><tbody>
          <tr><th scope="row">Password</th><td>Supports password and keyboard-interactive prompts. Remembered values use secure system storage.</td></tr>
          <tr><th scope="row">Private key</th><td>Choose an OpenSSH private key. Encrypted keys prompt for a passphrase, which can be remembered separately in secure system storage.</td></tr>
          <tr><th scope="row">SSH Agent</th><td>Uses the Agent inherited at application launch. macOS/Linux require a valid <code>SSH_AUTH_SOCK</code>; Windows uses OpenSSH Authentication Agent.</td></tr>
          <tr><th scope="row">Jump host</th><td>Connects through one saved direct target. The jump and destination authenticate independently.</td></tr>
        </tbody></table>
        <h3>Host keys and keepalive</h3>
        <p>Verify the SHA-256 fingerprint before trusting a host for the first time. When a saved fingerprint changes, confirm the server-key change with the administrator. Keepalive reduces idle gateway disconnects.</p>
      </>
    },
    {
      id: 'files', group: 'SSH', title: 'SFTP and transfers', icon: FolderOpen,
      searchText: 'sftp file local remote upload download drag multi select hidden favorite preview rename delete conflict queue retry progress',
      content: <>
        <h2>SFTP file management and transfers</h2>
        <p>Focus a connected SSH Pane to expose the Files view. The left browser shows this computer, and the right browser shows the remote filesystem for that SSH connection.</p>
        <h3>Browsing and file operations</h3>
        <ul>
          <li>Use back, forward, parent, refresh, or direct path entry. Filtering affects the current directory; the eye toggles hidden files; the star saves favorite paths.</li>
          <li>Double-click directories to enter and text files to preview content. Large previews can show up to 1 MiB from the beginning or end.</li>
          <li>Create folders, rename, or recursively delete from the toolbar. F2 renames one selected item and Delete removes selected items.</li>
          <li>Use <kbd>{commandKey}</kbd>-click to toggle selection, Shift-click for a range, and <kbd>{commandKey}+A</kbd> for the current list.</li>
        </ul>
        <h3>Transfers</h3>
        <ul>
          <li>Use the center arrows or drag between browsers. Files can also be dragged from the system file manager into the remote browser.</li>
          <li>Directories are scanned recursively and required parent directories are created. The transfer panel shows queue state, total progress, speed, ETA, and errors.</li>
          <li>Name conflicts may overwrite, skip, or auto-rename, and the decision may be applied to the current batch.</li>
          <li>Failed, interrupted, and cancelled items remain in history. Reconnect the matching SSH Pane before retrying. Completed records can be cleared together.</li>
        </ul>
      </>
    },
    {
      id: 'deployment', group: 'SSH', title: 'Deployment', icon: Rocket,
      searchText: 'deployment publish profile local directory remote directory preview diff new changed same remote only delete extraneous one way sync rsync',
      content: <>
        <h2>One-way deployment</h2>
        <p>Deployment repeatedly publishes one local directory to a directory on the focused SSH target. Open it from the Files toolbar for manual releases of static sites, build output, or configuration directories.</p>
        <h3>Workflow</h3>
        <ol>
          <li>Keep the SSH Pane connected, open Deployment, and create a profile.</li>
          <li>Enter a name, local directory, and absolute remote path. Optionally enable deletion of remote-only files.</li>
          <li>Save and preview. Luna Mux scans both trees and shows the differences.</li>
          <li>Review new, changed, same, and remote-only entries, then start deployment.</li>
          <li>New and changed files enter the transfer queue. Confirmed cleanup runs after every upload succeeds.</li>
        </ol>
        <h3>Comparison rules and limits</h3>
        <ul>
          <li>Equality uses size and modification time with roughly two seconds of tolerance.</li>
          <li>Changed files upload in full. Parent directories are created as needed for included files.</li>
          <li>Symbolic links stop the operation and prompt you to adjust the directory contents.</li>
          <li>The remote user needs read, create, and overwrite permissions, plus delete permission when cleanup is enabled.</li>
        </ul>
        <div className="help-warning"><TriangleAlert size={15} /><div><strong>Verify the remote root</strong><p>Cleanup deletes paths that exist only remotely in the preview. Confirm the remote root and difference list before starting deployment.</p></div></div>
      </>
    },
    {
      id: 'tunnels', group: 'SSH', title: 'Port forwarding', icon: Network,
      searchText: 'port forwarding tunnel local remote dynamic socks bind address port target localhost 127.0.0.1 0.0.0.0',
      content: <>
        <h2>SSH port forwarding</h2>
        <p>Focus a connected SSH Pane and select Port forwarding. Profiles persist with the SSH target, and running tunnel instances follow the current SSH connection.</p>
        <table className="help-detail-table"><tbody>
          <tr><th scope="row">Local</th><td>Listens locally and reaches a target through the SSH server. The target is resolved from the server's network perspective.</td></tr>
          <tr><th scope="row">Remote</th><td>Listens on the SSH server and forwards back to a target reachable from this computer. Server sshd policy controls availability.</td></tr>
          <tr><th scope="row">SOCKS5</th><td>Creates a local dynamic proxy for applications configured with the displayed endpoint.</td></tr>
        </tbody></table>
        <h3>Configure and run</h3>
        <ol>
          <li>Create a profile, select its type, and enter bind and target endpoints as required.</li>
          <li>Bind port <code>0</code> requests an automatic port; the running state shows the assigned value.</li>
          <li>Use <code>127.0.0.1</code> for local-only access. Use <code>0.0.0.0</code> when LAN exposure is intentional and protected by a firewall.</li>
          <li>A green running state confirms the listener. Target reachability is validated when a client connects.</li>
        </ol>
        <h3>Example</h3>
        <p>When a Web service on the current SSH server listens at <code>127.0.0.1:6000</code>, create a local forward from <code>127.0.0.1:16000</code> to the current SSH server's <code>127.0.0.1:6000</code>, then visit <code>http://127.0.0.1:16000/</code>.</p>
      </>
    },
    {
      id: 'ai-command', group: 'Terminal', title: 'AI command assistant', icon: WandSparkles,
      searchText: 'AI command assistant API base url key model provider thinking shell context redact history raw request risk execute',
      content: <>
        <h2>AI command assistant</h2>
        <p>The assistant generates one shell command for the focused connected local or SSH terminal. It uses the OpenAI-compatible service configured in Settings → AI command assistant.</p>
        <h3>Service setup</h3>
        <ol>
          <li>Enter API Base URL, model, and API Key. The URL may be a <code>/v1</code> base or a full chat-completions endpoint.</li>
          <li>Keep provider detection automatic unless you need model-specific thinking controls.</li>
          <li>Choose the default target shell and thinking mode, test the connection, and save. The API Key uses secure system storage.</li>
        </ol>
        <h3>Generate and use</h3>
        <ol>
          <li>Open AI command from a local or SSH terminal toolbar, confirm the target shell, and describe the task. Local terminals initially select macOS Shell, PowerShell, or Linux for WSL automatically.</li>
          <li>Optional terminal context is limited to 100 lines and 16,000 characters. Redaction masks common email, phone, and ID formats.</li>
          <li>Review and optionally edit the command, explanation, assumptions, warnings, and risk level.</li>
          <li>Copy writes to the clipboard; Insert writes without Enter; Execute asks again. High-risk commands also require typing the confirmation text.</li>
        </ol>
        <h3>History and diagnostics</h3>
        <p>The latest ten successful suggestions stay locally. Raw data shows the latest request and response for troubleshooting. The API Key is hidden, and selected terminal context appears in the request body.</p>
        <div className="help-warning"><TriangleAlert size={15} /><div><strong>Review every command</strong><p>Before executing deletion, overwrite, privilege, process, package, or database commands, recheck host, paths, and irreversible effects.</p></div></div>
      </>
    },
    {
      id: 'settings', group: 'Settings and support', title: 'Settings', icon: Settings,
      searchText: 'settings appearance theme dark light language icon terminal font size color opacity background image ssh remote agent integration injection ai command assistant tools diagnostics export',
      content: <>
        <h2>Application settings</h2>
        <p>Settings are grouped into General, SSH, and Tools. General contains interface and terminal appearance; SSH contains remote-connection integration; Tools contains helpers such as AI command assistant.</p>
        <h3>Appearance</h3>
        <ul>
          <li>Theme supports System, Light, and Dark. Language supports Simplified Chinese and English. Choices preview immediately and persist after Save.</li>
          <li>Select one of the Luna Mux app-icon variants.</li>
          <li>Export diagnostics creates a local JSON file containing version, platform, and runtime diagnostic information.</li>
        </ul>
        <h3>Terminal</h3>
        <ul>
          <li>Choose bundled JetBrains Mono, the system monospace font, a detected font, or enter a font name. Adjust size and foreground/background colors.</li>
          <li>Background opacity affects terminal color. Images support Cover, Contain, Stretch, and Tile and span the Session's entire terminal workspace.</li>
          <li>Changes preview in open terminals and apply to existing and future Panes after Save.</li>
        </ul>
        <h3>SSH</h3>
        <ul>
          <li>Remote Agent integration is a high-impact opt-in feature. Its enable confirmation lists remote probes, uploads, shell wrapping, reverse forwards, and local resource access.</li>
          <li>The setting applies to SSH Panes created or restarted afterward. Already connected ordinary SSH terminals keep their current runtime state.</li>
        </ul>
        <h3>Tools: AI command assistant</h3>
        <ul>
          <li>AI command assistant settings configure command generation for local and SSH terminals, including API service, model, target shell, and thinking mode.</li>
        </ul>
        <p>Diagnostics and connection backups focus on configuration and runtime state. Review exported files before sharing them.</p>
      </>
    },
    {
      id: 'security', group: 'Settings and support', title: 'Security and troubleshooting', icon: ShieldCheck,
      searchText: 'security troubleshoot error agent hook mcp browser chrome cdp ssh fingerprint credential keychain manager disconnect timeout sftp tunnel diagnostics',
      content: <>
        <h2>Security and troubleshooting</h2>
        <h3>Security boundaries</h3>
        <ul>
          <li>Passwords, private-key passphrases, and AI API Keys use macOS Keychain or Windows Credential Manager. Private-key files are managed by the filesystem.</li>
          <li>Each Agent runtime receives an ephemeral identity. Hooks, Luna MCP, and Browser MCP are scoped to its Session and Pane and revoked on exit.</li>
          <li>Browser Resources use Session-isolated Profiles. Profile directories and CDP endpoints are best managed through Luna Mux and agent-browser.</li>
        </ul>
        <h3>Agent state diagnostics</h3>
        <ul>
          <li>Check the Hook column in Agents. When it shows Terminal detection, restart the Agent inside its Luna Mux Pane to inject structured Hooks again.</li>
          <li>If Codex asks to trust Hooks, run <code>/hooks</code>. Keep the user-level Hook feature enabled for structured status.</li>
          <li>Run manually launched Agents inside the current Luna Mux terminal runtime so the Pane and runtime identity are associated correctly.</li>
        </ul>
        <h3>Browser startup diagnostics</h3>
        <ul>
          <li>If Browser shows Google Chrome not found, install Chrome and use Check for browser again.</li>
          <li>A CDP readiness timeout can indicate slow startup, a locked Profile, or an unhealthy process. Stop and restart from Browser and inspect its detailed error.</li>
          <li>Starting and recovering Chrome from the Browser view keeps the Profile, CDP port, and Agent routing under the same Session.</li>
        </ul>
        <h3>SSH, SFTP, and forwarding</h3>
        <ul>
          <li>For timeouts, check host, port, network, VPN, jump routing, and sshd policy.</li>
          <li>Confirm changed host keys with an administrator. For SSH Agent failures, check its service and <code>SSH_AUTH_SOCK</code>.</li>
          <li>SFTP initializes a channel on first use. Reconnect the matching Pane before retrying interrupted transfers.</li>
          <li>Remote-forward failures often involve <code>AllowTcpForwarding</code> or <code>GatewayPorts</code>; local failures often involve occupied or privileged ports.</li>
        </ul>
      </>
    }
  ]
}
