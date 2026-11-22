<script setup lang="ts">
import { computed, ref } from 'vue'
import type { BasicAnswer, BasicPromptData } from './prompt'

const props = defineProps<{
  data: BasicPromptData
  send: (answer: BasicAnswer | null) => void
}>()

const login = ref(props.data.login ?? '')
const password = ref('')
const sent = ref(false)
const attemptLabel = computed(() =>
  props.data.attempt > 1 ? `Essai ${String(props.data.attempt)} sur ${String(props.data.maxAttempts)}` : '',
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
  <main class="basic">
    <h1>{{ data.title }}</h1>
    <p
      v-if="data.error"
      class="error"
      role="alert"
    >
      {{ data.error }}
    </p>
    <p
      v-if="attemptLabel"
      class="attempt"
    >
      {{ attemptLabel }}
    </p>
    <form
      autocomplete="off"
      @submit.prevent="submit"
    >
      <label>
        Identifiant
        <input
          v-model="login"
          name="login"
          type="text"
          autocomplete="off"
          spellcheck="false"
          required
        >
      </label>
      <label>
        Mot de passe
        <input
          v-model="password"
          name="password"
          type="password"
          autocomplete="off"
          required
        >
      </label>
      <div class="actions">
        <button
          type="button"
          :disabled="sent"
          @click="cancel"
        >
          Annuler
        </button>
        <button
          type="submit"
          :disabled="sent"
        >
          Se connecter
        </button>
      </div>
    </form>
  </main>
</template>

<style scoped>
.basic {
  font-family: system-ui, sans-serif;
  padding: 1.25rem;
}
h1 {
  font-size: 1.1rem;
  margin: 0 0 1rem;
}
form {
  display: grid;
  gap: 0.75rem;
}
label {
  display: grid;
  gap: 0.25rem;
}
.error {
  color: #b00020;
}
.actions {
  display: flex;
  gap: 0.5rem;
  justify-content: flex-end;
}
</style>
