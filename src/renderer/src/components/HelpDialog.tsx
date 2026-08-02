import { useEffect, useMemo, useState, type ReactNode } from 'react'
import { BookOpen, CircleHelp, FolderOpen, Network, Rocket, Search, Server, Settings, ShieldCheck, SquareTerminal, X, type LucideIcon } from 'lucide-react'

interface HelpSection {
  id: string
  title: string
  icon: LucideIcon
  searchText: string
  content: ReactNode
}

function createSections(commandKey: string): HelpSection[] {
  return [
    {
      id: 'start', title: '快速开始', icon: BookOpen,
      searchText: '快速开始 新增连接 连接 终端 文件传输 session 会话',
      content: <>
        <h2>快速开始</h2>
        <p>从保存一台机器到打开终端，只需要完成以下步骤。</p>
        <ol>
          <li>点击侧边栏顶部的“新增连接”，填写名称、Host、端口、用户名和认证方式。</li>
          <li>点击侧边栏中的机器。首次连接需要输入凭据，并确认服务器主机指纹。</li>
          <li>连接成功后使用“终端”执行命令，或切换到“文件传输”浏览本地和远端目录。</li>
          <li>同一台机器已经连接时，再次点击侧边栏中的机器会创建一个独立 session。</li>
        </ol>
        <h3>界面布局</h3>
        <ul>
          <li>标签栏左侧按钮用于展开或折叠连接管理。</li>
          <li>侧边栏边缘可拖动调整宽度，双击边缘恢复默认宽度。</li>
          <li>每个标签对应独立 session；关闭标签会同时断开该 session。</li>
        </ul>
      </>
    },
    {
      id: 'connections', title: '连接与认证', icon: Server,
      searchText: '连接 认证 密码 私钥 agent 跳板机 分组 收藏 搜索 keepalive 保活 config 导入 host proxyjump 凭据 指纹',
      content: <>
        <h2>连接与认证</h2>
        <h3>管理连接</h3>
        <ul>
          <li>侧边栏顶部依次提供 OpenSSH Config 导入、新增、编辑和删除。</li>
          <li>连接可以设置分组和收藏；收藏连接优先排列，搜索会匹配名称、地址、用户名、分组和备注。</li>
          <li>密码或私钥口令可保存到系统安全存储，也可以在编辑连接时清除。</li>
        </ul>
        <h3>认证方式</h3>
        <dl>
          <dt>密码</dt><dd>连接时输入服务器密码，也支持 keyboard-interactive 提示。</dd>
          <dt>私钥</dt><dd>选择 OpenSSH 私钥文件；加密私钥连接时输入私钥口令。</dd>
          <dt>SSH Agent</dt><dd>使用应用启动时继承的系统 Agent。macOS/Linux 需要有效的 <code>SSH_AUTH_SOCK</code>，Windows 使用 OpenSSH Agent。</dd>
          <dt>跳板机</dt><dd>在“连接方式”中选择一个直连配置。目前支持单级跳板，跳板机和目标机器分别认证。</dd>
        </dl>
        <h3>Session 保活</h3>
        <p>编辑连接可设置保活间隔和最大无响应次数。保活能减少企业网关因空闲而断开连接，但无法跨越电脑休眠、网络切换或服务器强制回收。长时间任务建议在远端使用 <code>tmux</code> 或 <code>screen</code>。</p>
        <h3>导入 OpenSSH Config</h3>
        <p>点击侧边栏顶部的导入按钮，选择通常位于 <code>~/.ssh/config</code> 的文件，然后勾选要导入的 Host。</p>
        <ul>
          <li>支持映射 <code>HostName</code>、<code>User</code>、<code>Port</code>、第一个 <code>IdentityFile</code> 和 <code>ProxyJump</code>。</li>
          <li>仅显示具体 Host；通配符 Host 不会作为连接导入。</li>
          <li>出于安全原因，不执行 <code>Match exec</code>、<code>ProxyCommand</code> 或 shell 命令展开。</li>
        </ul>
      </>
    },
    {
      id: 'terminal', title: '会话与终端', icon: SquareTerminal,
      searchText: '会话 session 终端 标签 新建 断开 重连 搜索 复制 粘贴 链接 快捷键 ctrl command cmd webgl',
      content: <>
        <h2>会话与终端</h2>
        <h3>多 session</h3>
        <ul>
          <li>工具栏“新建会话”会为当前机器打开另一个独立 session。</li>
          <li>断开后标签和滚屏内容会保留，点击“连接”可在原标签中重新连接。</li>
          <li>标签可单击切换、中键关闭；后台标签不会丢失终端内容。</li>
        </ul>
        <h3>终端操作</h3>
        <ul>
          <li>拖选文本后按 <kbd>{commandKey}+C</kbd> 复制；按 <kbd>{commandKey}+V</kbd> 粘贴。</li>
          <li>按 <kbd>{commandKey}+F</kbd> 搜索滚屏内容，使用上下按钮切换结果，按 <kbd>Esc</kbd> 关闭搜索。</li>
          <li>终端中的 HTTP/HTTPS 链接可直接点击，并由系统默认浏览器打开。</li>
        </ul>
        <h3>应用快捷键</h3>
        <table><tbody>
          <tr><th>新建当前机器的 session</th><td><kbd>{commandKey}+Shift+T</kbd></td></tr>
          <tr><th>关闭当前标签</th><td><kbd>{commandKey}+W</kbd></td></tr>
          <tr><th>切换标签</th><td><kbd>Ctrl+Tab</kbd> / <kbd>Ctrl+Shift+Tab</kbd></td></tr>
          <tr><th>打开帮助</th><td><kbd>F1</kbd></td></tr>
        </tbody></table>
      </>
    },
    {
      id: 'files', title: 'SFTP 与传输', icon: FolderOpen,
      searchText: 'sftp 文件 目录 上传 下载 拖放 多选 隐藏 收藏 预览 重命名 删除 冲突 队列 重试 进度',
      content: <>
        <h2>SFTP 与文件传输</h2>
        <p>连接成功后切换到“文件传输”。左侧是本地文件，右侧是当前 session 对应的远端文件。</p>
        <h3>浏览与管理</h3>
        <ul>
          <li>使用后退、前进、上级目录、刷新或直接输入路径进行导航。</li>
          <li>筛选框只过滤当前目录；眼睛按钮显示或隐藏点文件；星标按钮收藏当前路径。</li>
          <li>双击目录进入，双击文件进行文本预览。大文件最多预览开头或末尾 1 MiB，二进制文件不会强制解码。</li>
          <li>工具栏支持新建目录、重命名和递归删除。删除操作不可撤销。</li>
        </ul>
        <h3>选择与传输</h3>
        <ul>
          <li>按住 <kbd>{commandKey}</kbd> 单击可增减选择，按住 <kbd>Shift</kbd> 单击可范围选择，<kbd>{commandKey}+A</kbd> 全选当前列表。</li>
          <li><kbd>F2</kbd> 重命名单个选中项，<kbd>Delete</kbd> 删除选中项。</li>
          <li>选择文件后使用中间箭头上传或下载，也可以在两栏间拖放。系统文件管理器中的文件可直接拖到远端栏上传。</li>
        </ul>
        <h3>队列与冲突</h3>
        <p>传输面板显示总体进度、速度和预计时间。发生同名冲突时可选择覆盖、跳过或自动重命名，并将选择应用到当前批次。失败、中断或取消的任务可以在连接恢复后重试。</p>
      </>
    },
    {
      id: 'deployment', title: '部署', icon: Rocket,
      searchText: '部署 profile 配置 本地目录 远端目录 预览 diff 新增 变化 删除 多余文件',
      content: <>
        <h2>单向部署</h2>
        <p>部署用于把一个本地目录同步到当前连接的远端目录。入口位于“文件传输”视图的 session 工具栏。</p>
        <ol>
          <li>新建部署配置，填写配置名称，选择本地目录并填写远端绝对路径。</li>
          <li>点击“保存并预览”，检查新增、变化、相同和仅远端文件。</li>
          <li>确认差异后点击“开始部署”，文件会进入普通传输队列。</li>
        </ol>
        <div className="help-warning"><strong>删除远端多余文件</strong><p>启用此选项后，仅当本次所有上传任务都成功时，才会删除预览中标记为“仅远端”的文件。开始前仍应确认远端目录和差异列表。</p></div>
      </>
    },
    {
      id: 'tunnels', title: '端口转发', icon: Network,
      searchText: '端口转发 tunnel 隧道 local remote dynamic socks5 监听地址 端口 目标 localhost 127.0.0.1 0.0.0.0',
      content: <>
        <h2>端口转发</h2>
        <p>连接成功后点击 session 工具栏中的“端口转发”。配置按机器保存，但每个运行中的转发只属于启动它的 session，session 断开时会自动停止。</p>
        <dl>
          <dt>本地转发</dt><dd>在本机监听端口，将连接通过 SSH 转发到服务器一侧可访问的目标。例如本机 <code>127.0.0.1:8080</code> 转到内网数据库或 Web 服务。</dd>
          <dt>远端转发</dt><dd>在 SSH 服务器上监听端口，并把连接转回本机可访问的目标。服务器是否允许由其 SSH 配置决定。</dd>
          <dt>SOCKS5</dt><dd>在本机创建动态代理。应用程序将代理设置为所显示的监听地址和端口即可。</dd>
        </dl>
        <h3>字段说明</h3>
        <ul>
          <li>监听地址通常使用 <code>127.0.0.1</code>；端口填 <code>0</code> 时由系统自动分配，启动后显示实际端口。</li>
          <li>本地和远端转发需要目标地址与目标端口；SOCKS5 会根据客户端请求动态选择目标。</li>
        </ul>
        <div className="help-warning"><strong>网络暴露</strong><p>使用 <code>0.0.0.0</code> 或其他外部接口会允许同一网络中的设备访问监听端口。除非明确需要并已配置防火墙，否则使用 <code>127.0.0.1</code>。</p></div>
      </>
    },
    {
      id: 'settings', title: '外观与诊断', icon: Settings,
      searchText: '设置 外观 界面 主题 深色 浅色 跟随系统 字体 字号 颜色 透明度 背景图 图片 图标 诊断 导出 json',
      content: <>
        <h2>外观与诊断</h2>
        <h3>界面主题</h3>
        <p>设置中的“界面”页签可以选择跟随系统、浅色或深色模式。界面主题应用于侧边栏、标签栏、文件传输和弹窗等非终端区域。</p>
        <h3>终端外观</h3>
        <ul>
          <li>设置中可以调整字体、字号、文字颜色、背景颜色和背景透明度。</li>
          <li>背景图支持覆盖、完整显示、拉伸和平铺。设置立即应用到已经打开的终端，不需要重新连接。</li>
          <li>应用图标可在“应用图标”页签中切换。</li>
        </ul>
        <h3>导出诊断</h3>
        <p>在“应用图标”页签点击“导出诊断信息”生成 JSON 文件。诊断包含应用版本、平台、非敏感连接元数据、session 状态和传输汇总。</p>
        <p>诊断不会包含密码、私钥内容或路径、主机指纹、终端输出以及文件内容。</p>
      </>
    },
    {
      id: 'security', title: '安全与排查', icon: ShieldCheck,
      searchText: '安全 主机指纹 changed known host 密码 凭据 断线 排查 agent sftp 超时 tmux screen',
      content: <>
        <h2>安全与常见排查</h2>
        <h3>主机指纹</h3>
        <p>首次连接会显示服务器 SHA-256 指纹。应与管理员提供的指纹核对后再信任。如果指纹发生变化，除非确认服务器密钥已更换，否则不要继续连接。</p>
        <h3>常见问题</h3>
        <dl>
          <dt>Agent 不可用</dt><dd>确认密钥已加入 Agent，并从具有正确 Agent 环境的终端启动应用；Windows 需启动 OpenSSH Authentication Agent 服务。</dd>
          <dt>空闲后断线</dt><dd>启用连接的 session 保活。如果企业网关仍强制回收，缩短保活间隔；长任务使用远端会话管理工具。</dd>
          <dt>SFTP 较慢</dt><dd>首次打开会初始化 SFTP 通道。应用会缓存目录并在可用时使用 SSH exec 加速，刷新操作会绕过缓存。</dd>
          <dt>无法远端转发</dt><dd>服务器可能禁用了 TCP forwarding 或远端监听。请检查 sshd 的 <code>AllowTcpForwarding</code> 和 <code>GatewayPorts</code> 策略。</dd>
          <dt>重新连接要密码</dt><dd>勾选“记住凭据”可使用系统安全存储；如果企业策略禁止长期凭据，断开后的重新认证无法由应用绕过。</dd>
        </dl>
      </>
    }
  ]
}

