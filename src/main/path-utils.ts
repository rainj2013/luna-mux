import { posix, win32 } from 'node:path'

export function renamedPath(path: string, attempt: number, remote: boolean): string {
  const api = remote ? posix : (path.includes('\\') ? win32 : posix)
  const directory = api.dirname(path)
  const extension = api.extname(path)
  const base = api.basename(path, extension)
  return api.join(directory, `${base} (${attempt})${extension}`)
}

export function localParentPath(path: string, platform: NodeJS.Platform = process.platform): string {
  const api = platform === 'win32' ? win32 : posix
  const normalized = api.normalize(path)
  const root = api.parse(normalized).root
  if (normalized === root) return root
  const parent = api.dirname(normalized)
  return parent.length < root.length ? root : parent
}

export function validateBookmark(input: { name: string; host: string; port: number; username: string }): string[] {
  const errors: string[] = []
  if (!input.name.trim()) errors.push('请输入连接名称')
  if (!input.host.trim()) errors.push('请输入 Host 或 IP')
  if (!Number.isInteger(input.port) || input.port < 1 || input.port > 65535) errors.push('端口必须在 1 到 65535 之间')
  if (!input.username.trim()) errors.push('请输入用户名')
  return errors
}
