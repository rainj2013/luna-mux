import { createHash } from 'node:crypto'
import { chmod, mkdir, readFile, rename, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import process from 'node:process'

const version = '0.36.0'
const releases = {
  'win32-x64': {
    asset: 'agent-browser-win32-x64.exe',
    target: 'x86_64-pc-windows-msvc',
    extension: '.exe',
    sha256: '412ff72737a109e93f5304b0ff76c988fb6f1f451d0fc7e010577922bcc20ff3'
  },
  'darwin-x64': {
    asset: 'agent-browser-darwin-x64',
    target: 'x86_64-apple-darwin',
    extension: '',
    sha256: '45d9ac061a7d72e61eaff905326e2e19365f4dadb12142ea2f2d76d84689c708'
  },
  'darwin-arm64': {
    asset: 'agent-browser-darwin-arm64',
    target: 'aarch64-apple-darwin',
    extension: '',
    sha256: 'b2106ab39db0838e7b1772f7f26f760518de56d09053150c56f9dddf15af997d'
  }
}

const release = releases[`${process.platform}-${process.arch}`]
if (!release) {
  throw new Error(`agent-browser ${version} has no Luna Mux sidecar for ${process.platform}-${process.arch}`)
}

const repoRoot = path.resolve(import.meta.dirname, '..')
const binaryDir = path.join(repoRoot, 'app', 'native', 'binaries')
const destination = path.join(binaryDir, `agent-browser-${release.target}${release.extension}`)
const temporary = `${destination}.download`
const digest = (bytes) => createHash('sha256').update(bytes).digest('hex')

await mkdir(binaryDir, { recursive: true })
try {
  const installed = await readFile(destination)
  if (digest(installed) === release.sha256) {
    if (process.platform !== 'win32') await chmod(destination, 0o755)
    console.log(`agent-browser ${version} sidecar is ready`)
    process.exit(0)
  }
} catch (error) {
  if (error?.code !== 'ENOENT') throw error
}

const url = `https://github.com/vercel-labs/agent-browser/releases/download/v${version}/${release.asset}`
const response = await fetch(url, { redirect: 'follow' })
if (!response.ok) throw new Error(`failed to download ${url}: HTTP ${response.status}`)
const bytes = Buffer.from(await response.arrayBuffer())
const actual = digest(bytes)
if (actual !== release.sha256) {
  throw new Error(`agent-browser ${version} checksum mismatch: expected ${release.sha256}, received ${actual}`)
}

await rm(temporary, { force: true })
await writeFile(temporary, bytes)
if (process.platform !== 'win32') await chmod(temporary, 0o755)
await rm(destination, { force: true })
await rename(temporary, destination)
console.log(`downloaded agent-browser ${version} sidecar for ${release.target}`)
