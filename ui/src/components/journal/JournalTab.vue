<script setup lang="ts">
import { ChevronDown, CircleAlert, CircleCheck, Download, Info, Pause, Radio, Search, X } from '@lucide/vue'
import { computed, ref } from 'vue'
import type { AppSnapshot } from '@/api/types'
import AppBanner from '@/components/banners/AppBanner.vue'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Input } from '@/components/ui/input'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { fr } from '@/i18n/fr'
import { useNow } from '@/lib/now'
import { terms } from '@/lib/search'
import { cn } from '@/lib/utils'
import { useStore } from '@/store/snapshot'
import JournalList from './JournalList.vue'
import { JOURNAL_LIMIT, type StatusFilter, environmentChoices, labelsOf, matches, rowOf } from './model'

const props = defineProps<{ snapshot: AppSnapshot }>()
const store = useStore()
const journal = store.journal
const now = useNow()

const ALL = 'all'
const query = ref('')
const environment = ref<string>(ALL)
const status = ref<StatusFilter>('all')
const expanded = ref<ReadonlySet<number>>(new Set())

const words = computed(() => terms(query.value))
const labels = computed(() => labelsOf(props.snapshot))
const shown = computed(() => journal.state.shown)
const rows = computed(() => shown.value.map((e) => rowOf(e, labels.value)))
const visible = computed(() => {
  const criteria = {
    environment: environment.value === ALL ? null : environment.value,
    status: status.value,
    words: words.value,
  }
  // Newest first.
  return rows.value.filter((r) => matches(r, criteria)).reverse()
})
const choices = computed(() => environmentChoices(shown.value, props.snapshot))
const chosen = computed(() => choices.value.find((c) => (c.id ?? ALL) === environment.value) ?? choices.value[0])
const filtered = computed(() => words.value.length > 0 || environment.value !== ALL || status.value !== 'all')
const held = computed(() => journal.state.shown.length + journal.state.pending.length)
// Information alerts other than the catalogue cache (shown by the MCP tab): start-up hardening, for the diagnostic.
const notices = computed(() => props.snapshot.alerts.filter((a) => a.level === 'info' && a.kind !== 'catalogCache'))

const STATUSES: { id: StatusFilter; label: string; help: string | null }[] = [
  { id: 'all', label: fr.journal.status.all, help: null },
  { id: 'problems', label: fr.journal.status.problems, help: fr.journal.status.problemsHelp },
  { id: 'policies', label: fr.journal.status.policies, help: fr.journal.status.policiesHelp },
]

function toggle(seq: number) {
  const next = new Set(expanded.value)
  if (next.has(seq)) next.delete(seq)
  else next.add(seq)
  expanded.value = next
}

function clearFilters() {
  query.value = ''
  environment.value = ALL
  status.value = 'all'
}

function switchLive() {
  if (journal.state.paused) journal.resume()
  else journal.pause()
}
</script>

