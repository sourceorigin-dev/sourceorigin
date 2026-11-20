import { defineConfig, type Project } from '@playwright/test';

/** What every project of the documentation's screenshots shares. */
const screenshots: Project = {
  testDir: './tests/browser/screenshots',
  snapshotPathTemplate: 'docs/images/{arg}{ext}',
  // The pages follow the browser's preference, so they show their dark theme.
  use: { browserName: 'chromium', colorScheme: 'dark' }
};

export default defineConfig({
  testDir: './tests/browser',
  workers: 1,
  forbidOnly: Boolean(process.env.CI),
  retries: 0,
  timeout: 60_000,
  use: {
    baseURL: process.env.E2E_AUTH_URL ?? 'http://localhost:3000',
    // Authentication artifacts must never be included in screenshots or traces.
    trace: 'off',
    screenshot: 'off',
    video: 'off',
    launchOptions: { executablePath: process.env.CHROMIUM_EXECUTABLE },
    ignoreHTTPSErrors: Boolean(process.env.E2E_ALLOW_SELF_SIGNED_TLS)
  },
  projects: [
    {
      name: 'chromium',
      testIgnore: ['screenshots/**', 'no-javascript.spec.ts'],
      use: { browserName: 'chromium' }
    },
    // The pages in a browser that runs no script of theirs.
    {
      name: 'no-javascript',
      testMatch: 'no-javascript.spec.ts',
      use: { browserName: 'chromium', javaScriptEnabled: false }
    },
    // The documentation's screenshots, which `just docs screenshots` takes into
    // docs/images. They are not tests, so the suite never runs them. The
    // dashboard's run first, alone on the fresh database, so its overview counts
    // only the administrator it signs up.
    { ...screenshots, name: 'docs-admin', testMatch: 'admin.spec.ts' },
    { ...screenshots, name: 'docs', testIgnore: 'admin.spec.ts', dependencies: ['docs-admin'] }
  ]
});