export function HelpDialog({ onClose }: { onClose(): void }): React.JSX.Element {
  const commandKey = window.api.platform === 'darwin' ? 'Cmd' : 'Ctrl'
  const sections = useMemo(() => createSections(commandKey), [commandKey])
  const [active, setActive] = useState('start')
  const [query, setQuery] = useState('')
  const normalizedQuery = query.trim().toLowerCase()
  const matches = normalizedQuery ? sections.filter((section) => `${section.title} ${section.searchText}`.toLowerCase().includes(normalizedQuery)) : sections
  const shown = normalizedQuery ? matches : sections.filter((section) => section.id === active)

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => { if (event.key === 'Escape') onClose() }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [onClose])

  return <div className="help-backdrop" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose() }}>
    <section className="help-dialog" role="dialog" aria-modal="true" aria-label="使用帮助">
      <header><div className="help-title"><CircleHelp size={18} /><strong>使用帮助</strong></div><label className="help-search"><Search size={15} /><input autoFocus placeholder="搜索帮助" value={query} onChange={(event) => setQuery(event.target.value)} /></label><button className="icon-button" title="关闭帮助" onClick={onClose}><X size={18} /></button></header>
      <div className="help-body">
        <nav aria-label="帮助目录">{matches.map((section) => { const Icon = section.icon; return <button key={section.id} className={!normalizedQuery && active === section.id ? 'active' : ''} onClick={() => { setActive(section.id); setQuery('') }}><Icon size={16} /><span>{section.title}</span></button> })}</nav>
        <main>{shown.length ? shown.map((section) => <article key={section.id} className="help-article">{section.content}</article>) : <div className="help-no-results"><Search size={24} /><strong>没有匹配的帮助内容</strong><span>尝试搜索“连接”“SFTP”或“端口转发”</span></div>}</main>
      </div>
    </section>
  </div>
}