<template>
  <div class="flex flex-1 flex-col gap-3">
    <div
      v-if="notices.length > 0 || journal.state.export.phase === 'saved' || journal.state.export.phase === 'failed'"
      class="flex flex-col gap-2"
    >
      <AppBanner
        v-for="notice in notices"
        :key="notice.kind + notice.message"
        tone="neutral"
        :data-banner="notice.kind"
      >
        <template #icon>
          <Info class="size-4" />
        </template>
        {{ notice.message }}
      </AppBanner>
      <AppBanner
        v-if="journal.state.export.phase === 'saved'"
        tone="neutral"
        data-banner="export-saved"
      >
        <template #icon>
          <CircleCheck class="size-4" />
        </template>
        {{ fr.journal.export.saved }}
        <span
          class="font-mono text-xs break-all"
          data-field="archive-path"
        >{{ journal.state.export.path }}</span>
        <template #action>
          <Button
            size="sm"
            variant="ghost"
            @click="journal.dismissExport()"
          >
            {{ fr.journal.export.dismiss }}
          </Button>
        </template>
      </AppBanner>
      <AppBanner
        v-if="journal.state.export.phase === 'failed'"
        tone="error"
        data-banner="export-failed"
      >
        <template #icon>
          <CircleAlert class="size-4" />
        </template>
        {{ fr.journal.export.failed(journal.state.export.reason) }}
        <template #action>
          <Button
            size="sm"
            variant="ghost"
            @click="journal.dismissExport()"
          >
            {{ fr.journal.export.dismiss }}
          </Button>
        </template>
      </AppBanner>
    </div>

    <div class="flex flex-wrap items-center gap-2">
      <div class="relative min-w-56 flex-1">
        <Search
          class="text-muted-foreground pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2"
          aria-hidden="true"
        />
        <Input
          v-model="query"
          type="text"
          role="searchbox"
          :aria-label="fr.journal.search.label"
          :placeholder="fr.journal.search.placeholder"
          class="h-8 pr-8 pl-8"
          data-field="journal-search"
          @keydown.escape="query = ''"
        />
        <button
          v-if="query"
          type="button"
          class="text-muted-foreground hover:text-foreground focus-visible:ring-ring/50 absolute top-1/2 right-1.5 flex size-5 -translate-y-1/2 items-center justify-center rounded-sm outline-none focus-visible:ring-3"
          :aria-label="fr.journal.search.clear"
          @click="query = ''"
        >
          <X
            class="size-3.5"
            aria-hidden="true"
          />
        </button>
      </div>
      <DropdownMenu>
        <DropdownMenuTrigger as-child>
          <Button
            size="sm"
            variant="outline"
            class="w-48 justify-between font-normal"
            :aria-label="`${fr.journal.filter.label} : ${chosen ? fr.journal.filter.choice(chosen.label, chosen.count) : ''}`"
            data-field="journal-environment"
          >
            <span class="truncate">{{ chosen ? fr.journal.filter.choice(chosen.label, chosen.count) : '' }}</span>
            <ChevronDown class="opacity-50" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent
          align="end"
          class="w-48"
        >
          <DropdownMenuLabel class="text-muted-foreground text-xs font-normal">
            {{ fr.journal.filter.label }}
          </DropdownMenuLabel>
          <DropdownMenuSeparator />
          <DropdownMenuRadioGroup v-model="environment">
            <DropdownMenuRadioItem
              v-for="choice in choices"
              :key="choice.id ?? ALL"
              :value="choice.id ?? ALL"
              :data-choice="choice.id ?? ALL"
            >
              <span class="flex-1">{{ choice.label }}</span>
              <span class="text-muted-foreground text-xs tabular-nums">{{ choice.count }}</span>
            </DropdownMenuRadioItem>
          </DropdownMenuRadioGroup>
        </DropdownMenuContent>
      </DropdownMenu>
      <div
        role="radiogroup"
        :aria-label="fr.journal.status.label"
        class="bg-muted inline-flex h-8 items-center rounded-md p-0.5"
        data-field="journal-status"
      >
        <template
          v-for="choice in STATUSES"
          :key="choice.id"
        >
          <Tooltip v-if="choice.help">
            <TooltipTrigger as-child>
              <button
                type="button"
                role="radio"
                :aria-checked="status === choice.id"
                :class="
                  cn(
                    'focus-visible:ring-ring/50 h-7 rounded-[5px] px-3 text-sm outline-none focus-visible:ring-3',
                    status === choice.id
                      ? 'bg-background text-foreground font-medium shadow-sm'
                      : 'text-muted-foreground hover:text-foreground',
                  )
                "
                :data-status-filter="choice.id"
                @click="status = choice.id"
              >
                {{ choice.label }}
              </button>
            </TooltipTrigger>
            <TooltipContent class="max-w-64">
              {{ choice.help }}
            </TooltipContent>
          </Tooltip>
          <button
            v-else
            type="button"
            role="radio"
            :aria-checked="status === choice.id"
            :class="
              cn(
                'focus-visible:ring-ring/50 h-7 rounded-[5px] px-3 text-sm outline-none focus-visible:ring-3',
                status === choice.id
                  ? 'bg-background text-foreground font-medium shadow-sm'
                  : 'text-muted-foreground hover:text-foreground',
              )
            "
            :data-status-filter="choice.id"
            @click="status = choice.id"
          >
            {{ choice.label }}
          </button>
        </template>
      </div>
    </div>

    <div class="flex min-h-8 flex-wrap items-center gap-x-3 gap-y-2 text-sm">
      <Button
        size="sm"
        :variant="journal.state.paused ? 'secondary' : 'outline'"
        class="w-28"
        :aria-label="
          journal.state.paused
            ? `${fr.journal.live.paused} — ${fr.journal.live.resume}`
            : `${fr.journal.live.live} — ${fr.journal.live.pause}`
        "
        :data-live="!journal.state.paused"
        data-action="live"
        @click="switchLive"
      >
        <Pause
          v-if="journal.state.paused"
          aria-hidden="true"
        />
        <Radio
          v-else
          aria-hidden="true"
        />
        {{ journal.state.paused ? fr.journal.live.paused : fr.journal.live.live }}
      </Button>
      <span
        class="text-muted-foreground tabular-nums"
        data-field="journal-count"
      >{{ fr.journal.count(visible.length, shown.length) }}</span>
      <span
        v-if="journal.state.paused"
        class="flex items-center gap-2"
        role="status"
        data-field="pending"
        :data-pending="journal.state.pending.length"
      >
        <span>{{ fr.journal.pending(journal.state.pending.length) }}</span>
        <Button
          v-if="journal.state.pending.length > 0"
          size="sm"
          variant="outline"
          class="h-7"
          data-action="show-pending"
          @click="journal.resume()"
        >
          {{ fr.journal.showPending }}
        </Button>
      </span>
      <span
        v-if="held >= JOURNAL_LIMIT"
        class="text-muted-foreground text-xs"
        data-field="limit"
      >{{ fr.journal.limit(JOURNAL_LIMIT) }}</span>
      <Button
        size="sm"
        variant="outline"
        class="ml-auto"
        :disabled="journal.state.export.phase === 'running'"
        data-action="export"
        @click="journal.exportDiagnostic()"
      >
        <Download aria-hidden="true" />
        {{ journal.state.export.phase === 'running' ? fr.journal.export.running : fr.journal.export.button }}
      </Button>
    </div>

    <p
      v-if="journal.state.phase !== 'ready'"
      class="text-muted-foreground rounded-lg border px-4 py-12 text-center text-sm"
      :role="journal.state.phase === 'loading' ? 'status' : 'alert'"
      :data-journal-phase="journal.state.phase"
    >
      {{ fr.journal[journal.state.phase] }}
    </p>
    <JournalList
      v-else
      class="flex-1"
      :rows="visible"
      :words="words"
      :expanded="expanded"
      :now="now"
      @toggle="toggle"
    >
      <template #empty>
        <div
          class="flex flex-col items-center px-4 py-12 text-center"
          :data-empty="filtered ? 'filtered' : 'journal'"
        >
          <p class="text-muted-foreground max-w-md text-sm">
            <template v-if="!filtered">
              {{ fr.journal.empty.none }}
            </template>
            <template v-else-if="words.length > 0">
              {{ fr.journal.empty.noResult(query.trim()) }}
            </template>
            <template v-else>
              {{ fr.journal.empty.noMatch }}
            </template>
          </p>
          <Button
            v-if="filtered"
            size="sm"
            variant="outline"
            class="mt-3"
            data-action="clear-filters"
            @click="clearFilters"
          >
            {{ fr.journal.empty.clear }}
          </Button>
        </div>
      </template>
    </JournalList>
  </div>
</template>
