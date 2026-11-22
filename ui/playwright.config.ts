import { defineConfig } from '@playwright/test'

// End-to-end tests of the main window on `vite preview` with the simulated backend (spec 016). Outside of
// `cargo xtask ci`: `npm run e2e`. Screenshots go to `CAPTURES_DIR` when it is set.
export default defineConfig({
  testDir: 'e2e',
  fullyParallel: true,
  reporter: 'list',
  use: {
    baseURL: 'http://127.0.0.1:4173',
    trace: 'off',
    locale: 'fr-FR',
    timezoneId: 'Europe/Paris',
    viewport: { width: 960, height: 680 },
    deviceScaleFactor: 2,
  },
  webServer: {
    command: 'npm run preview:mock -- --host 127.0.0.1',
    url: 'http://127.0.0.1:4173/index.html',
    reuseExistingServer: false,
    timeout: 60_000,
  },
})
