<script setup lang="ts">
import { RefreshCw } from '@lucide/vue'
import { computed } from 'vue'
import type { ToolView } from '@/api/types'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { fr } from '@/i18n/fr'
import DecisionIcon from './DecisionIcon.vue'
import HighlightedText from '@/components/common/HighlightedText.vue'
import type { McpRow } from './model'

// The tools of an unfolded row: all of them, or those the search found (their matching words highlighted). New and
// modified tools come first, with their mark and their policy.
const props = defineProps<{
  row: McpRow
  tools: readonly ToolView[]
  words: readonly string[]
  /** The list is the result of the search, not every tool. */
  searched: boolean
  canReload: boolean
}>()
const emit = defineEmits<{ reload: [] }>()

const rank = (tool: ToolView) => (props.row.added.has(tool.name) ? 0 : props.row.modified.has(tool.name) ? 1 : 2)
const ordered = computed(() => [...props.tools].sort((a, b) => rank(a) - rank(b)))
const removed = computed(() => props.row.instance.update?.removed ?? [])
</script>

<template>
  <div
    class="grid gap-2 px-3 py-3"
    :data-tools="row.id"
  >
    <div
      v-if="searched || row.instance.lastError || canReload || removed.length > 0"
      class="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs"
    >
      <p
        v-if="searched"
        class="text-muted-foreground"
        data-field="matching"
      >
        {{ fr.mcp.tools.matching(tools.length, row.instance.tools.length) }}
      </p>
      <p
        v-if="row.instance.lastError"
        class="text-destructive-text"
        data-field="last-error"
      >
        {{ fr.mcp.row.lastError }} : {{ row.instance.lastError }}
      </p>
      <p
        v-if="removed.length > 0"
        class="text-muted-foreground"
        data-field="removed"
      >
        {{ fr.mcp.tools.removed(removed) }}
      </p>
      <Button
        v-if="canReload"
        size="xs"
        variant="outline"
        class="ml-auto"
        data-action="reload"
        @click="emit('reload')"
      >
        <RefreshCw />
        {{ fr.mcp.row.reload }}
      </Button>
    </div>
    <p
      v-if="ordered.length === 0"
      class="text-muted-foreground text-xs"
    >
      {{ fr.mcp.tools.none }}
    </p>
    <table
      v-else
      class="w-full table-fixed text-xs"
    >
      <colgroup>
        <col class="w-[32%]">
        <col>
        <col class="w-20">
        <col class="w-48">
      </colgroup>
      <thead class="text-muted-foreground">
        <tr class="border-b">
          <th class="py-1.5 pr-3 text-left font-medium">
            {{ fr.mcp.tools.name }}
          </th>
          <th class="py-1.5 pr-3 text-left font-medium">
            {{ fr.mcp.tools.description }}
          </th>
          <th class="py-1.5 pr-3 text-left font-medium">
            {{ fr.mcp.tools.category }}
          </th>
          <th class="py-1.5 text-left font-medium">
            {{ fr.mcp.tools.policy }}
          </th>
        </tr>
      </thead>
      <tbody>
        <tr
          v-for="tool in ordered"
          :key="tool.name"
          class="border-b align-top last:border-0"
          :data-tool="tool.name"
        >
          <td class="py-1.5 pr-3">
            <div class="flex flex-wrap items-center gap-1">
              <span class="font-mono font-medium break-all">
                <HighlightedText
                  :text="tool.name"
                  :words="words"
                />
              </span>
              <Badge
                v-if="row.added.has(tool.name)"
                class="h-4 px-1.5 text-[10px]"
                data-mark="added"
              >
                {{ fr.mcp.tools.marks.added }}
              </Badge>
              <Badge
                v-if="row.modified.has(tool.name)"
                variant="outline"
                class="h-4 px-1.5 text-[10px]"
                data-mark="modified"
              >
                {{ fr.mcp.tools.marks.modified }}
              </Badge>
              <Badge
                v-if="!tool.known"
                variant="outline"
                class="h-4 border-dashed px-1.5 text-[10px]"
                :title="fr.mcp.tools.unknownHelp"
                data-mark="unknown"
              >
                {{ fr.mcp.tools.marks.unknown }}
              </Badge>
            </div>
            <span class="text-muted-foreground block font-mono text-[11px] break-all">
              <HighlightedText
                :text="tool.exposedName"
                :words="words"
              />
            </span>
          </td>
          <td class="py-1.5 pr-3 leading-relaxed break-words">
            <HighlightedText
              v-if="tool.description"
              :text="tool.description"
              :words="words"
            />
            <span
              v-else
              class="text-muted-foreground italic"
            >{{ fr.mcp.tools.noDescription }}</span>
          </td>
          <td class="py-1.5 pr-3">
            {{ fr.mcp.category[tool.category] }}
          </td>
          <td class="py-1.5">
            <Tooltip>
              <TooltipTrigger as-child>
                <button
                  type="button"
                  class="focus-visible:ring-ring/50 inline-flex items-center gap-1 rounded-sm text-left outline-none focus-visible:ring-3 [&_svg]:size-3.5 [&_svg]:shrink-0"
                  :data-decision="tool.policy.decision"
                  data-field="tool-policy"
                >
                  <DecisionIcon :decision="tool.policy.decision" />
                  {{ fr.mcp.decisionDetail[tool.policy.decision] }}
                </button>
              </TooltipTrigger>
              <TooltipContent
                side="left"
                class="max-w-xs"
              >
                {{ tool.policy.explanation }}
              </TooltipContent>
            </Tooltip>
          </td>
        </tr>
      </tbody>
    </table>
  </div>
</template>
