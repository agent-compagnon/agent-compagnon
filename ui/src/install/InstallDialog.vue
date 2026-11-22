<script setup lang="ts">
import { computed, ref } from 'vue'
import { buttonVariants } from '@/components/ui/button'
import { fr } from '@/i18n/fr'
import { type InstallAnswer, type InstallData, type Send, failure } from './install'

const props = defineProps<{
  data: InstallData
  send: Send
}>()

type State = 'proposed' | 'working' | 'failed'

const state = ref<State>('proposed')
const autostart = ref(props.data.autostart)
const reason = ref('')
const update = computed(() => props.data.kind === 'update')
const texts = computed(() => (update.value ? fr.install.update : fr.install.install))
const installed = computed(() => props.data.installedVersion ?? '')

// One answer at a time; the application ignores any other anyway. Any refusal leaves the application in its failed
// state, which offers to continue from here or to quit.
async function answer(value: InstallAnswer) {
  if (state.value === 'working') return
  state.value = 'working'
  try {
    await props.send(value)
  } catch (error) {
    reason.value = failure(error)
    state.value = 'failed'
  }
}

function confirm() {
  void answer(update.value ? { choice: 'update' } : { choice: 'install', autostart: autostart.value })
}
</script>

<template>
  <main
    class="flex min-h-dvh flex-col gap-4 p-6"
    :data-state="state"
  >
    <template v-if="state !== 'failed'">
      <h1 class="text-lg font-semibold">
        {{ texts.title }}
      </h1>
      <template v-if="update">
        <p class="versions text-sm">
          {{ fr.install.update.versions(installed, data.version) }}
        </p>
        <p class="text-muted-foreground text-sm">
          {{ fr.install.update.running }}
        </p>
        <p class="text-muted-foreground text-sm">
          {{ fr.install.update.kept }}
        </p>
      </template>
      <template v-else>
        <p class="text-sm">
          {{ fr.install.install.intro(data.version) }}
        </p>
        <dl class="grid grid-cols-[max-content_1fr] gap-x-3 text-sm">
          <dt class="text-muted-foreground">
            {{ fr.install.install.location }}
          </dt>
          <dd class="location font-mono text-xs leading-5 break-all">
            {{ data.location }}
          </dd>
        </dl>
        <p class="text-muted-foreground text-sm">
          {{ fr.install.install.shortcut }}
        </p>
        <label class="bg-muted/50 flex items-start gap-3 rounded-md border p-3">
          <input
            v-model="autostart"
            class="accent-primary mt-0.5 size-4 shrink-0"
            name="autostart"
            type="checkbox"
            :disabled="state === 'working'"
            aria-describedby="autostart-hint"
          >
          <span class="grid gap-1">
            <span class="text-sm font-medium">{{ fr.install.install.autostart }}</span>
            <span
              id="autostart-hint"
              class="text-muted-foreground text-xs"
            >{{ fr.install.install.autostartHint }}</span>
          </span>
        </label>
        <p class="text-muted-foreground text-sm">
          {{ fr.install.install.running }}
        </p>
      </template>
      <div class="mt-auto flex items-center justify-end gap-2">
        <p
          class="working mr-auto text-sm font-medium"
          aria-live="polite"
        >
          <template v-if="state === 'working'">
            {{ texts.working }}
          </template>
        </p>
        <button
          type="button"
          class="later"
          :class="buttonVariants({ variant: 'outline' })"
          :disabled="state === 'working'"
          @click="answer({ choice: 'later' })"
        >
          {{ update ? fr.install.update.later(installed) : fr.install.install.later }}
        </button>
        <button
          type="button"
          class="confirm"
          :class="buttonVariants()"
          :disabled="state === 'working'"
          @click="confirm"
        >
          {{ texts.confirm }}
        </button>
      </div>
    </template>
    <template v-else>
      <h1 class="text-lg font-semibold">
        {{ texts.failedTitle }}
      </h1>
      <p
        class="error border-destructive/40 bg-destructive-muted text-destructive-text rounded-md border px-3 py-2 text-sm"
        role="alert"
      >
        {{ reason }}
      </p>
      <p class="text-muted-foreground text-sm">
        {{ fr.install.failed.fallback }}
      </p>
      <div class="mt-auto flex justify-end gap-2">
        <button
          type="button"
          class="quit"
          :class="buttonVariants({ variant: 'outline' })"
          @click="answer({ choice: 'quit' })"
        >
          {{ fr.install.failed.quit }}
        </button>
        <button
          type="button"
          class="continue"
          :class="buttonVariants()"
          @click="answer({ choice: 'continue' })"
        >
          {{ fr.install.failed.continue }}
        </button>
      </div>
    </template>
  </main>
</template>
