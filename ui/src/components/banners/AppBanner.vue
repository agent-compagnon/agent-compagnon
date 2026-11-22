<script setup lang="ts">
import { computed } from 'vue'
import { cn } from '@/lib/utils'

// A global message: orange only for what waits for the user, red for errors, otherwise black and white.
const props = defineProps<{
  tone: 'attention' | 'error' | 'neutral'
}>()

const classes = computed(() =>
  cn(
    'flex min-h-12 items-center gap-3 rounded-lg border px-4 py-2.5 text-sm',
    props.tone === 'attention' && 'border-attention/40 bg-attention-muted text-attention-text',
    props.tone === 'error' && 'border-destructive/40 bg-destructive-muted text-destructive-text',
    props.tone === 'neutral' && 'border-border bg-muted text-foreground',
  ),
)
</script>

<template>
  <div
    :class="classes"
    :role="tone === 'error' ? 'alert' : 'status'"
    :data-tone="tone"
  >
    <span
      v-if="$slots.icon"
      class="flex shrink-0 items-center"
      aria-hidden="true"
    ><slot name="icon" /></span>
    <p class="min-w-0 flex-1 leading-snug">
      <slot />
    </p>
    <div
      v-if="$slots.action"
      class="flex shrink-0 items-center gap-2"
    >
      <slot name="action" />
    </div>
  </div>
</template>
