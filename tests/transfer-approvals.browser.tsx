import { useState } from 'react'
import { createRoot } from 'react-dom/client'
import { AgentWorkbench } from '../app/frontend/src/components/AgentWorkbench'
import { TransferApprovalCards, TransferApprovalNotice, useTransferApprovals } from '../app/frontend/src/components/TransferApprovalCards'
import { I18nProvider } from '../app/frontend/src/i18n'
import type { AppApi, PendingTransferApproval } from '../app/frontend/src/types'
import '../app/frontend/src/styles.css'

let requests: PendingTransferApproval[] = []
let serial = 0
let failResolve = false
let failReads = false
const decisions: Array<{ id: string; approved: boolean }> = []
const queued: string[] = []
function add(direction: 'upload' | 'download'): void {
  const id = `fixture-${++serial}`
  requests.push({
    approval: { approvalId: id, callerId: 'agent-runtime:fixture', expiresAt: new Date(Date.now() + 300000).toISOString() },
    request: { sessionId: 'ssh-runtime', direction, sourcePaths: direction === 'upload' ? ['D:\\工作目录\\demo project\\config.json', 'D:\\工作目录\\demo project\\src\\main.ts'] : ['/srv/demo/logs/app.log'], destinationDirectory: direction === 'upload' ? '/srv/demo/project' : 'D:\\downloads' },
    destinationPaths: direction === 'upload' ? ['/srv/demo/project/config.json', '/srv/demo/project/main.ts'] : ['D:\\downloads\\app.log'],
    callerName: 'Codex · 实现部署配置', targetName: '开发服务器 · developer@192.0.2.20:22', muxSessionId: 'fixture-session',
  })
}
add('upload')
window.api = {
  transferApprovals: {
    async list() {
      if (failReads) throw new Error('读取审批列表失败（测试）')
      return structuredClone(requests)
    },
    async resolve(id: string, approved: boolean) {
      await new Promise((resolve) => setTimeout(resolve, 350))
      if (failResolve) { failResolve = false; throw new Error('审批处理失败（测试）') }
      if (!requests.some((request) => request.approval.approvalId === id)) throw new Error('审批已取消或过期（测试）')
      decisions.push({ id, approved })
      if (approved) queued.push(id)
      requests = requests.filter((request) => request.approval.approvalId !== id)
    },
  },
  settings: {
    async getAgentLaunch() { return { codexYolo: false, claudeCodeYolo: false } },
    async getAgentSftpEnabled() { return false },
  },
} as AppApi
Object.assign(window, { transferApprovalFixture: { decisions, queued, pending: () => structuredClone(requests) } })

function Fixture(): React.JSX.Element {
  const [dark, setDark] = useState(false)
  const [overview, setOverview] = useState(false)
  const state = useTransferApprovals()
  return <I18nProvider language="zh-CN" setLanguage={() => {}}><div className="app-shell" data-theme={dark ? 'dark' : 'light'} style={{ display: 'flex', flexDirection: 'column', minHeight: '100vh' }}>
    <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8, padding: 12 }}>
      <button onClick={() => { add('upload'); state.retry() }}>新增上传</button>
      <button onClick={() => { add('download'); state.retry() }}>新增下载</button>
      <button onClick={() => { requests = []; state.retry() }}>取消全部请求</button>
      <button onClick={() => { failResolve = true }}>下次审批失败</button>
      <button onClick={() => { failReads = !failReads; state.retry() }}>切换读取失败</button>
      <button onClick={() => setDark((value) => !value)}>切换主题</button>
      <button onClick={() => setOverview((value) => !value)}>切换工作区</button>
    </div>
    {overview ? <AgentWorkbench approvalCount={state.pending.length} approvals={<TransferApprovalCards state={state} />}><p>Agent 运行状态</p></AgentWorkbench> : <main style={{ padding: 20 }}>终端工作区（审批不会抢走焦点）</main>}
    {!overview && state.pending.length > 0 && <TransferApprovalNotice count={state.pending.length} onOpen={() => setOverview(true)} />}
  </div></I18nProvider>
}
createRoot(document.getElementById('root')!).render(<Fixture />)
