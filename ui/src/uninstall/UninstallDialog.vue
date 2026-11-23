<script setup lang="ts">
import { computed, ref } from 'vue'
import { buttonVariants } from '@/components/ui/button'
import { fr, list } from '@/i18n/fr'
import { type Send, type UninstallAnswer, type UninstallData, type UninstallReport, failure } from './uninstall'

const props = defineProps<{
  data: UninstallData
  send: Send
}>()

type State = 'proposed' | 'working' | 'failed' | 'reported'

const state = ref<State>('proposed')
const removeClients = ref(true)
const reason = ref('')
const report = ref<UninstallReport | null>(null)

const ours = computed(() => props.data.clients.filter((c) => c.entry === 'ours'))
const others = computed(() => props.data.clients.filter((c) => c.entry !== 'ours'))
const clientsHint = computed(() => {
  if (ours.value.length === 0) return fr.uninstall.noClient
  const labels = list(ours.value.map((c) => c.label))
  return removeClients.value ? fr.uninstall.clientsHint(labels) : fr.uninstall.clientsKept(labels)
})
const complete = computed(
  () => report.value !== null && !report.value.keptInstalled && !report.value.left.some((l) => l.failed),
)

// One answer at a time; the application ignores any other anyway. A refusal means nothing was changed: the window
// says why and offers to try again.
async function answer(value: UninstallAnswer) {
  if (state.value === 'working') return
  const uninstalling = value.choice === 'uninstall'
  if (uninstalling) state.value = 'working'
  try {
    const reply = await props.send(value)
    if (uninstalling && reply !== null) {
      report.value = reply
      state.value = 'reported'
    }
  } catch (error) {
    reason.value = failure(error)
    state.value = 'failed'
  }
}

function uninstall() {
  void answer({ choice: 'uninstall', removeClients: removeClients.value })
}
</script>

