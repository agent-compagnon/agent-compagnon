<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
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
  <main class="confirm">
    <h1>Confirmation demandée</h1>
    <dl>
      <dt>Outil</dt>
      <dd class="path">
        {{ data.path }}
      </dd>
      <dt>Demandé par</dt>
      <dd class="client">
        {{ data.client }}
      </dd>
      <dt>Politique</dt>
      <dd class="explanation">
        {{ data.explanation }}
      </dd>
    </dl>
    <h2>Arguments</h2>
    <pre class="arguments">{{ data.arguments }}</pre>
    <p
      class="countdown"
      role="timer"
    >
      <template v-if="expired">
        Délai dépassé : l'appel est refusé.
      </template>
      <template v-else>
        Sans réponse, l'appel sera refusé dans {{ countdown(left) }}.
      </template>
    </p>
    <div class="actions">
      <button
        type="button"
        class="refuse"
        :disabled="sent"
        @click="answer('refuse')"
      >
        Refuser
      </button>
      <button
        type="button"
        class="approve"
        :disabled="sent || expired"
        @click="answer('approve')"
      >
        Autoriser une fois
      </button>
    </div>
  </main>
</template>

<style scoped>
.confirm {
  font-family: system-ui, sans-serif;
  padding: 1.25rem;
}
h1 {
  font-size: 1.1rem;
  margin: 0 0 1rem;
}
h2 {
  font-size: 1rem;
  margin: 1rem 0 0.5rem;
}
dl {
  display: grid;
  grid-template-columns: max-content 1fr;
  gap: 0.25rem 0.75rem;
  margin: 0;
}
dt {
  font-weight: 600;
}
dd {
  margin: 0;
  overflow-wrap: anywhere;
}
.arguments {
  background: #f4f4f4;
  max-height: 16rem;
  overflow: auto;
  padding: 0.5rem;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}
.actions {
  display: flex;
  gap: 0.5rem;
  justify-content: flex-end;
}
</style>
