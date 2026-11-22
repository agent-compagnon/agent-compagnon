<script setup lang="ts">
import { useVirtualizer } from '@tanstack/vue-virtual'
import { computed, ref } from 'vue'
import { fr } from '@/i18n/fr'
import { cn } from '@/lib/utils'
import JournalRow from './JournalRow.vue'
import type { JournalRow as Row } from './model'

// The rows, newest first, virtualized: only those in view are in the DOM, however many entries are held. An
// unfolded row is taller: each rendered row is measured, and measured again when its height changes (the
// virtualizer observes it), so that scrolling stays right.
const props = defineProps<{
  rows: readonly Row[]
  words: readonly string[]
  expanded: ReadonlySet<number>
  now: number
}>()
const emit = defineEmits<{ toggle: [seq: number] }>()

const GRID =
  'grid grid-cols-[1.25rem_4.25rem_7.5rem_3.25rem_7.25rem_minmax(0,1fr)_9rem_3.75rem] items-center gap-x-2 px-3'

/** Height of a folded row, before it is measured. */
const ROW_HEIGHT = 37

const scroller = ref<HTMLElement | null>(null)
const virtualizer = useVirtualizer(
  computed(() => ({
    count: props.rows.length,
    getScrollElement: () => scroller.value,
    estimateSize: () => ROW_HEIGHT,
    overscan: 10,
    getItemKey: (index: number) => props.rows[index]?.entry.seq ?? index,
  })),
)
const items = computed(() =>
  virtualizer.value.getVirtualItems().flatMap((item) => {
    const row = props.rows[item.index]
    return row ? [{ item, row }] : []
  }),
)
const height = computed(() => `${String(virtualizer.value.getTotalSize())}px`)

function measure(element: unknown) {
  if (element instanceof Element) virtualizer.value.measureElement(element)
}
</script>

<template>
  <div class="flex min-h-0 flex-col rounded-lg border">
    <div
      :class="cn(GRID, 'text-muted-foreground bg-muted/40 h-8 border-b text-xs font-medium')"
      aria-hidden="true"
      data-header
    >
      <span />
      <span>{{ fr.journal.columns.time }}</span>
      <span>{{ fr.journal.columns.client }}</span>
      <span>{{ fr.journal.columns.environment }}</span>
      <span>{{ fr.journal.columns.mcp }}</span>
      <span>{{ fr.journal.columns.tool }}</span>
      <span>{{ fr.journal.columns.status }}</span>
      <span class="text-right">{{ fr.journal.columns.duration }}</span>
    </div>
    <div
      ref="scroller"
      class="min-h-64 flex-[1_1_0] overflow-y-auto text-[0.8125rem] [overflow-anchor:none]"
      data-scroller
    >
      <slot
        v-if="rows.length === 0"
        name="empty"
      />
      <div
        v-else
        role="list"
        :aria-label="fr.tabs.journal"
        class="relative w-full"
        :style="{ height }"
      >
        <div
          v-for="{ item, row } in items"
          :key="String(item.key)"
          :ref="measure"
          :data-index="item.index"
          class="absolute top-0 left-0 w-full"
          :style="{ transform: `translateY(${String(item.start)}px)` }"
        >
          <JournalRow
            :row="row"
            :words="words"
            :expanded="expanded.has(row.entry.seq)"
            :now="now"
            :grid="GRID"
            @toggle="emit('toggle', row.entry.seq)"
          />
        </div>
      </div>
    </div>
  </div>
</template>
