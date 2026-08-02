import SSHConfig, { type Section } from 'ssh-config'
import { join } from 'node:path'
import type { SshConfigImportEntry } from '../shared/types'

function values(value: Section['value']): string[] {
  return typeof value === 'string' ? [value] : value.map((item) => item.val)
}

function first(value: string | string[] | undefined): string {
  return Array.isArray(value) ? value[0] ?? '' : value ?? ''
}

function expandHome(path: string, homeDirectory: string): string {
  if (path === '~') return homeDirectory
  return path.startsWith('~/') || path.startsWith('~\\') ? join(homeDirectory, path.slice(2)) : path
}

function proxyAlias(value: string): string {
  const firstHop = value.split(',')[0]?.trim() ?? ''
  if (!firstHop || firstHop.toLowerCase() === 'none') return ''
  const withoutUser = firstHop.includes('@') ? firstHop.slice(firstHop.lastIndexOf('@') + 1) : firstHop
  return withoutUser.replace(/:\d+$/, '')
}

export function parseSshConfig(text: string, homeDirectory: string): SshConfigImportEntry[] {
  const config = SSHConfig.parse(text)
  const aliases = new Set<string>()
  for (const line of config) {
    if (line.type !== SSHConfig.DIRECTIVE || line.param.toLowerCase() !== 'host' || !('config' in line)) continue
    for (const alias of values(line.value)) {
      const trimmed = alias.trim()
      if (trimmed && !trimmed.startsWith('!') && !/[?*[\]]/.test(trimmed)) aliases.add(trimmed)
    }
  }

  const entries: SshConfigImportEntry[] = []
  for (const alias of aliases) {
    const computed = config.compute(alias, { ignoreCase: true, matchExec: false })
    const host = first(computed['hostname']) || alias
    const username = first(computed['user']) || process.env['USER'] || process.env['USERNAME'] || ''
    const parsedPort = Number(first(computed['port']) || 22)
    if (!host || !username || !Number.isInteger(parsedPort) || parsedPort < 1 || parsedPort > 65535) continue
    const identityFile = first(computed['identityfile'])
    entries.push({
      alias,
      name: alias,
      host,
      port: parsedPort,
      username,
      privateKeyPath: identityFile ? expandHome(identityFile, homeDirectory) : '',
      proxyJumpAlias: proxyAlias(first(computed['proxyjump']))
    })
  }
  return entries
}
