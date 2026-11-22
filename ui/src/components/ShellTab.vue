<script setup lang="ts">
// The Journal tab arrives with spec 018.
defineProps<{ text: string }>()
</script>

<template>
  <p class="text-muted-foreground rounded-lg border border-dashed px-4 py-12 text-center text-sm">
    {{ text }}
  </p>
</template>
