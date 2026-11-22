<script setup lang="ts">
import { computed } from 'vue'
import { fr } from '@/i18n/fr'
import type { AppSnapshot, ProfileChoice } from '@/api/types'
import { profileEffect } from './model'

const props = defineProps<{ snapshot: AppSnapshot; choice: ProfileChoice }>()
const effect = computed(() => profileEffect(props.snapshot, props.choice))
</script>

<template>
  <p class="text-muted-foreground text-xs">
    {{ fr.profile.environments(effect.exposed) }}
    <template v-if="effect.sensitive.length > 0">
      {{ fr.profile.sensitive(effect.sensitive) }}
    </template>
  </p>
</template>
