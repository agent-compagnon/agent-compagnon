import { createApp, h } from 'vue'
import '../style.css'
import UninstallDialog from './UninstallDialog.vue'
import { type Send, type UninstallData, type UninstallWindow, sender, uninstallData } from './uninstall'

const win = window as unknown as UninstallWindow

function mount(data: UninstallData, send: Send) {
  createApp({ render: () => h(UninstallDialog, { data, send }) }).mount('#uninstall')
}

// The simulated uninstallation exists only in `--mode mock` (screenshots and end-to-end tests): the application's
// build keeps the first branch alone.
if (import.meta.env.MODE === 'mock') {
  void import('./mock').then(({ simulated }) => {
    const { data, send } = simulated(window.location.search)
    mount(data, send)
  })
} else {
  const data = uninstallData(win)
  if (data) mount(data, sender(win, data.command))
}
