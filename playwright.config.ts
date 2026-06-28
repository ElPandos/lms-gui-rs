import { defineConfig, devices } from '@playwright/test';

/**
 * Playwright config for LMS GUI (Rust/Axum dashboard).
 *
 * The webServer block auto-starts `cargo run` in local mode (LMS_LOCAL=1)
 * on port 3001, so tests can run without a live LM Studio instance.
 * Playwright waits for /api/health to respond before running tests.
 *
 * Usage:
 *   npx playwright test                 # run all e2e tests
 *   npx playwright test --ui            # interactive UI mode
 *   npx playwright test --debug         # step-through debugging
 *   npx playwright test --headed        # show browser window
 */
export default defineConfig({
  testDir: './tests',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 1 : undefined,
  reporter: [
    ['html', { open: 'never' }],
    ['list'],
  ],
  use: {
    baseURL: 'http://localhost:3001',
    trace: 'on-first-retry',
    screenshot: 'only-on-failure',
    video: 'retain-on-failure',
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
  webServer: {
    command: 'LMS_LOCAL=1 LMS_PORT=3001 cargo run',
    url: 'http://localhost:3001/api/health',
    reuseExistingServer: !process.env.CI,
    timeout: 120_000, // cargo run may need to compile on first run
  },
});
