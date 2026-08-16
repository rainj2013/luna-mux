import { resolve } from 'node:path'
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

export default defineConfig({
  root: resolve('app/frontend'),
  clearScreen: false,
  plugins: [react()],
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
