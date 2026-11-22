import vue from '@vitejs/plugin-vue'
import { defineConfig } from 'vitest/config'

// Served by Tauri from `ui/dist` (spec 011): relative paths, no external resource, nothing inlined that the CSP
// would refuse.
export default defineConfig({
  plugins: [vue()],
  base: './',
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    target: 'es2022',
    assetsInlineLimit: 0,
    // The main window, the Basic input window of spec 012 and the detail window of the confirmations of spec 013
    // (local pages, no remote content).
    rolldownOptions: {
      input: {
        main: 'index.html',
        basic: 'basic.html',
        confirm: 'confirm.html',
      },
    },
  },
  // The capture script of the login windows lives in the `auth` crate (spec 012); its tests here read it raw.
  server: {
    fs: {
      allow: ['.', '../crates/auth/src/webview'],
    },
  },
  test: {
    environment: 'jsdom',
  },
})
