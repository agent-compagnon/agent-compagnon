import { createApp, h } from 'vue'
import '../style.css'
import BasicPrompt from './BasicPrompt.vue'
import { type BasicWindow, promptData, sendAnswer } from './prompt'

const win = window as unknown as BasicWindow
const data = promptData(win)
if (data) {
  createApp({
    render: () =>
      h(BasicPrompt, {
        data,
        send: (answer) => {
          sendAnswer(win, data.command, answer)
        },
      }),
  }).mount('#basic')
}
