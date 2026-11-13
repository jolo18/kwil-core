import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import path from 'node:path'
import { defineConfig } from 'vite'

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: { alias: { '@': path.resolve(__dirname, 'src') } },
  server: {
    port: 5173,
    host: true,
    // In development the app calls /api on its own origin and Vite forwards it to the backend.
    proxy: { '/api': 'http://localhost:8080' },
  },
})
