import { spawnSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import process from 'node:process'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const iconRoot = path.join(root, 'assets', 'icons')
const sourceRoot = path.join(iconRoot, 'source')
const tauriCli = path.join(root, 'node_modules', '@tauri-apps', 'cli', 'tauri.js')
const variants = ['luna', 'graphite', 'signal', 'light']
const checkOnly = process.argv.includes('--check')
const temporaryRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'luna-mux-icons-'))

function generate(source, output, pngOnly = false) {
  const argumentsList = [tauriCli, 'icon', source, '--output', output]
  if (pngOnly) argumentsList.push('--png', '512')
  const result = spawnSync(process.execPath, argumentsList, { cwd: root, encoding: 'utf8' })
  if (result.status !== 0) {
    throw new Error(result.error?.message || result.stderr || result.stdout || `Icon generation failed for ${source}`)
  }
}

function publish(generatedPath, targetName) {
  const target = path.join(iconRoot, targetName)
  if (checkOnly) {
    if (!fs.existsSync(target) || !fs.readFileSync(generatedPath).equals(fs.readFileSync(target))) {
      throw new Error(`Generated icon is stale: assets/icons/${targetName}`)
    }
    return
  }
  fs.copyFileSync(generatedPath, target)
}

function validatePackagedIcon(targetName) {
  const target = path.join(iconRoot, targetName)
  if (!fs.existsSync(target)) {
    throw new Error(`Packaged icon is missing: assets/icons/${targetName}`)
  }
  const data = fs.readFileSync(target)
  if (targetName.endsWith('.icns') && (data.subarray(0, 4).toString('ascii') !== 'icns' || data.readUInt32BE(4) !== data.length)) {
    throw new Error(`Packaged ICNS is invalid: assets/icons/${targetName}`)
  }
}

try {
  for (const variant of variants) {
    const output = path.join(temporaryRoot, variant)
    generate(path.join(sourceRoot, `${variant}.svg`), output, variant !== 'luna')
    const pngName = fs.readdirSync(output).find((name) => name === '512x512.png' || name === 'icon.png')
    if (!pngName) throw new Error(`Tauri did not generate a PNG for ${variant}`)
    const generatedPng = path.join(output, pngName)
    const committedPng = path.join(iconRoot, `${variant}.png`)
    const sourceChanged = !fs.existsSync(committedPng) || !fs.readFileSync(generatedPng).equals(fs.readFileSync(committedPng))
    publish(generatedPng, `${variant}.png`)
    if (variant === 'luna') {
      publish(path.join(output, 'icon.ico'), 'luna.ico')
      if (!checkOnly && (sourceChanged || !fs.existsSync(path.join(iconRoot, 'luna.icns')))) {
        fs.copyFileSync(path.join(output, 'icon.icns'), path.join(iconRoot, 'luna.icns'))
      }
      validatePackagedIcon('luna.icns')
    }
  }
} finally {
  fs.rmSync(temporaryRoot, { recursive: true, force: true })
}

console.log(`${checkOnly ? 'Verified' : 'Generated'} ${variants.length} Luna Mux icon variants from SVG sources.`)
