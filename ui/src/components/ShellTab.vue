<script setup lang="ts">
// The MCP and Journal tabs arrive with specs 017 and 018.
defineProps<{ text: string }>()
</script>

<template>
  <p class="text-muted-foreground rounded-lg border border-dashed px-4 py-12 text-center text-sm">
    {{ text }}
  </p>
</template>
