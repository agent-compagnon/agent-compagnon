<script setup lang="ts">
import { computed } from 'vue'
import { highlight } from '@/lib/search'

// Text from a server, cut into segments and rendered as text: never as HTML.
const props = defineProps<{ text: string; words: readonly string[] }>()
const segments = computed(() => highlight(props.text, props.words))
</script>

<template>
  <template
    v-for="(segment, index) in segments"
    :key="index"
  >
    <mark
      v-if="segment.match"
      class="bg-foreground/15 text-foreground rounded-xs font-semibold"
      data-highlight
    >{{ segment.text }}</mark>
    <template v-else>
      {{ segment.text }}
    </template>
  </template>
</template>
