<script setup lang="ts">
import { computed } from 'vue'
import type { PolicySummaryView } from '@/api/types'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { fr } from '@/i18n/fr'
import DecisionIcon from './DecisionIcon.vue'
import { CATEGORIES } from './model'

// The default policy of an instance: one mark per category, the full explanation in the tooltip.
const props = defineProps<{ policy: PolicySummaryView }>()
const lines = computed(() =>
  CATEGORIES.map((c) => ({
    category: c,
    text: fr.mcp.policy.line(fr.mcp.category[c], fr.mcp.decisionDetail[props.policy[c].decision]),
    explanation: props.policy[c].explanation,
  })),
)
</script>

<template>
  <Tooltip>
    <TooltipTrigger as-child>
      <button
        type="button"
        class="focus-visible:ring-ring/50 inline-flex items-center gap-1 rounded-md outline-none focus-visible:ring-3"
        :aria-label="`${fr.mcp.columns.policy} : ${lines.map((l) => l.text).join(', ')}`"
        data-field="policy"
      >
        <span
          v-for="c in CATEGORIES"
          :key="c"
          class="text-muted-foreground inline-flex h-5 items-center gap-0.5 rounded border px-1 text-[11px] font-medium [&_svg]:size-3"
          :class="policy[c].decision === 'allow' ? '' : 'text-foreground border-foreground/30'"
          :data-decision="policy[c].decision"
          :data-category="c"
        >
          {{ fr.mcp.categoryLetter[c] }}
          <DecisionIcon :decision="policy[c].decision" />
        </span>
      </button>
    </TooltipTrigger>
    <TooltipContent
      side="bottom"
      class="max-w-sm text-left"
    >
      <p class="mb-1 font-medium">
        {{ fr.mcp.policy.heading }}
      </p>
      <ul class="grid gap-1">
        <li
          v-for="line in lines"
          :key="line.category"
        >
          <span class="font-medium">{{ line.text }}</span>
          <span class="block opacity-75">{{ line.explanation }}</span>
        </li>
      </ul>
    </TooltipContent>
  </Tooltip>
</template>
