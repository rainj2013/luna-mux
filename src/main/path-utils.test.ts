import { describe, expect, it } from 'vitest'
import { localParentPath, renamedPath, validateBookmark } from './path-utils'

describe('renamedPath', () => {
  it('adds a counter before an extension for remote paths', () => {
    expect(renamedPath('/tmp/report.pdf', 2, true)).toBe('/tmp/report (2).pdf')
  })
  it('supports Windows paths', () => {
    expect(renamedPath('C:\\Users\\me\\notes.txt', 1, false)).toBe('C:\\Users\\me\\notes (1).txt')
  })
})

describe('validate connection', () => {
  it('reports required fields and invalid ports', () => {
    expect(validateBookmark({ name: '', host: '', port: 0, username: '' })).toHaveLength(4)
  })
  it('accepts a normal bookmark', () => {
    expect(validateBookmark({ name: 'prod', host: 'server.test', port: 22, username: 'root' })).toEqual([])
  })
})

describe('localParentPath', () => {
  it('stays at POSIX root', () => {
    expect(localParentPath('/', 'darwin')).toBe('/')
    expect(localParentPath('/Users/me', 'darwin')).toBe('/Users')
  })

  it('stays at Windows drive and UNC roots', () => {
    expect(localParentPath('C:\\', 'win32')).toBe('C:\\')
    expect(localParentPath('C:\\Users\\me', 'win32')).toBe('C:\\Users')
    expect(localParentPath('\\\\server\\share\\', 'win32')).toBe('\\\\server\\share\\')
    expect(localParentPath('\\\\server\\share\\folder', 'win32')).toBe('\\\\server\\share\\')
  })
})
