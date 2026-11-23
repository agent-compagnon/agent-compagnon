import { fileURLToPath, URL } from 'node:url'
import tailwindcss from '@tailwindcss/vite'
import vue from '@vitejs/plugin-vue'
import { defineConfig } from 'vitest/config'

// The CSP of the application (`app/tauri.conf.json`), also served by `vite preview` so that the end-to-end tests
// see what the WebView refuses.
const CSP = [
  "default-src 'self'",
  "script-src 'self'",
  "style-src 'self'",
  "img-src 'self' data:",
  "font-src 'self'",
  'connect-src ipc: http://ipc.localhost',
  "object-src 'none'",
  "base-uri 'none'",
  "form-action 'none'",
  "frame-ancestors 'none'",
].join('; ')

// Served by Tauri from `ui/dist` (spec 011): relative paths, no external resource, nothing inlined that the CSP
// would refuse. `--mode mock` builds the window with the simulated backend into `dist-mock/` (spec 016): never the
// application's build.
export default defineConfig(({ mode }) => ({
  plugins: [vue(), tailwindcss()],
  base: './',
  resolve: {
    alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) },
  },
  build: {
    outDir: mode === 'mock' ? 'dist-mock' : 'dist',
    emptyOutDir: true,
    target: 'es2022',
    assetsInlineLimit: 0,
    // The main window, the Basic input window of spec 012, the detail window of the confirmations of spec 013, the
    // installation window of spec 019 and the uninstallation window of spec 022 (local pages, no remote content).
    rolldownOptions: {
      input: {
        main: 'index.html',
        basic: 'basic.html',
        confirm: 'confirm.html',
        install: 'install.html',
        uninstall: 'uninstall.html',
      },
    },
  },
  // The capture script of the login windows lives in the `auth` crate (spec 012), the commands of the window are
  // declared in `app/` (spec 016): their tests here read them raw.
  server: {
    fs: {
      allow: ['.', '../crates/auth/src/webview', '../app'],
    },
  },
  preview: {
    headers: { 'Content-Security-Policy': CSP },
  },
  test: {
    environment: 'jsdom',
    // `src/theme.spec.ts` reads the tokens of `style.css` raw.
    css: { include: [/style\.css/] },
    include: ['src/**/*.spec.ts'],
  },
}))
