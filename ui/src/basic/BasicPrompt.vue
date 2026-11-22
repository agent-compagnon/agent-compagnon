<script setup lang="ts">
import { computed, ref } from 'vue'
import { buttonVariants } from '@/components/ui/button'
import { fr } from '@/i18n/fr'
import type { BasicAnswer, BasicPromptData } from './prompt'

const field =
  'border-input bg-background focus-visible:border-ring focus-visible:ring-ring/50 h-9 rounded-md border px-3 text-sm font-normal outline-none focus-visible:ring-3'

const props = defineProps<{
  data: BasicPromptData
  send: (answer: BasicAnswer | null) => void
}>()

const login = ref(props.data.login ?? '')
const password = ref('')
const sent = ref(false)
const attemptLabel = computed(() =>
  props.data.attempt > 1 ? fr.basic.attempt(props.data.attempt, props.data.maxAttempts) : '',
)

// The form is emptied before the answer leaves: nothing typed stays in the page.
function submit() {
  if (sent.value) return
  sent.value = true
  const answer = { login: login.value, password: password.value }
  login.value = ''
  password.value = ''
  props.send(answer)
}

function cancel() {
  if (sent.value) return
  sent.value = true
  login.value = ''
  password.value = ''
  props.send(null)
}
</script>

<template>
  <main class="flex min-h-dvh flex-col gap-4 p-5">
    <h1 class="text-base font-semibold">
      {{ data.title }}
    </h1>
    <p
      v-if="data.error"
      class="error border-destructive/40 bg-destructive-muted text-destructive-text rounded-md border px-3 py-2 text-sm"
      role="alert"
    >
      {{ data.error }}
    </p>
    <p
      v-if="attemptLabel"
      class="attempt text-muted-foreground text-sm"
    >
      {{ attemptLabel }}
    </p>
    <form
      class="grid gap-3"
      autocomplete="off"
      @submit.prevent="submit"
    >
      <label class="grid gap-1.5 text-sm font-medium">
        {{ fr.basic.login }}
        <input
          v-model="login"
          :class="field"
          name="login"
          type="text"
          autocomplete="off"
          spellcheck="false"
          required
        >
      </label>
      <label class="grid gap-1.5 text-sm font-medium">
        {{ fr.basic.password }}
        <input
          v-model="password"
          :class="field"
          name="password"
          type="password"
          autocomplete="off"
          required
        >
      </label>
      <div class="mt-2 flex justify-end gap-2">
        <button
          type="button"
          :class="buttonVariants({ variant: 'outline' })"
          :disabled="sent"
          @click="cancel"
        >
          {{ fr.basic.cancel }}
        </button>
        <button
          type="submit"
          :class="buttonVariants()"
          :disabled="sent"
        >
          {{ fr.basic.submit }}
        </button>
      </div>
    </form>
  </main>
</template>
