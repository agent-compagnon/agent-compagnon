import { createApp, h } from 'vue'
import App from './App.vue'
import type { Backend } from './api/backend'
import { insideTauri, tauriBackend } from './api/tauri'
import { createStore } from './store/snapshot'
import './style.css'

// The simulated backend exists only in `--mode mock`: in the application's build this branch is removed, and
// `scripts/check-dist.mjs` checks that nothing of it is left. Without Tauri and without the mock, nothing is shown.
async function backend(): Promise<Backend | null> {
  if (import.meta.env.MODE === 'mock') {
    const mock = await import('./api/mock')
    return mock.createMockBackend(mock.scenarioOf(window.location.search))
  }
  return insideTauri() ? tauriBackend : null
}

void backend().then((chosen) => {
  const store = chosen ? createStore(chosen) : null
  createApp({ render: () => h(App, { store }) }).mount('#app')
})
