import { defineConfig } from 'vitest/config'
import vue from '@vitejs/plugin-vue'

export default defineConfig({
  plugins: [vue()],
  clearScreen: false,
  server: { strictPort: true, port: 1420 },
  test: { environment: 'node' },
})
