import { onBeforeUnmount, onMounted, ref, type Ref } from 'vue'

/** The current time, refreshed every 30 s (the « jusqu'à demain » labels change at midnight). */
export function useNow(period = 30_000): Ref<number> {
  const now = ref(Date.now())
  let timer: ReturnType<typeof setInterval> | undefined
  onMounted(() => {
    timer = setInterval(() => {
      now.value = Date.now()
    }, period)
  })
  onBeforeUnmount(() => {
    if (timer !== undefined) clearInterval(timer)
  })
  return now
}
