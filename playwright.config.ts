import { defineConfig, devices } from '@playwright/test';
import { createHash } from 'node:crypto';

// Not 1420: a running `tauri dev` owns that port without the E2E mocks, and reusing it fails every test.
// One port per checkout (1430-1929, from its path), so a worktree never reuses another checkout's dev server.
// E2E_PORT overrides it.
function checkoutPort(): number {
  const hash = createHash('sha1').update(process.cwd()).digest();
  return 1430 + (hash.readUInt16BE(0) % 500);
}
const devPort = Number(process.env.E2E_PORT) || checkoutPort();

export default defineConfig({
  webServer: {
    command: 'pnpm run dev',
    port: devPort,
    reuseExistingServer: !process.env.CI,
    env: {
      E2E: 'true',
      VITE_PORT: String(devPort),
    },
  },
  testDir: './tests/e2e',
  globalSetup: './tests/e2e/utils/setup.ts',
  globalTeardown: './tests/e2e/utils/teardown.ts',
  use: {
    baseURL: `http://localhost:${devPort}`,
    viewport: { width: 800, height: 900 },
  },
  projects: [
    {
      name: 'chromium',
      use: {
        ...devices['Desktop Chrome'],
      },
    },
  ],
});
