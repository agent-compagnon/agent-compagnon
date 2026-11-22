<script setup lang="ts">
import { ChevronRight } from '@lucide/vue'
import { computed } from 'vue'
import HighlightedText from '@/components/common/HighlightedText.vue'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { fr } from '@/i18n/fr'
import { cn } from '@/lib/utils'
import EntryDetails from './EntryDetails.vue'
import { type JournalRow, formatDuration, formatTime } from './model'

const props = defineProps<{
  row: JournalRow
  words: readonly string[]
  expanded: boolean
  now: number
  /** Columns of the list, shared with its header. */
  grid: string
}>()
const emit = defineEmits<{ toggle: [] }>()

const entry = computed(() => props.row.entry)
const isCall = computed(() => entry.value.kind === 'call')
const detailsId = computed(() => `journal-details-${String(entry.value.seq)}`)
const what = computed(() => `${entry.value.tool ?? entry.value.summary}, ${formatTime(entry.value.at, props.now)}`)
</script>

<template>
  <div
    role="listitem"
    class="border-b"
    :data-entry="entry.seq"
    :data-kind="entry.kind"
    :data-status="entry.status ?? undefined"
    :data-expandable="row.expandable"
    :data-error="row.error || undefined"
  >
    <div
      :class="cn(grid, 'min-h-9 py-1.5', row.expandable && 'hover:bg-muted/50 cursor-pointer')"
      @click="row.expandable && emit('toggle')"
    >
      <span
        class="flex items-center"
      >
        <button
          v-if="row.expandable"
          type="button"
          class="text-muted-foreground hover:text-foreground focus-visible:ring-ring/50 -m-1 flex size-6 items-center justify-center rounded-sm outline-none focus-visible:ring-3"
          :aria-expanded="expanded"
          :aria-controls="expanded ? detailsId : undefined"
          :aria-label="expanded ? fr.journal.row.hide(what) : fr.journal.row.show(what)"
          data-action="toggle"
          @click.stop="emit('toggle')"
        >
          <ChevronRight
            :class="cn('size-4 transition-transform', expanded && 'rotate-90')"
            aria-hidden="true"
          />
        </button>
      </span>
      <span
        class="text-muted-foreground font-mono text-xs tabular-nums"
        data-cell="time"
      >{{ formatTime(entry.at, now) }}</span>
      <template v-if="isCall">
        <span
          class="truncate"
          :title="entry.client ?? undefined"
          data-cell="client"
        ><HighlightedText
          :text="entry.client ?? ''"
          :words="words"
        /></span>
        <span
          class="truncate font-medium"
          data-cell="environment"
        ><HighlightedText
          :text="row.environment ?? ''"
          :words="words"
        /></span>
        <span
          class="truncate"
          :title="row.mcp ?? undefined"
          data-cell="mcp"
        ><HighlightedText
          :text="row.mcp ?? ''"
          :words="words"
        /></span>
        <span
          class="truncate font-mono text-xs"
          :title="entry.tool ?? undefined"
          data-cell="tool"
        ><HighlightedText
          :text="entry.tool ?? ''"
          :words="words"
        /></span>
      </template>
      <span
        v-else
        class="col-span-4 truncate"
        :title="entry.summary"
        data-cell="summary"
      ><HighlightedText
        :text="entry.summary"
        :words="words"
      /></span>
      <span
        :class="
          cn(
            'flex min-w-0 items-center gap-1.5',
            row.error ? 'text-destructive-text font-medium' : !isCall && 'text-muted-foreground',
          )
        "
        data-cell="status"
      >
        <span class="truncate"><HighlightedText
          :text="row.status"
          :words="words"
        /></span>
        <Tooltip v-if="entry.slow">
          <TooltipTrigger as-child>
            <span
              class="border-foreground/30 text-foreground shrink-0 rounded-sm border px-1 text-[0.625rem] leading-4 font-medium"
              tabindex="0"
              data-mark="slow"
            >{{ fr.journal.slow }}</span>
          </TooltipTrigger>
          <TooltipContent>{{ fr.journal.slowHelp }}</TooltipContent>
        </Tooltip>
      </span>
      <span
        class="text-muted-foreground text-right text-xs tabular-nums"
        data-cell="duration"
      >{{ entry.durationMs === null ? '' : formatDuration(entry.durationMs) }}</span>
    </div>
    <EntryDetails
      v-if="expanded && entry.details"
      :id="detailsId"
      :details="entry.details"
    />
  </div>
</template>
