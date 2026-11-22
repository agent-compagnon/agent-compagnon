<script setup lang="ts">
import { computed } from 'vue'
import type { DetailsView } from '@/api/types'
import { fr } from '@/i18n/fr'
import { prettyJson } from './model'

// What the core kept of a call that did not end normally, or a slow one. Already filtered: shown as text, nothing
// is interpreted, nothing is filtered again here.
const props = defineProps<{ details: DetailsView; id: string }>()

const masked = computed(() => props.details.masked.exact + props.details.masked.generic)
const kinds = computed(() => props.details.masked.kinds.map((k) => fr.journal.maskedKind[k]))
const request = computed(() => prettyJson(props.details.request))
const response = computed(() => prettyJson(props.details.response))
</script>

<template>
  <div
    :id="id"
    class="flex flex-col gap-3 pt-1 pr-3 pb-3 pl-10 text-xs"
    data-details
  >
    <dl class="grid grid-cols-[8.5rem_minmax(0,1fr)] gap-x-3 gap-y-1">
      <dt class="text-muted-foreground">
        {{ fr.journal.details.trace }}
      </dt>
      <dd
        class="font-mono tabular-nums"
        data-detail="trace"
      >
        {{ details.callId }}
      </dd>
      <dt class="text-muted-foreground">
        {{ fr.journal.details.client }}
      </dt>
      <dd data-detail="client">
        {{ details.client }}
      </dd>
      <dt class="text-muted-foreground">
        {{ fr.journal.details.session }}
      </dt>
      <dd
        class="font-mono"
        data-detail="session"
      >
        {{ details.session ?? fr.journal.details.noSession }}
      </dd>
      <dt class="text-muted-foreground">
        {{ fr.journal.details.policy }}
      </dt>
      <dd data-detail="policy">
        <template v-if="details.policy">
          {{ details.policy.explanation }}
        </template>
        <template v-else>
          {{ fr.journal.details.noPolicy }}
        </template>
      </dd>
      <template v-if="masked > 0">
        <dt class="text-muted-foreground">
          {{ fr.journal.details.masked }}
        </dt>
        <dd data-detail="masked">
          {{ fr.journal.details.maskedCount(masked, kinds) }}
        </dd>
      </template>
    </dl>
    <div class="grid grid-cols-2 gap-3">
      <figure class="flex min-w-0 flex-col gap-1">
        <figcaption class="text-muted-foreground">
          {{ fr.journal.details.request }}
        </figcaption>
        <pre
          class="bg-muted max-h-56 overflow-auto rounded-md px-3 py-2 font-mono text-[0.6875rem] leading-relaxed break-words whitespace-pre-wrap"
          data-detail="request"
        >{{ request }}</pre>
      </figure>
      <figure class="flex min-w-0 flex-col gap-1">
        <figcaption class="text-muted-foreground">
          {{ fr.journal.details.response }}
        </figcaption>
        <pre
          class="bg-muted max-h-56 overflow-auto rounded-md px-3 py-2 font-mono text-[0.6875rem] leading-relaxed break-words whitespace-pre-wrap"
          data-detail="response"
        >{{ response }}</pre>
      </figure>
    </div>
  </div>
</template>
