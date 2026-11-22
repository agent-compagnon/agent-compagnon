<script setup lang="ts">
import { Ban, Bell, Check, MessageSquareText } from '@lucide/vue'
import type { DecisionKind } from '@/api/types'

// Black and white: a refusal protects, it is not an error. The decision is also written next to it or in the
// tooltip: the icon is never the only signal.
defineProps<{ decision: DecisionKind }>()
</script>

<template>
  <Check
    v-if="decision === 'allow'"
    aria-hidden="true"
  />
  <MessageSquareText
    v-else-if="decision === 'confirmElicitation'"
    aria-hidden="true"
  />
  <Bell
    v-else-if="decision === 'confirmNotification'"
    aria-hidden="true"
  />
  <Ban
    v-else
    aria-hidden="true"
  />
</template>
