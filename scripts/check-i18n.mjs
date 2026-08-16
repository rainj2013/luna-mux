import fs from 'node:fs'
import path from 'node:path'

const root = process.cwd()
const localeDir = path.join(root, 'app/frontend/src/locales')
const baseFile = 'zh-CN.messages.ts'
const messageFiles = fs.readdirSync(localeDir).filter((name) => name.endsWith('.messages.ts')).sort()

function readCatalog(fileName) {
  const source = fs.readFileSync(path.join(localeDir, fileName), 'utf8')
  const body = source.match(/export default\s+(\{[\s\S]*\})\s+as const\s*$/)?.[1]
  if (!body) throw new Error(`${fileName}: expected an exported object followed by "as const"`)
  const keys = [...body.matchAll(/^\s*"([^"]+)"\s*:/gm)].map((match) => match[1])
  const duplicates = keys.filter((key, index) => keys.indexOf(key) !== index)
  if (duplicates.length) throw new Error(`${fileName}: duplicate keys: ${[...new Set(duplicates)].join(', ')}`)
  return JSON.parse(body)
}

function placeholders(message) {
  return [...message.matchAll(/\{\{\s*([A-Za-z0-9_]+)\s*\}\}/g)].map((match) => match[1]).sort()
}

if (!messageFiles.includes(baseFile)) throw new Error(`Missing base catalog: ${baseFile}`)
const base = readCatalog(baseFile)
const baseKeys = Object.keys(base).sort()
const failures = []

for (const fileName of messageFiles.filter((name) => name !== baseFile)) {
  const catalog = readCatalog(fileName)
  const keys = Object.keys(catalog).sort()
  const missing = baseKeys.filter((key) => !(key in catalog))
  const extra = keys.filter((key) => !(key in base))
  if (missing.length) failures.push(`${fileName}: missing keys: ${missing.join(', ')}`)
  if (extra.length) failures.push(`${fileName}: extra keys: ${extra.join(', ')}`)
  for (const key of baseKeys.filter((candidate) => candidate in catalog)) {
    const expected = placeholders(base[key])
    const actual = placeholders(catalog[key])
    if (expected.join('\0') !== actual.join('\0')) failures.push(`${fileName}: placeholder mismatch for ${key} (${expected.join(', ')} != ${actual.join(', ')})`)
  }
}

function sourceFiles(directory) {
  return fs.readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const absolute = path.join(directory, entry.name)
    return entry.isDirectory() ? sourceFiles(absolute) : /\.tsx?$/.test(entry.name) ? [absolute] : []
  })
}

for (const fileName of sourceFiles(path.join(root, 'app/frontend/src')).filter((name) => !name.startsWith(localeDir))) {
  if (/\btr\s*\(/.test(fs.readFileSync(fileName, 'utf8'))) failures.push(`${path.relative(root, fileName)}: legacy tr() call found`)
}

if (failures.length) {
  process.stderr.write(`${failures.join('\n')}\n`)
  process.exitCode = 1
} else {
  process.stdout.write(`Validated ${messageFiles.length} locale catalogs with ${baseKeys.length} keys.\n`)
}
