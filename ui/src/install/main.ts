import { createApp, h } from 'vue'
import '../style.css'
import InstallDialog from './InstallDialog.vue'
import { type InstallData, type InstallWindow, type Send, installData, sender } from './install'

const win = window as unknown as InstallWindow

function mount(data: InstallData, send: Send) {
  createApp({ render: () => h(InstallDialog, { data, send }) }).mount('#install')
}

// The simulated installation exists only in `--mode mock` (screenshots and end-to-end tests): the application's
// build keeps the first branch alone.
if (import.meta.env.MODE === 'mock') {
  void import('./mock').then(({ simulated }) => {
    const { data, send } = simulated(window.location.search)
    mount(data, send)
  })
} else {
  const data = installData(win)
  if (data) mount(data, sender(win, data.command))
}
