<script setup lang="ts">
import { TriangleAlert } from '@lucide/vue'
import { computed } from 'vue'
import type { AppSnapshot } from '@/api/types'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { fr } from '@/i18n/fr'
import { cn } from '@/lib/utils'
import { overThreshold } from './model'

// Orange strictly above the threshold: too many tools is something the user has to act on.
const props = defineProps<{ snapshot: AppSnapshot }>()
const over = computed(() => overThreshold(props.snapshot))
const help = computed(() =>
  over.value
    ? fr.mcp.counter.over(props.snapshot.exposedTools, props.snapshot.exposedToolsThreshold)
    : fr.mcp.counter.help(props.snapshot.exposedToolsThreshold),
)
</script>

<template>
  <Tooltip>
    <TooltipTrigger as-child>
      <button
        type="button"
        :class="
          cn(
            'focus-visible:ring-ring/50 inline-flex h-7 shrink-0 items-center gap-1.5 rounded-full border px-3 text-xs font-medium tabular-nums outline-none focus-visible:ring-3',
            over ? 'border-attention/50 bg-attention-muted text-attention-text' : 'text-muted-foreground',
          )
        "
        :aria-describedby="over ? 'tool-counter-help' : undefined"
        :data-over="over"
        data-counter
      >
        <TriangleAlert
          v-if="over"
          class="size-3.5"
          aria-hidden="true"
        />
        {{ fr.mcp.counter.text(snapshot.exposedTools) }}
      </button>
    </TooltipTrigger>
    <TooltipContent
      side="bottom"
      class="max-w-xs"
    >
      {{ help }}
    </TooltipContent>
  </Tooltip>
  <span
    v-if="over"
    id="tool-counter-help"
    class="sr-only"
  >{{ help }}</span>
</template>
