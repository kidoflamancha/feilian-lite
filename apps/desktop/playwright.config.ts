import { defineConfig } from '@playwright/test'

export default defineConfig({
  testDir: './tests',
  reporter: 'line',
  use: {
    baseURL: 'http://127.0.0.1:1420',
    screenshot: 'only-on-failure',
  },
  webServer: {
    command: 'npm run dev -- --host 127.0.0.1',
    url: 'http://127.0.0.1:1420',
    reuseExistingServer: false,
  },
})