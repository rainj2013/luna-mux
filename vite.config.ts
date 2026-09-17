import { resolve } from 'node:path'
import { defineConfig, type Plugin } from 'vite'
import react from '@vitejs/plugin-react'

const pad = (value: number): string => String(value).padStart(2, '0')

function buildTimeText(): string {
  const at = new Date()
  return `${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())} ${pad(at.getHours())}:${pad(at.getMinutes())}:${pad(at.getSeconds())}`
}

const buildTimeModule = 'virtual:build-time'
const resolvedBuildTimeModule = `\0${buildTimeModule}`

// Packaging time is served as a real module instead of a `define` constant: `define`
// replacements do not reach the JSX transform path in dev, which would leave the
// identifier undefined at runtime there.
function buildTimePlugin(): Plugin {
  const buildTime = buildTimeText()
  return {
    name: 'luna-mux-build-time',
    resolveId: (id) => (id === buildTimeModule ? resolvedBuildTimeModule : null),
    load: (id) => (id === resolvedBuildTimeModule ? `export const BUILD_TIME = ${JSON.stringify(buildTime)}\n` : null)
  }
}

export default defineConfig({
  root: resolve('app/frontend'),
  clearScreen: false,
  plugins: [react(), buildTimePlugin()],
  server: { port: 1420, strictPort: true },
  envPrefix: ['VITE_', 'TAURI_'],
  build: {
    outDir: resolve('app/frontend/dist'),
    emptyOutDir: true,
    rollupOptions: {
      output: {
        manualChunks: {
          react: ['react', 'react-dom'],
          tauri: ['@tauri-apps/api', '@tauri-apps/plugin-clipboard-manager', '@tauri-apps/plugin-dialog', '@tauri-apps/plugin-opener'],
          terminal: ['@xterm/xterm', '@xterm/addon-fit', '@xterm/addon-search', '@xterm/addon-web-links', '@xterm/addon-webgl'],
          icons: ['lucide-react']
        }
      }
    }
  }
})
