import { createHash } from 'node:crypto'
import { mkdir, mkdtemp, readFile, rename, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'

// portable-pty loads these beside the executable. The inbox Windows ConPTY
// drops SIXEL and capability queries before xterm can receive them.
if (process.platform !== 'win32') process.exit(0)
if (process.arch !== 'x64') throw new Error('The Windows ConPTY bundle requires x64')

const version = '1.24.261001001'
const archiveHash = '4d6aaddc1d2385c9f5897df28f33879f699f8f2783315d5204cf3d8c3616ac5f'
const files = [
  { name: 'conpty.dll', source: 'runtimes/win-x64/native/conpty.dll', sha256: '5afea6c9480e2c7dfd195376dd10996b0ae25cdc6f014e6247fd8aedc8c34852' },
  { name: 'OpenConsole.exe', source: 'build/native/runtimes/x64/OpenConsole.exe', sha256: '6f8e68dec4e8e5e15a54ecc8afa127b0e62139edc004ffbb4892125ff01d5737' }
]
const destination = path.resolve(import.meta.dirname, '../app/native/binaries/conpty')
const digest = bytes => createHash('sha256').update(bytes).digest('hex')
const valid = await Promise.all(files.map(async file => {
  try { return digest(await readFile(path.join(destination, file.name))) === file.sha256 }
  catch (error) { if (error.code === 'ENOENT') return false; throw error }
}))
if (valid.every(Boolean)) {
  console.log(`Windows ConPTY ${version} is ready`)
  process.exit(0)
}

const temporary = await mkdtemp(path.join(tmpdir(), 'luna-mux-conpty-'))
try {
  const url = `https://api.nuget.org/v3-flatcontainer/microsoft.windows.console.conpty/${version}/microsoft.windows.console.conpty.${version}.nupkg`
  const response = await fetch(url, { signal: AbortSignal.timeout(60000) })
  if (!response.ok) throw new Error(`ConPTY download failed: HTTP ${response.status}`)
  const archive = Buffer.from(await response.arrayBuffer())
  if (digest(archive) !== archiveHash) throw new Error('ConPTY archive checksum mismatch')
  const archivePath = path.join(temporary, 'conpty.zip')
  const extracted = path.join(temporary, 'extracted')
  await writeFile(archivePath, archive)
  const powershell = path.join(process.env.SystemRoot, 'System32/WindowsPowerShell/v1.0/powershell.exe')
  const result = spawnSync(powershell, ['-NoLogo', '-NoProfile', '-NonInteractive', '-Command',
    'Expand-Archive -LiteralPath $env:LUNA_CONPTY_ARCHIVE -DestinationPath $env:LUNA_CONPTY_EXTRACTED -ErrorAction Stop'], {
    env: { ...process.env, LUNA_CONPTY_ARCHIVE: archivePath, LUNA_CONPTY_EXTRACTED: extracted },
    encoding: 'utf8', windowsHide: true, timeout: 60000, maxBuffer: 1024 * 1024
  })
  if (result.error) throw result.error
  if (result.status !== 0) throw new Error(`ConPTY extraction failed: ${result.stderr}`)
  // Verify both files before replacing either member of the matched pair.
  const binaries = await Promise.all(files.map(async file => {
    const bytes = await readFile(path.join(extracted, file.source))
    if (digest(bytes) !== file.sha256) throw new Error(`ConPTY ${file.name} checksum mismatch`)
    return { ...file, bytes }
  }))
  await mkdir(destination, { recursive: true })
  for (const file of binaries) {
    const target = path.join(destination, file.name)
    await writeFile(`${target}.download`, file.bytes)
    await rename(`${target}.download`, target)
  }
  console.log(`Downloaded Windows ConPTY ${version}`)
} finally {
  await rm(temporary, { recursive: true, force: true })
}
