import vue from '@vitejs/plugin-vue'
import { defineConfig } from 'vite'

// https://vite.dev/config/
export default defineConfig({
  plugins: [vue()],
  server: {
    port: 1420,
    strictPort: true,
  },
  optimizeDeps: {
    include: ['@lucide/vue', 'qrcode.vue'],
  },
  clearScreen: false,
})
