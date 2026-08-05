import { useEffect, useMemo, useState, type ReactNode } from 'react'
import { BookOpen, Bot, CircleHelp, FolderOpen, Network, Rocket, Search, Server, Settings, ShieldCheck, SquareTerminal, TriangleAlert, WandSparkles, X, type LucideIcon } from 'lucide-react'

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
          <li>双击侧边栏中的机器。首次连接需要输入凭据，并确认服务器主机指纹。</li>
          <li>连接成功后使用“终端”执行命令，或切换到“文件传输”浏览本地和远端目录。</li>
          <li>同一台机器已经连接时，再次双击侧边栏中的机器会创建一个独立 session。</li>
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
      searchText: '连接 认证 密码 私钥 agent 跳板机 分组 收藏 搜索 keepalive 保活 config 导入 导出 备份 host proxyjump 凭据 指纹',
      content: <>
        <h2>连接与认证</h2>
        <p>连接配置保存在本机。机器、分组和备份功能集中在左侧连接栏，认证方式和保活参数在新增或编辑连接时设置。</p>
        <h3>管理连接</h3>
        <ul>
          <li><strong>选择与连接：</strong>单击选中机器，双击建立连接。侧边栏顶部保留新增和删除两个常用操作。</li>
          <li><strong>更多操作：</strong>右键机器可编辑、复制或删除；右键侧边栏空白处可新增分组、导入和导出。</li>
          <li><strong>整理连接：</strong>机器和分组均可拖动排序，也可将机器拖入分组。收藏连接会优先排列。</li>
          <li><strong>查找：</strong>搜索会匹配连接名称、地址、用户名、分组和备注。</li>
          <li><strong>保存凭据：</strong>密码或私钥口令可保存到系统安全存储，也可在编辑连接时清除。</li>
        </ul>
        <h3>认证方式</h3>
        <table className="help-detail-table"><tbody>
          <tr><th scope="row">密码</th><td>连接时输入服务器密码，也支持 keyboard-interactive 提示。</td></tr>
          <tr><th scope="row">私钥</th><td>选择 OpenSSH 私钥文件；加密私钥连接时输入私钥口令。</td></tr>
          <tr><th scope="row">SSH Agent</th><td>使用应用启动时继承的系统 Agent。macOS/Linux 需要有效的 <code>SSH_AUTH_SOCK</code>，Windows 使用 OpenSSH Agent。</td></tr>
        </tbody></table>
        <h3>连接路径</h3>
        <table className="help-detail-table"><tbody>
          <tr><th scope="row">直连</th><td>当前电脑直接连接目标机器。适用于目标地址可从当前网络访问的情况。</td></tr>
          <tr><th scope="row">跳板机</th><td>先连接一台已保存的直连机器，再由它连接目标机器。目前支持单级跳板，跳板机和目标机器分别认证。</td></tr>
        </tbody></table>
        <h3>连接稳定性</h3>
        <p>编辑连接可设置保活间隔和最大无响应次数。保活能减少企业网关因空闲而断开连接，但无法跨越电脑休眠、网络切换或服务器强制回收。长时间任务建议在远端使用 <code>tmux</code> 或 <code>screen</code>。</p>
        <h3>导入与备份</h3>
        <div className="help-table-scroll"><table className="help-comparison-table"><colgroup><col /><col /><col /></colgroup><thead><tr><th scope="col">项目</th><th scope="col">OpenSSH Config</th><th scope="col">连接备份</th></tr></thead><tbody>
          <tr><th scope="row">入口</th><td>右键侧边栏空白处，选择“导入 OpenSSH Config”</td><td>右键侧边栏空白处，选择导入或导出连接备份</td></tr>
          <tr><th scope="row">用途</th><td>读取通常位于 <code>~/.ssh/config</code> 的现有 SSH 配置</td><td>迁移或备份本软件中的全部连接和分组</td></tr>
          <tr><th scope="row">包含内容</th><td>Host、地址、用户、端口、首个私钥和 ProxyJump</td><td>分组、顺序、收藏、备注、保活和跳板机关系</td></tr>
          <tr><th scope="row">不会导入</th><td>通配 Host、命令及其他不安全规则</td><td>密码、私钥口令及其他安全存储中的凭据</td></tr>
        </tbody></table></div>
      </>
    },
    {
      id: 'terminal', title: '会话与终端', icon: SquareTerminal,
      searchText: '会话 session 终端 标签 新建 断开 重连 搜索 复制 粘贴 链接 快捷键 ctrl command cmd webgl',
      content: <>
        <h2>会话与终端</h2>
        <h3>多 session</h3>
        <ul>
          <li>已有活动 session 时，再次点击侧边栏中的同一机器会打开另一个独立 session。</li>
          <li>断开后标签和滚屏内容会保留，再次点击侧边栏中的机器可在原标签中重新连接。</li>
          <li>标签可单击切换、中键关闭；后台标签不会丢失终端内容。</li>
        </ul>
        <h3>终端操作</h3>
        <ul>
          <li>拖选文本后按 <kbd>{commandKey}+C</kbd> 复制；按 <kbd>{commandKey}+V</kbd> 粘贴。</li>
          <li>按 <kbd>{commandKey}+F</kbd> 搜索滚屏内容，使用上下按钮切换结果，按 <kbd>Esc</kbd> 关闭搜索。</li>
          <li>终端中的 HTTP/HTTPS 链接可直接点击，并由系统默认浏览器打开。</li>
        </ul>
        <h3>应用快捷键</h3>
        <table className="help-shortcut-table"><tbody>
          <tr><th scope="row">新建当前机器的 session</th><td><kbd>{commandKey}+Shift+T</kbd></td></tr>
          <tr><th scope="row">关闭当前标签</th><td><kbd>{commandKey}+W</kbd></td></tr>
          <tr><th scope="row">切换标签</th><td><kbd>Ctrl+Tab</kbd> / <kbd>Ctrl+Shift+Tab</kbd></td></tr>
          <tr><th scope="row">打开帮助</th><td><kbd>F1</kbd></td></tr>
        </tbody></table>
      </>
    },
    {
      id: 'ai-settings', title: 'AI 服务配置', icon: Bot,
      searchText: 'AI 服务 配置 连接 API Base URL Key 模型 厂商 思考 推理 chat completions OpenAI 兼容 测试连接',
      content: <>
        <h2>AI 服务配置</h2>
        <p>AI 服务配置是应用级能力，供 AI 命令助手及后续其他 AI 功能共同使用。入口位于“设置 → AI”。</p>
        <h3>连接服务</h3>
        <ol>
          <li>打开“设置 → AI”，填写 API Base URL、模型名称和 API Key。</li>
          <li>模型厂商可保持“自动识别”；只有自动识别不准确时才需要手动指定。</li>
          <li>点击“测试连接”，确认地址、密钥、模型和请求格式均可用，然后保存设置。</li>
        </ol>
        <h3>协议与地址</h3>
        <ul>
          <li>API 使用 OpenAI 兼容的 <code>/chat/completions</code> 请求格式。Base URL 可填写以 <code>/v1</code> 结尾的基础地址，也可直接填写完整端点。</li>
          <li>厂商选项用于适配不同模型的思考控制参数；服务端或代理仍需接受 OpenAI 兼容请求。</li>
          <li>API Key 保存在系统安全存储中，不写入连接备份或诊断文件。</li>
          <li>“跟随模型默认”不发送额外思考参数；开启或尽量关闭会根据所选厂商生成对应参数。部分模型无法完全关闭思考。</li>
        </ul>
        <h3>默认目标 Shell</h3>
        <p>默认目标 Shell 是当前 AI 功能的通用偏好。AI 命令助手打开时会使用此值，并允许针对当前任务临时切换；临时切换后会同步更新默认值。</p>
      </>
    },
    {
      id: 'ai-command', title: 'AI 命令助手', icon: WandSparkles,
      searchText: 'AI 命令 助手 shell 上下文 100行 16000 脱敏 手机号 邮箱 证件号 历史 原始 请求 响应 风险 确认 复制 填入 执行',
      content: <>
        <h2>AI 命令助手</h2>
        <p>AI 命令助手根据自然语言生成一条适用于目标 Shell 的命令。入口位于已打开机器的终端工具栏。使用前需要先完成 AI 服务配置。</p>
        <h3>生成与使用命令</h3>
        <ol>
          <li>确认目标机器和 Shell，输入要完成的操作，然后点击“生成命令”。</li>
          <li>检查命令、解释、假设、注意事项和风险等级。命令可以在结果框中继续修改。</li>
          <li>选择“复制”“填入终端”或“执行”。填入终端只写入命令，不发送回车；执行会先显示二次确认。</li>
        </ol>
        <h3>终端上下文与脱敏</h3>
        <ul>
          <li>勾选“附带终端最近 100 行”后，最多发送最近 100 行、16000 个字符，帮助模型理解文件名、目录和最近输出。</li>
          <li>自动脱敏会掩码手机号、邮箱、证件号等常见格式，但无法保证识别所有业务敏感数据。发送前仍应自行判断。</li>
          <li>终端内容只作为不可信参考数据，提示词明确要求模型不得执行或服从其中出现的指令。</li>
        </ul>
        <h3>历史与原始数据</h3>
        <ul>
          <li>历史按钮保存最近 10 条成功生成的命令，包括需求、Shell、说明和风险信息。历史仅保存在本机，可复制、重新使用或清空。</li>
          <li>原始数据按钮显示最近一次 AI 请求和响应，适合排查限流、余额不足或返回格式异常。API Key 会隐藏，但请求正文可能包含所选终端上下文。</li>
        </ul>
        <div className="help-warning"><TriangleAlert size={15} aria-hidden="true" /><div><strong>执行前人工确认</strong><p>风险判断由模型结果和本地规则共同给出，只能作为辅助。尤其是删除、覆盖、权限、进程、软件包和数据库命令，应确认目标路径及影响范围后再执行。</p></div></div>
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
      searchText: '部署 profile 配置 本地目录 远端目录 预览 diff 新增 变化 相同 仅远端 删除 多余文件 单向同步 发布 rsync 哈希 修改时间 符号链接',
      content: <>
        <h2>单向部署</h2>
        <p>部署用于将一个固定的本地目录反复发布到当前连接的远端目录。它会先比较整个目录，再上传新增或变化的文件；适合发布网站、构建产物、脚本和配置目录。入口位于“文件传输”视图的 session 工具栏。</p>
        <h3>与普通文件传输的区别</h3>
        <div className="help-table-scroll"><table className="help-comparison-table"><colgroup><col /><col /><col /></colgroup><thead><tr><th scope="col">能力</th><th scope="col">普通文件传输</th><th scope="col">部署</th></tr></thead><tbody>
          <tr><th scope="row">操作方式</th><td>每次手动选择文件或目录</td><td>保存固定的本地目录和远端目录</td></tr>
          <tr><th scope="row">方向</th><td>支持上传和下载</td><td>仅从本地发布到远端</td></tr>
          <tr><th scope="row">执行前比较</th><td>不比较整个目录</td><td>递归扫描两端并显示差异</td></tr>
          <tr><th scope="row">传输内容</th><td>传输本次选择的项目</td><td>只上传新增和变化的文件</td></tr>
          <tr><th scope="row">远端多余文件</th><td>保持不动</td><td>可选择在上传全部成功后删除</td></tr>
          <tr><th scope="row">适用场景</th><td>临时复制、下载或整理文件</td><td>重复发布同一项目或目录</td></tr>
        </tbody></table></div>
        <h3>使用流程</h3>
        <ol>
          <li>保持目标机器的 SSH session 已连接，然后打开“文件传输”视图中的部署入口。</li>
          <li>新建部署配置，填写便于识别的名称，选择本地目录，并填写远端绝对路径。</li>
          <li>点击“保存并预览”。应用会递归扫描两端目录，但此时不会上传或删除文件。</li>
          <li>逐项检查差异，尤其确认远端目录以及“仅远端”项目是否符合预期。</li>
          <li>点击“开始部署”。新增和变化文件会进入普通传输队列，可在传输面板查看进度、速度和失败原因。</li>
          <li>等待传输完成。下次发布时复用相同配置，再次预览并执行即可。</li>
        </ol>
        <h3>差异状态</h3>
        <table className="deployment-status-table"><colgroup><col /><col /></colgroup><tbody>
          <tr><th scope="row">新增</th><td>文件只存在于本地，部署时上传。</td></tr>
          <tr><th scope="row">变化</th><td>两端都有同名文件，但大小或修改时间不同，部署时上传并覆盖远端文件。</td></tr>
          <tr><th scope="row">相同</th><td>文件大小相同，且修改时间差不超过约 2 秒，不会重复上传。</td></tr>
          <tr><th scope="row">仅远端</th><td>文件只存在于远端。默认保留；启用删除选项后才会删除。</td></tr>
        </tbody></table>
        <h3>示例：发布前端构建产物</h3>
        <p>本地构建目录为 <code>~/project/dist</code>，远端站点目录为 <code>/srv/www/app</code>。保存配置后先预览差异，确认无误再部署。以后重新构建时，只需再次预览和执行，不必逐个选择文件。</p>
        <h3>当前判断规则与限制</h3>
        <ul>
          <li>差异依据是文件大小和修改时间，不计算内容哈希；时间比较允许约 2 秒误差。</li>
          <li>变化文件会整文件重新上传，不是 <code>rsync</code> 的分块增量传输。</li>
          <li>部署只处理普通文件；本地或远端目录中存在符号链接时会停止并提示，不会跟随链接。</li>
          <li>部署不是持续监控或双向同步。修改远端文件不会自动下载到本地，之后部署可能覆盖该远端修改。</li>
          <li>远端目录必须对当前 SSH 用户具有读取、创建、覆盖权限；启用删除时还需要删除权限。</li>
          <li>空目录本身不会单独同步；所需父目录会在上传文件时创建。</li>
        </ul>
        <div className="help-warning"><TriangleAlert size={15} aria-hidden="true" /><div><strong>删除远端多余文件</strong><p>启用此选项后，仅当本次所有上传任务都成功时，才会删除预览中标记为“仅远端”的文件。开始前仍应确认远端目录和差异列表。</p></div></div>
      </>
    },
    {
      id: 'tunnels', title: '端口转发', icon: Network,
      searchText: '端口转发 tunnel 隧道 local remote dynamic socks5 监听地址 端口 目标 localhost 127.0.0.1 0.0.0.0',
      content: <>
        <h2>端口转发</h2>
        <p>连接成功后点击 session 工具栏中的“端口转发”。配置按机器保存，但每个运行中的转发只属于启动它的 session，session 断开时会自动停止。</p>
        <h3>本地转发</h3>
        <p>本机程序先连接本机监听端口，流量经过 SSH 加密通道，再由 SSH 服务器连接目标。目标地址是从 SSH 服务器的网络视角解析，不是由本机直接访问。</p>
        <p><code>本机应用 → 本机监听端口 → SSH 服务器 → 目标服务</code></p>
        <ul>
          <li><strong>服务器自身服务：</strong>目标位置选“当前 SSH 服务器”，目标地址自动使用 <code>127.0.0.1</code>。</li>
          <li><strong>内网其他机器：</strong>目标位置选“其他主机”，填写 SSH 服务器能够访问的主机名或内网 IP。</li>
        </ul>
        <h3>本地 Web 服务示例</h3>
        <p>远端 Web 服务监听 <code>127.0.0.1:6000</code> 时，可以配置本机监听 <code>127.0.0.1:16000</code>，目标选择“当前 SSH 服务器”，目标端口填写 <code>6000</code>，然后访问 <code>http://127.0.0.1:16000/</code>。</p>
        <p>监听端口和目标端口不需要相同。Chrome 会阻止部分高风险端口，例如 <code>6000</code>，因此 Web 服务建议使用不同的本机监听端口。</p>
        <h3>其他转发类型</h3>
        <table className="help-detail-table"><tbody>
          <tr><th scope="row">远端转发</th><td>在 SSH 服务器上监听端口，通过 SSH 通道把连接转回当前电脑可访问的目标。目标地址从当前电脑的网络视角解析；服务器是否允许远端监听由 SSH 配置决定。</td></tr>
          <tr><th scope="row">SOCKS5</th><td>在本机创建动态代理。应用程序将代理设置为所显示的监听地址和端口即可。</td></tr>
        </tbody></table>
        <h3>字段说明</h3>
        <ul>
          <li>配置名称可以留空，保存或启动时会根据转发类型、监听端点和目标端点自动生成。</li>
          <li>监听地址通常使用 <code>127.0.0.1</code>；监听端口填 <code>0</code> 时由系统自动分配，启动后显示实际端口。</li>
          <li>macOS 普通应用无法监听 <code>1–1023</code> 的本机特权端口。例如目标端口为 <code>80</code> 时，可使用本机监听端口 <code>18080</code>，目标端口仍填写 <code>80</code>。</li>
          <li>绿色“运行中”表示监听已建立。目标是否可达会在客户端首次连接时验证，出错后状态会显示具体原因。</li>
          <li>SOCKS5 不需要固定目标，目标由使用该代理的客户端请求决定。</li>
        </ul>
        <h3>排查</h3>
        <ul>
          <li>本地转发启动后，可在本机使用 <code>curl -v http://监听地址:监听端口/</code> 或对应协议客户端验证。</li>
          <li>目标在 SSH 服务器自身时，可先在远端终端使用 <code>ss -lntp</code> 确认服务实际监听的地址和端口。</li>
          <li>能够建立连接但没有页面，通常表示目标不是 HTTP 服务、需要 HTTPS，或应用协议不匹配。</li>
        </ul>
        <div className="help-warning"><TriangleAlert size={15} aria-hidden="true" /><div><strong>网络暴露</strong><p>使用 <code>0.0.0.0</code> 或其他外部接口会允许同一网络中的设备访问监听端口。除非明确需要并已配置防火墙，否则使用 <code>127.0.0.1</code>。</p></div></div>
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
          <li>设置中可以调整字体、字号、文字颜色、背景颜色和背景透明度。JetBrains Mono Regular 随应用内置，也可选择检测到的本机等宽字体或手动输入字体名称；中文字符由系统字体补充。</li>
          <li>背景图支持覆盖、完整显示、拉伸和平铺。设置立即应用到已经打开的终端，不需要重新连接。</li>
          <li>应用图标可在“界面”页签中切换。</li>
        </ul>
        <h3>导出诊断</h3>
        <p>在“界面”页签点击“导出诊断信息”生成 JSON 文件。诊断包含应用版本、平台、非敏感连接元数据、session 状态和传输汇总。</p>
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
        <ul>
          <li><strong>Agent 不可用：</strong>确认密钥已加入 Agent，并从具有正确 Agent 环境的终端启动应用；Windows 需启动 OpenSSH Authentication Agent 服务。</li>
          <li><strong>空闲后断线：</strong>启用连接的 session 保活。如果企业网关仍强制回收，缩短保活间隔；长任务使用远端会话管理工具。</li>
          <li><strong>SFTP 较慢：</strong>首次打开会初始化 SFTP 通道。应用会缓存目录并在可用时使用 SSH exec 加速，刷新操作会绕过缓存。</li>
          <li><strong>无法远端转发：</strong>服务器可能禁用了 TCP forwarding 或远端监听。请检查 sshd 的 <code>AllowTcpForwarding</code> 和 <code>GatewayPorts</code> 策略。</li>
          <li><strong>重新连接要密码：</strong>勾选“记住凭据”可使用系统安全存储；如果企业策略禁止长期凭据，断开后的重新认证无法由应用绕过。</li>
        </ul>
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
