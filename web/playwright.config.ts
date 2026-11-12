import { defineConfig, devices } from '@playwright/test'

// Starts the backend (`kwil dev`, demo AI, local Postgres) and the web app unless they are
// already running. Set PW_CHROMIUM to use a pre-installed browser binary.
export default defineConfig({
  testDir: './e2e',
  timeout: 120_000,
  expect: { timeout: 20_000 },
  fullyParallel: false,
  workers: 1,
  reporter: 'list',
  use: {
    baseURL: 'http://localhost:5173',
    trace: 'off',
    launchOptions: process.env.PW_CHROMIUM ? { executablePath: process.env.PW_CHROMIUM } : {},
  },
  projects: [
    { name: 'desktop', use: { ...devices['Desktop Chrome'] } },
  ],
  webServer: [
    {
      command: 'cd ../backend && cargo run -q -- dev',
      url: 'http://localhost:8080/healthz',
      reuseExistingServer: true,
      timeout: 300_000,
      env: {
        DATABASE_URL: process.env.DATABASE_URL ?? 'postgres://postgres@localhost:5432/kwil',
        AI_PROVIDER: 'demo',
        AUTH_MODE: 'dev',
        BLOG_BASE_DOMAIN: 'localhost',
      },
    },
    { command: 'npx vite --port 5173', url: 'http://localhost:5173', reuseExistingServer: true, timeout: 60_000 },
  ],
})
