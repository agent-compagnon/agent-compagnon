<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { buttonVariants } from '@/components/ui/button'
import { fr } from '@/i18n/fr'
import { type ConfirmAnswer, type ConfirmData, countdown } from './detail'

const props = defineProps<{
  data: ConfirmData
  send: (answer: ConfirmAnswer) => void
  /** Milliseconds since an arbitrary origin; `performance.now` in the window. */
  now?: () => number
}>()

const clock = props.now ?? (() => performance.now())
const started = clock()
const left = ref(props.data.remainingMs)
const sent = ref(false)
const expired = computed(() => left.value <= 0)
let timer: ReturnType<typeof setInterval> | undefined

function tick() {
  left.value = Math.max(0, props.data.remainingMs - (clock() - started))
  if (expired.value && timer !== undefined) {
    clearInterval(timer)
    timer = undefined
  }
}

onMounted(() => {
  timer = setInterval(tick, 250)
})
onBeforeUnmount(() => {
  if (timer !== undefined) clearInterval(timer)
})

function answer(value: ConfirmAnswer) {
  tick()
  if (sent.value || (value === 'approve' && expired.value)) return
  sent.value = true
  props.send(value)
}
</script>

<template>
  <main class="flex min-h-dvh flex-col gap-4 p-5">
    <h1 class="text-base font-semibold">
      {{ fr.confirm.title }}
    </h1>
    <dl class="grid grid-cols-[max-content_1fr] gap-x-4 gap-y-1.5 text-sm">
      <dt class="text-muted-foreground">
        {{ fr.confirm.tool }}
      </dt>
      <dd class="path font-medium break-all">
        {{ data.path }}
      </dd>
      <dt class="text-muted-foreground">
        {{ fr.confirm.client }}
      </dt>
      <dd class="client break-all">
        {{ data.client }}
      </dd>
      <dt class="text-muted-foreground">
        {{ fr.confirm.policy }}
      </dt>
      <dd class="explanation">
        {{ data.explanation }}
      </dd>
    </dl>
    <h2 class="text-sm font-semibold">
      {{ fr.confirm.arguments }}
    </h2>
    <pre class="arguments bg-muted max-h-64 overflow-auto rounded-md border p-3 font-mono text-xs break-all whitespace-pre-wrap">{{ data.arguments }}</pre>
    <p
      :class="['countdown text-sm', expired ? 'text-destructive-text' : 'text-attention-text']"
      role="timer"
    >
      <template v-if="expired">
        {{ fr.confirm.expired }}
      </template>
      <template v-else>
        {{ fr.confirm.remaining(countdown(left)) }}
      </template>
    </p>
    <div class="mt-auto flex justify-end gap-2">
      <button
        type="button"
        :class="['refuse', buttonVariants({ variant: 'outline' })]"
        :disabled="sent"
        @click="answer('refuse')"
      >
        {{ fr.confirm.refuse }}
      </button>
      <button
        type="button"
        :class="['approve', buttonVariants()]"
        :disabled="sent || expired"
        @click="answer('approve')"
      >
        {{ fr.confirm.approve }}
      </button>
    </div>
  </main>
</template>
