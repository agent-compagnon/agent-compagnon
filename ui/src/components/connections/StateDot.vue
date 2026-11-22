<script setup lang="ts">
import { computed } from 'vue'
import type { SessionStateKind } from '@/api/types'
import { cn } from '@/lib/utils'

// Filled: active; outline: never connected; faded: expired; orange: a login waits. The state is also written in
// words next to it: the dot is never the only signal.
const props = defineProps<{ kind: SessionStateKind }>()

const classes = computed(() =>
  cn(
    'inline-block size-2.5 shrink-0 rounded-full',
    props.kind === 'active' && 'bg-foreground',
    props.kind === 'neverConnected' && 'border-foreground/70 border-[1.5px]',
    props.kind === 'expired' && 'bg-muted-foreground/35',
    props.kind === 'loginRequested' && 'bg-attention ring-attention/25 ring-4',
  ),
)
</script>

<template>
  <span
    :class="classes"
    :data-dot="kind"
    aria-hidden="true"
  />
</template>
