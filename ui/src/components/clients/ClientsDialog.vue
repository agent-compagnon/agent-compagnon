<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import {
  AlertDialog,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'
import { reason } from '@/api/backend'
import type { ApplyOutcome, ClientAction, ClientLocation, ClientPreview, ClientsView, LocationId } from '@/api/types'
import { fr } from '@/i18n/fr'
import { useStore } from '@/store/snapshot'
import {
  applicable,
  canInstall,
  canRemove,
  changeText,
  detail,
  diffLines,
  labelOf,
  probablyIgnored,
  twoClaudeLocations,
} from './model'

// « Clients LLM… » (spec 023): the state of each configuration file, then for one of them a preview of the change,
// then the change, which the application refuses if the file moved since the preview. The page sends a location,
// an action and the fingerprint of the preview; it never names a file.
const props = defineProps<{ open: boolean }>()
const emit = defineEmits<{ 'update:open': [open: boolean] }>()
const store = useStore()

type Phase = 'list' | 'preview' | 'done'
const phase = ref<Phase>('list')
const view = ref<ClientsView | null>(null)
const error = ref<string | null>(null)
const preview = ref<ClientPreview | null>(null)
const changed = ref(false)
const busy = ref(false)
const done = ref<{ location: LocationId; action: ClientAction; outcome: ApplyOutcome } | null>(null)
const copy = ref<{ text: string; ok: boolean } | null>(null)

async function load() {
  error.value = null
  try {
    view.value = await store.llmClients()
  } catch (e) {
    view.value = null
    error.value = fr.clients.loadFailed(reason(e))
  }
}

watch(
  () => props.open,
  (open) => {
    if (!open) return
    phase.value = 'list'
    preview.value = null
    done.value = null
    copy.value = null
    void load()
  },
  { immediate: true },
)

async function ask(location: LocationId, action: ClientAction) {
  error.value = null
  copy.value = null
  changed.value = false
  try {
    preview.value = await store.previewLlmClient(location, action)
    phase.value = 'preview'
  } catch (e) {
    error.value = reason(e)
  }
}

async function apply() {
  const shown = preview.value
  if (shown === null) return
  busy.value = true
  error.value = null
  try {
    const outcome = await store.applyLlmClient(shown.location, shown.action, shown.fingerprint)
    if (outcome.status === 'changed') {
      // What is written is what was shown: the new state is shown, and asked again.
      preview.value = await store.previewLlmClient(shown.location, shown.action)
      changed.value = true
    } else {
      done.value = { location: shown.location, action: shown.action, outcome }
      phase.value = 'done'
      await load()
    }
  } catch (e) {
    error.value = reason(e)
  } finally {
    busy.value = false
  }
}

function back() {
  phase.value = 'list'
  preview.value = null
  error.value = null
  copy.value = null
  void load()
}

// The clipboard of the WebView (a click is a user gesture): no capability of the application is needed. If it is
// refused, the text stays shown for a copy by hand.
async function copyText(text: string) {
  try {
    await navigator.clipboard.writeText(text)
    copy.value = { text, ok: true }
  } catch {
    copy.value = { text, ok: false }
  }
}

const lines = computed(() => (preview.value === null ? [] : diffLines(preview.value)))
const clientOf = (id: LocationId) => view.value?.locations.find((l) => l.id === id)?.client ?? 'claudeDesktop'
const title = computed(() => {
  if (phase.value === 'preview' && preview.value) {
    return fr.clients.preview.title[preview.value.action](labelOf(preview.value.location))
  }
  if (phase.value === 'done' && done.value) {
    if (done.value.outcome.status !== 'applied') return fr.clients.title
    return fr.clients.done[done.value.action](labelOf(done.value.location))
  }
  return fr.clients.title
})

function stateClass(location: ClientLocation): string {
  switch (location.state.kind) {
    case 'otherCommand':
      return 'text-attention-text font-medium'
    case 'unreadable':
      return 'text-destructive-text font-medium'
    case 'notInstalled':
      return 'text-muted-foreground'
    case 'configured':
      return 'font-medium'
    default:
      return ''
  }
}
</script>

<template>
  <AlertDialog
    :open="open"
    @update:open="(value) => emit('update:open', value)"
  >
    <AlertDialogContent
      class="max-h-[calc(100dvh-2rem)] overflow-y-auto sm:max-w-2xl"
      data-dialog="clients"
      :data-phase="phase"
    >
      <AlertDialogHeader>
        <AlertDialogTitle>{{ title }}</AlertDialogTitle>
        <AlertDialogDescription
          v-if="phase === 'list'"
          class="text-sm"
        >
          {{ fr.clients.intro }}
        </AlertDialogDescription>
      </AlertDialogHeader>

      <!-- The list of the clients. -->
      <div
        v-if="phase === 'list'"
        class="grid min-w-0 gap-3 text-sm"
      >
        <p
          v-if="view === null && error === null"
          class="text-muted-foreground"
          role="status"
        >
          {{ fr.clients.loading }}
        </p>
        <template v-if="view">
          <p
            v-if="view.command === null"
            class="rounded-md border p-3"
            data-notice="not-installed"
          >
            {{ fr.clients.notInstalled }}
          </p>
          <p
            v-else
            class="text-muted-foreground text-xs"
          >
            {{ fr.clients.command }}
            <code class="text-foreground font-mono break-all">{{ view.command }} stdio</code>
          </p>
          <p
            v-if="twoClaudeLocations(view)"
            class="rounded-md border p-3"
            data-notice="two-locations"
          >
            {{ fr.clients.twoLocations }}
          </p>
          <ul class="divide-y rounded-md border">
            <li
              v-for="location in view.locations"
              :key="location.id"
              class="grid min-w-0 gap-1.5 p-3"
              :data-location="location.id"
              :data-state="location.state.kind"
            >
              <div class="flex min-w-0 items-start gap-3">
                <div class="min-w-0 flex-1">
                  <p class="font-medium">
                    {{ fr.clients.client[location.client] }}
                    <span class="text-muted-foreground font-normal">· {{ fr.clients.variant[location.id] }}</span>
                  </p>
                  <p class="text-muted-foreground font-mono text-xs break-all">
                    {{ location.file }}
                  </p>
                </div>
                <span
                  class="shrink-0 pt-0.5 text-xs"
                  :class="stateClass(location)"
                  data-client-state
                >{{ fr.clients.state[location.state.kind] }}</span>
              </div>
              <p
                v-if="detail(location)"
                class="text-xs break-all"
                :class="location.state.kind === 'unreadable' ? 'text-destructive-text' : ''"
              >
                {{ detail(location) }}
              </p>
              <p
                v-if="probablyIgnored(view, location)"
                class="text-muted-foreground text-xs"
              >
                {{ fr.clients.probablyIgnored }}
              </p>
              <p
                v-if="location.otherServers > 0"
                class="text-muted-foreground text-xs"
              >
                {{ fr.clients.otherServers(location.otherServers) }}
              </p>
              <div
                v-if="location.state.kind !== 'notInstalled'"
                class="flex gap-2 pt-1"
              >
                <Button
                  size="sm"
                  variant="outline"
                  :disabled="!canInstall(view, location)"
                  :aria-label="fr.clients.installIn(fr.clients.label[location.id])"
                  data-action="client-install"
                  @click="ask(location.id, 'install')"
                >
                  {{ fr.clients.install }}
                </Button>
                <Button
                  size="sm"
                  variant="outline"
                  :disabled="!canRemove(location)"
                  :aria-label="fr.clients.removeFrom(fr.clients.label[location.id])"
                  data-action="client-remove"
                  @click="ask(location.id, 'remove')"
                >
                  {{ fr.clients.remove }}
                </Button>
              </div>
            </li>
          </ul>
          <div
            v-if="view.snippet"
            class="grid gap-2"
          >
            <div class="flex items-center gap-3">
              <Button
                size="sm"
                variant="outline"
                data-action="client-copy"
                @click="copyText(view.snippet)"
              >
                {{ fr.clients.copy }}
              </Button>
              <span class="text-muted-foreground text-xs">{{ fr.clients.copyHelp }}</span>
            </div>
          </div>
        </template>
      </div>

      <!-- The preview of one change. -->
      <div
        v-else-if="phase === 'preview' && preview"
        class="grid min-w-0 gap-3 text-sm"
      >
        <p
          v-if="changed"
          class="rounded-md border p-3"
          data-notice="changed"
        >
          {{ fr.clients.preview.changed }}
        </p>
        <p class="text-muted-foreground text-xs">
          {{ fr.clients.preview.file }}
          <code class="text-foreground font-mono break-all">{{ preview.file }}</code>
        </p>
        <p :class="preview.change === 'manual' ? 'text-destructive-text' : ''">
          {{ changeText(preview) }}
        </p>
        <pre
          v-if="lines.length > 0"
          class="bg-muted rounded-md p-3 font-mono text-xs leading-relaxed break-all whitespace-pre-wrap"
          data-diff
        ><template
          v-for="(line, i) in lines"
          :key="i"
        ><span
          :class="line.sign === '-' ? 'text-muted-foreground line-through' : ''"
          :data-sign="line.sign"
        >{{ line.sign }} {{ line.text }}</span>{{ '\n' }}</template></pre>
        <template v-if="applicable(preview)">
          <p class="text-muted-foreground text-xs">
            {{ fr.clients.preview.untouched(preview.otherServers) }}
          </p>
          <p
            v-if="preview.backup"
            class="text-muted-foreground text-xs"
          >
            {{ fr.clients.preview.backup }}
          </p>
        </template>
        <template v-if="preview.change === 'manual'">
          <template v-if="preview.snippet">
            <p>{{ fr.clients.preview.manualInstall }}</p>
            <pre
              class="bg-muted rounded-md p-3 font-mono text-xs break-all whitespace-pre-wrap select-all"
              data-snippet
            >{{ preview.snippet }}</pre>
            <div>
              <Button
                size="sm"
                variant="outline"
                data-action="client-copy-entry"
                @click="copyText(preview.snippet)"
              >
                {{ fr.clients.preview.copy }}
              </Button>
            </div>
          </template>
          <p v-else>
            {{ fr.clients.preview.manualRemove }}
          </p>
        </template>
      </div>

      <!-- After the change. -->
      <div
        v-else-if="phase === 'done' && done"
        class="grid gap-3 text-sm"
        data-done
      >
        <template v-if="done.outcome.status === 'applied'">
          <p
            v-if="done.outcome.backup"
            class="text-muted-foreground text-xs"
          >
            {{ fr.clients.done.backup(done.outcome.backup) }}
          </p>
          <p
            class="rounded-md border p-3 font-medium"
            data-notice="restart"
          >
            {{ fr.clients.done.restart[clientOf(done.location)] }}
          </p>
        </template>
        <p v-else>
          {{ fr.clients.done.nothing }}
        </p>
      </div>

      <p
        v-if="copy"
        class="text-xs"
        role="status"
        data-copy
      >
        {{ copy.ok ? fr.clients.copied : fr.clients.copyFailed }}
      </p>
      <!-- In the preview, the entry copied is already shown above. -->
      <pre
        v-if="copy && (phase === 'list' || !copy.ok)"
        class="bg-muted rounded-md p-3 font-mono text-xs break-all whitespace-pre-wrap select-all"
        data-copied
      >{{ copy.text }}</pre>

      <p
        v-if="error"
        class="text-destructive-text text-sm"
        role="alert"
        data-error
      >
        {{ error }}
      </p>

      <AlertDialogFooter>
        <template v-if="phase === 'preview' && preview">
          <Button
            variant="outline"
            data-action="client-back"
            @click="back"
          >
            {{ applicable(preview) ? fr.clients.preview.cancel : fr.clients.preview.back }}
          </Button>
          <Button
            v-if="applicable(preview)"
            :disabled="busy"
            data-action="client-confirm"
            @click="apply"
          >
            {{ busy ? fr.clients.preview.working : fr.clients[preview.action] }}
          </Button>
        </template>
        <template v-else-if="phase === 'done'">
          <Button
            variant="outline"
            data-action="client-back"
            @click="back"
          >
            {{ fr.clients.done.back }}
          </Button>
          <Button
            data-action="client-close"
            @click="emit('update:open', false)"
          >
            {{ fr.clients.close }}
          </Button>
        </template>
        <Button
          v-else
          variant="outline"
          data-action="client-close"
          @click="emit('update:open', false)"
        >
          {{ fr.clients.close }}
        </Button>
      </AlertDialogFooter>
    </AlertDialogContent>
  </AlertDialog>
</template>
