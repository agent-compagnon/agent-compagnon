<script setup lang="ts">
import { ArrowDown, ArrowUp, ArrowUpDown, ChevronRight } from '@lucide/vue'
import {
  type ColumnDef,
  type SortingState,
  getCoreRowModel,
  getSortedRowModel,
  useVueTable,
} from '@tanstack/vue-table'
import { computed, ref, watch } from 'vue'
import type { AppSnapshot } from '@/api/types'
import StateDot from '@/components/connections/StateDot.vue'
import { Switch } from '@/components/ui/switch'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { fr } from '@/i18n/fr'
import { cn } from '@/lib/utils'
import PolicyCell from './PolicyCell.vue'
import ToolList from './ToolList.vue'
import { type Visible, policyRank, rowName, stateRank, switchBlocked } from './model'

const props = defineProps<{
  snapshot: AppSnapshot
  visible: Visible[]
  words: readonly string[]
  /** A row to unfold, for « Voir » of an update banner. */
  focus: string | null
}>()
const emit = defineEmits<{
  setMcpExposed: [mcp: string, exposed: boolean]
  reload: [environment: string, mcp: string]
}>()

type ColumnId = 'mcp' | 'environment' | 'session' | 'tools' | 'policy' | 'state' | 'exposed'

const columns: ColumnDef<Visible>[] = [
  { id: 'mcp', accessorFn: (v) => v.row.mcp.label },
  { id: 'environment', accessorFn: (v) => v.row.environmentIndex },
  { id: 'session', accessorFn: (v) => v.row.session?.label ?? '' },
  { id: 'tools', accessorFn: (v) => v.row.instance.tools.length },
  { id: 'policy', accessorFn: (v) => policyRank(v.row) },
  { id: 'state', accessorFn: (v) => stateRank(v.row) },
  { id: 'exposed', accessorFn: (v) => (v.row.instance.exposed ? 0 : 1) },
]

const sorting = ref<SortingState>([])
const table = useVueTable({
  get data() {
    return props.visible
  },
  columns,
  state: {
    get sorting() {
      return sorting.value
    },
  },
  onSortingChange: (updater) => {
    sorting.value = typeof updater === 'function' ? updater(sorting.value) : updater
  },
  getRowId: (v) => v.row.id,
  getCoreRowModel: getCoreRowModel(),
  getSortedRowModel: getSortedRowModel(),
})

const HEADINGS: Record<ColumnId, string> = {
  mcp: fr.mcp.columns.mcp,
  environment: fr.mcp.columns.environment,
  session: fr.mcp.columns.session,
  tools: fr.mcp.columns.tools,
  policy: fr.mcp.columns.policy,
  state: fr.mcp.columns.state,
  exposed: fr.mcp.columns.exposed,
}
const WIDTHS: Record<ColumnId, string> = {
  mcp: '',
  environment: 'w-[4.5rem]',
  session: 'w-36',
  tools: 'w-[4.5rem] text-right',
  policy: 'w-[7.5rem]',
  state: 'w-32',
  exposed: 'w-[4.5rem]',
}
const heading = (id: string) => HEADINGS[id as ColumnId]
const width = (id: string) => WIDTHS[id as ColumnId]

// Unfolded rows: those the user opened, and those the search unfolded (its tools matched) unless the user closed them.
const opened = ref(new Set<string>())
const closed = ref(new Set<string>())
watch(
  () => props.words.join(' '),
  () => {
    closed.value = new Set()
  },
)
watch(
  () => props.focus,
  (focus) => {
    if (focus !== null) opened.value = new Set([...opened.value, focus])
  },
  { immediate: true },
)

function isOpen(v: Visible): boolean {
  if (opened.value.has(v.row.id)) return true
  return v.match.tools !== null && !closed.value.has(v.row.id)
}

function toggle(v: Visible) {
  const id = v.row.id
  const open = isOpen(v)
  const nextOpened = new Set(opened.value)
  const nextClosed = new Set(closed.value)
  if (open) {
    nextOpened.delete(id)
    nextClosed.add(id)
  } else {
    nextClosed.delete(id)
    nextOpened.add(id)
  }
  opened.value = nextOpened
  closed.value = nextClosed
}

const columnCount = columns.length
const empty = computed(() => props.visible.length === 0)

