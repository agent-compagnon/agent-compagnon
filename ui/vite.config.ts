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
  },
  test: {
    environment: 'jsdom',
  },
})
