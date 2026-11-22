<script setup lang="ts">
import { RotateCw } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { fr } from '@/i18n/fr'
import type { AppSnapshot } from '@/api/types'
import ProfileSelector from './profile/ProfileSelector.vue'

defineProps<{ snapshot: AppSnapshot }>()
</script>

<template>
  <header class="shrink-0 border-b">
    <div class="mx-auto flex h-12 max-w-4xl items-center gap-3 px-6">
      <h1 class="text-sm font-semibold tracking-tight">
        {{ fr.app.name }}
      </h1>
      <span class="text-muted-foreground text-xs tabular-nums">{{ fr.app.version(snapshot.version) }}</span>
      <div class="-mr-2.5 ml-auto flex items-center gap-2">
        <!-- Spec 020 sends `update` and the command that applies it. -->
        <Button
          v-if="snapshot.update"
          size="sm"
          variant="outline"
          disabled
          data-action="update"
        >
          <RotateCw />
          {{ fr.app.update }}
        </Button>
        <ProfileSelector :snapshot="snapshot" />
      </div>
    </div>
  </header>
</template>