<template>
  <main
    class="flex h-dvh flex-col gap-4 p-6"
    :data-state="state"
  >
    <template v-if="state === 'proposed' || state === 'working'">
      <h1 class="text-lg font-semibold">
        {{ fr.uninstall.title }}
      </h1>
      <div class="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto">
        <p class="text-sm">
          {{ fr.uninstall.intro(data.version) }}
        </p>
        <section aria-labelledby="removed-title">
          <h2
            id="removed-title"
            class="text-sm font-medium"
          >
            {{ fr.uninstall.removedTitle }}
          </h2>
          <ul class="removed text-muted-foreground mt-1 list-disc space-y-0.5 pl-5 text-sm">
            <li>{{ fr.uninstall.removed.program(data.location) }}</li>
            <li>{{ fr.uninstall.removed.data(data.data) }}</li>
            <li>{{ fr.uninstall.removed.windows }}</li>
            <li>{{ fr.uninstall.removed.registry }}</li>
          </ul>
        </section>
        <label class="bg-muted/50 flex items-start gap-3 rounded-md border p-3">
          <input
            v-model="removeClients"
            class="accent-primary mt-0.5 size-4 shrink-0"
            name="removeClients"
            type="checkbox"
            :disabled="state === 'working'"
            aria-describedby="clients-hint"
          >
          <span class="grid gap-1">
            <span class="text-sm font-medium">{{ fr.uninstall.clients }}</span>
            <span
              id="clients-hint"
              class="text-muted-foreground text-xs"
            >{{ clientsHint }}</span>
          </span>
        </label>
        <section aria-labelledby="kept-title">
          <h2
            id="kept-title"
            class="text-sm font-medium"
          >
            {{ fr.uninstall.keptTitle }}
          </h2>
          <ul class="kept text-muted-foreground mt-1 list-disc space-y-0.5 pl-5 text-sm">
            <li
              v-for="client in others"
              :key="client.location"
              class="break-words"
            >
              {{
                client.entry === 'other'
                  ? fr.uninstall.kept.other(client.label, client.detail)
                  : fr.uninstall.kept.unreadable(client.label, client.detail)
              }}
            </li>
            <li>{{ fr.uninstall.kept.backups }}</li>
            <li>{{ fr.uninstall.kept.elsewhere }}</li>
            <li>{{ fr.uninstall.kept.secrets }}</li>
          </ul>
        </section>
        <p class="text-sm">
          {{ fr.uninstall.closeClients }}
        </p>
      </div>
      <div class="flex items-center justify-end gap-2">
        <p
          class="working mr-auto text-sm font-medium"
          aria-live="polite"
        >
          <template v-if="state === 'working'">
            {{ fr.uninstall.working }}
          </template>
        </p>
        <button
          type="button"
          class="cancel"
          :class="buttonVariants({ variant: 'outline' })"
          :disabled="state === 'working'"
          @click="answer({ choice: 'cancel' })"
        >
          {{ fr.uninstall.cancel }}
        </button>
        <button
          type="button"
          class="confirm"
          :class="buttonVariants()"
          :disabled="state === 'working'"
          @click="uninstall"
        >
          {{ fr.uninstall.confirm }}
        </button>
      </div>
    </template>
    <template v-else-if="state === 'failed'">
      <h1 class="text-lg font-semibold">
        {{ fr.uninstall.failedTitle }}
      </h1>
      <p
        class="error border-destructive/40 bg-destructive-muted text-destructive-text rounded-md border px-3 py-2 text-sm"
        role="alert"
      >
        {{ reason }}
      </p>
      <div class="mt-auto flex justify-end gap-2">
        <button
          type="button"
          class="close"
          :class="buttonVariants({ variant: 'outline' })"
          @click="answer({ choice: 'cancel' })"
        >
          {{ fr.uninstall.close }}
        </button>
        <button
          type="button"
          class="retry"
          :class="buttonVariants()"
          @click="uninstall"
        >
          {{ fr.uninstall.retry }}
        </button>
      </div>
    </template>
    <template v-else-if="report !== null">
      <h1 class="text-lg font-semibold">
        {{ complete ? fr.uninstall.doneTitle : fr.uninstall.partialTitle }}
      </h1>
      <div class="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto">
        <section
          v-if="report.removed.length + report.atClose.length > 0"
          aria-labelledby="done-removed"
        >
          <h2
            id="done-removed"
            class="text-sm font-medium"
          >
            {{ fr.uninstall.doneRemoved }}
          </h2>
          <ul class="done-removed text-muted-foreground mt-1 list-disc space-y-0.5 pl-5 text-sm">
            <li
              v-for="item in report.removed"
              :key="item"
            >
              {{ item }}
            </li>
            <li
              v-for="item in report.atClose"
              :key="item"
            >
              {{ fr.uninstall.atClose(item) }}
            </li>
          </ul>
        </section>
        <section
          v-if="report.left.length > 0"
          aria-labelledby="done-left"
        >
          <h2
            id="done-left"
            class="text-sm font-medium"
          >
            {{ fr.uninstall.doneLeft }}
          </h2>
          <ul class="done-left mt-1 list-disc space-y-1 pl-5 text-sm">
            <li
              v-for="item in report.left"
              :key="item.what"
              class="break-words"
              :class="item.failed ? 'failed text-destructive-text' : 'text-muted-foreground'"
            >
              <span class="font-medium">{{ item.what }}</span> : {{ item.why }}
            </li>
          </ul>
        </section>
        <p
          v-if="report.keptInstalled"
          class="kept-installed text-sm"
        >
          {{ fr.uninstall.keptInstalled }}
        </p>
      </div>
      <div class="flex justify-end">
        <button
          type="button"
          class="close"
          :class="buttonVariants()"
          @click="answer({ choice: 'close' })"
        >
          {{ fr.uninstall.close }}
        </button>
      </div>
    </template>
  </main>
</template>
