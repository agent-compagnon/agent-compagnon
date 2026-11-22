import { createApp, h } from 'vue'
import ConfirmDetail from './ConfirmDetail.vue'
import { type ConfirmWindow, confirmData, sendAnswer } from './detail'

const win = window as unknown as ConfirmWindow
const data = confirmData(win)
if (data) {
  createApp({
    render: () =>
      h(ConfirmDetail, {
        data,
        send: (answer) => {
          sendAnswer(win, data.command, answer)
        },
      }),
  }).mount('#confirm')
}