function ariaSort(sorted: false | 'asc' | 'desc') {
  if (sorted === 'asc') return 'ascending'
  if (sorted === 'desc') return 'descending'
  return undefined
}
</script>

<template>
  <Table class="table-fixed">
    <TableHeader>
      <TableRow
        v-for="group in table.getHeaderGroups()"
        :key="group.id"
        class="hover:bg-transparent"
      >
        <TableHead
          v-for="header in group.headers"
          :key="header.id"
          :class="cn('text-muted-foreground h-9 px-2 text-xs', width(header.column.id))"
          :aria-sort="ariaSort(header.column.getIsSorted())"
        >
          <button
            type="button"
            :class="
              cn(
                'hover:text-foreground focus-visible:ring-ring/50 inline-flex items-center gap-1 rounded-sm outline-none focus-visible:ring-3',
                header.column.getIsSorted() && 'text-foreground',
                header.column.id === 'tools' && 'flex-row-reverse',
              )
            "
            :aria-label="fr.mcp.columns.sort(heading(header.column.id))"
            :data-sort="header.column.id"
            @click="header.column.toggleSorting(header.column.getIsSorted() === 'asc')"
          >
            {{ heading(header.column.id) }}
            <ArrowUp
              v-if="header.column.getIsSorted() === 'asc'"
              class="size-3"
              aria-hidden="true"
            />
            <ArrowDown
              v-else-if="header.column.getIsSorted() === 'desc'"
              class="size-3"
              aria-hidden="true"
            />
            <ArrowUpDown
              v-else
              class="size-3 opacity-40"
              aria-hidden="true"
            />
          </button>
        </TableHead>
      </TableRow>
    </TableHeader>
    <TableBody>
      <template
        v-for="tableRow in table.getRowModel().rows"
        :key="tableRow.id"
      >
        <TableRow
          :data-row="tableRow.original.row.id"
          :data-open="isOpen(tableRow.original)"
          :class="cn(isOpen(tableRow.original) && 'border-b-0 bg-muted/40 hover:bg-muted/40')"
        >
          <TableCell class="px-2 py-1.5">
            <button
              type="button"
              class="focus-visible:ring-ring/50 group flex w-full min-w-0 items-center gap-1.5 rounded-sm text-left outline-none focus-visible:ring-3"
              :aria-expanded="isOpen(tableRow.original)"
              :aria-controls="`tools-${tableRow.original.row.id}`"
              :aria-label="
                isOpen(tableRow.original)
                  ? fr.mcp.row.hide(rowName(tableRow.original.row))
                  : fr.mcp.row.show(rowName(tableRow.original.row))
              "
              data-action="expand"
              @click="toggle(tableRow.original)"
            >
              <ChevronRight
                :class="
                  cn(
                    'text-muted-foreground size-3.5 shrink-0 transition-transform',
                    isOpen(tableRow.original) && 'rotate-90',
                  )
                "
                aria-hidden="true"
              />
              <span class="flex min-w-0 items-baseline gap-1.5">
                <span
                  class="truncate font-medium"
                  :title="tableRow.original.row.mcp.label"
                >{{ tableRow.original.row.mcp.label }}</span>
                <span class="text-muted-foreground shrink-0 text-xs">
                  {{ tableRow.original.row.mcp.local ? fr.mcp.local : fr.mcp.remote }}
                </span>
              </span>
            </button>
          </TableCell>
          <TableCell
            :class="
              cn(
                'px-2 py-1.5 text-xs font-medium',
                !tableRow.original.row.environment.exposed && 'text-muted-foreground',
              )
            "
            data-field="environment"
          >
            {{ tableRow.original.row.environment.label }}
          </TableCell>
          <TableCell
            class="px-2 py-1.5"
            data-field="session"
          >
            <span
              v-if="tableRow.original.row.session"
              class="flex min-w-0 items-center gap-2 text-xs"
            >
              <StateDot :kind="tableRow.original.row.session.state.kind" />
              <span class="truncate">{{ tableRow.original.row.session.label }}</span>
            </span>
            <span
              v-else
              class="text-muted-foreground text-xs"
            >{{ fr.mcp.noSession }}</span>
          </TableCell>
          <TableCell
            class="px-2 py-1.5 text-right text-xs tabular-nums"
            data-field="tools"
          >
            {{ tableRow.original.row.instance.tools.length }}
          </TableCell>
          <TableCell class="px-2 py-1.5">
            <PolicyCell :policy="tableRow.original.row.instance.policy" />
          </TableCell>
          <TableCell
            class="px-2 py-1.5"
            data-field="state"
          >
            <Tooltip>
              <TooltipTrigger as-child>
                <button
                  type="button"
                  :class="
                    cn(
                      'focus-visible:ring-ring/50 rounded-sm text-xs outline-none focus-visible:ring-3',
                      tableRow.original.row.instance.state === 'unreachable'
                        ? 'text-destructive-text font-medium'
                        : tableRow.original.row.instance.state === 'connected'
                          ? 'text-foreground'
                          : 'text-muted-foreground',
                    )
                  "
                  :data-state="tableRow.original.row.instance.state"
                >
                  {{ fr.mcp.state[tableRow.original.row.instance.state] }}
                </button>
              </TooltipTrigger>
              <TooltipContent
                side="bottom"
                class="max-w-xs"
              >
                <p>{{ fr.mcp.stateHelp[tableRow.original.row.instance.state] }}</p>
                <p
                  v-if="tableRow.original.row.instance.lastError"
                  class="mt-1 opacity-75"
                >
                  {{ fr.mcp.row.lastError }} : {{ tableRow.original.row.instance.lastError }}
                </p>
              </TooltipContent>
            </Tooltip>
          </TableCell>
          <TableCell
            class="px-2 py-1.5"
            data-field="exposed"
          >
            <Tooltip>
              <TooltipTrigger as-child>
                <span
                  class="focus-visible:ring-ring/50 inline-flex rounded-full outline-none focus-visible:ring-3"
                  :tabindex="switchBlocked(snapshot, tableRow.original.row) ? 0 : undefined"
                >
                  <Switch
                    class="data-[state=unchecked]:bg-foreground/15 dark:data-[state=unchecked]:bg-foreground/20 disabled:opacity-40"
                    :model-value="tableRow.original.row.instance.exposed"
                    :disabled="switchBlocked(snapshot, tableRow.original.row) !== null"
                    :aria-label="
                      fr.mcp.row.expose(tableRow.original.row.mcp.label, tableRow.original.row.environment.label)
                    "
                    :aria-describedby="`why-${tableRow.original.row.id}`"
                    data-action="expose"
                    @update:model-value="(value: boolean) => emit('setMcpExposed', tableRow.original.row.mcp.id, value)"
                  />
                </span>
              </TooltipTrigger>
              <TooltipContent
                side="left"
                class="max-w-xs"
              >
                {{
                  switchBlocked(snapshot, tableRow.original.row) ??
                    (tableRow.original.row.mcp.disabled
                      ? fr.mcp.row.restoreEverywhere(tableRow.original.row.mcp.label)
                      : fr.mcp.row.cutEverywhere(tableRow.original.row.mcp.label))
                }}
              </TooltipContent>
            </Tooltip>
            <span
              :id="`why-${tableRow.original.row.id}`"
              class="sr-only"
              data-field="why"
            >{{
              switchBlocked(snapshot, tableRow.original.row) ??
                (tableRow.original.row.mcp.disabled
                  ? fr.mcp.row.restoreEverywhere(tableRow.original.row.mcp.label)
                  : fr.mcp.row.cutEverywhere(tableRow.original.row.mcp.label))
            }}</span>
          </TableCell>
        </TableRow>
        <TableRow
          v-if="isOpen(tableRow.original)"
          :id="`tools-${tableRow.original.row.id}`"
          class="bg-muted/40 hover:bg-muted/40"
        >
          <TableCell
            :colspan="columnCount"
            class="p-0 whitespace-normal"
          >
            <ToolList
              :row="tableRow.original.row"
              :tools="tableRow.original.match.tools ?? tableRow.original.row.instance.tools"
              :words="tableRow.original.match.tools ? words : []"
              :searched="tableRow.original.match.tools !== null"
              :can-reload="tableRow.original.row.instance.exposed"
              @reload="emit('reload', tableRow.original.row.environment.id, tableRow.original.row.mcp.id)"
            />
          </TableCell>
        </TableRow>
      </template>
      <TableRow
        v-if="empty"
        class="hover:bg-transparent"
      >
        <TableCell
          :colspan="columnCount"
          class="py-10 text-center whitespace-normal"
          data-empty="rows"
        >
          <slot name="empty" />
        </TableCell>
      </TableRow>
    </TableBody>
  </Table>
</template>
