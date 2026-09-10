// Pure path helpers for the in-app file dialog. Every filesystem call still goes through the
// native `files_*` commands; these helpers only split a path into breadcrumb segments and join a
// chosen name back onto the current directory. Both POSIX and Windows separators appear here
// because the dialog runs against whatever host the app is running on.

export interface FileDialogCrumb { name: string; path: string }

export function fileDialogSeparator(path: string): string {
  return path.includes('\\') ? '\\' : '/'
}

export function fileDialogExtension(name: string): string {
  const index = name.lastIndexOf('.')
  return index > 0 && index < name.length - 1 ? name.slice(index + 1).toLowerCase() : ''
}

export function fileDialogJoin(directory: string, name: string): string {
  if (!directory) return name
  const separator = fileDialogSeparator(directory)
  return directory.endsWith(separator) ? directory + name : directory + separator + name
}

// Windows save dialogs append the selected filter's extension; keep the same promise here so
// "dump" still lands as "dump.sql".
export function fileDialogWithExtension(name: string, extensions: string[]): string {
  const primary = extensions[0]
  if (!primary) return name
  const base = name.replace(/\.+$/, '')
  return base.includes('.') || !base ? name : `${base}.${primary}`
}

export function fileDialogSegments(path: string): FileDialogCrumb[] {
  if (!path) return []
  const separator = fileDialogSeparator(path)
  const trimmed = path.replace(/[\\/]+$/, '')
  if (!trimmed) return [{ name: separator === '\\' ? path : '/', path: separator === '\\' ? path : '/' }]
  const drive = /^[a-zA-Z]:$/.exec(trimmed)
  if (drive) return [{ name: `${drive[0]}\\`, path: `${drive[0]}\\` }]
  const parts = trimmed.split(/[\\/]+/).filter(Boolean)
  const crumbs: FileDialogCrumb[] = []
  let rest = parts
  if (/^[\\/]{2}/.test(trimmed) && parts.length >= 2) {
    const root = `\\\\${parts[0]}\\${parts[1]}\\`
    crumbs.push({ name: root, path: root })
    rest = parts.slice(2)
  } else if (/^[a-zA-Z]:/.test(parts[0] ?? '')) {
    const root = `${parts[0]}\\`
    crumbs.push({ name: root, path: root })
    rest = parts.slice(1)
  } else if (trimmed.startsWith('/')) {
    crumbs.push({ name: '/', path: '/' })
  }
  let current = crumbs.length ? crumbs[0]!.path : ''
  for (const part of rest) {
    current = current ? (current.endsWith(separator) ? current + part : `${current}${separator}${part}`) : part
    crumbs.push({ name: part, path: current })
  }
  return crumbs
}
