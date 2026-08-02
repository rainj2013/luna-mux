import { resolve } from 'node:path'
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

export default defineConfig({
  root: resolve('app/frontend'),
  clearScreen: false,
  plugins: [react()],
  server: { port: 1420, strictPort: true },
  envPrefix: ['VITE_', 'TAURI_'],
  build: { outDir: resolve('app/frontend/dist'), emptyOutDir: true }
})
