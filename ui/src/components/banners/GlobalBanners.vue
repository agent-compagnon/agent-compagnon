<script setup lang="ts">
import { CircleAlert, CirclePause, ShieldQuestion } from '@lucide/vue'
import { computed } from 'vue'
import { Button } from '@/components/ui/button'
import { fr } from '@/i18n/fr'
import type { AppSnapshot } from '@/api/types'
import { useStore } from '@/store/snapshot'
import AppBanner from './AppBanner.vue'

// The banners of the whole window: a waiting confirmation (« Voir » opens its detail window, the only one that
// shows the full arguments and the only place to approve), the suspended gateway, the errors to act on.
const props = defineProps<{ snapshot: AppSnapshot }>()
const store = useStore()

// Information and tool-count alerts belong to the MCP and Journal tabs (specs 017, 018).
const errors = computed(() => props.snapshot.alerts.filter((a) => a.level === 'error'))
</script>

<template>
  <div
    v-if="snapshot.pendingConfirmations > 0 || snapshot.gatewaySuspended || errors.length > 0 || store.state.commandError"
    class="flex flex-col gap-2"
  >
    <AppBanner
      v-if="snapshot.pendingConfirmations > 0"
      tone="attention"
      data-banner="confirmations"
    >
      <template #icon>
        <ShieldQuestion class="size-4" />
      </template>
      {{ fr.banners.confirmations(snapshot.pendingConfirmations) }}
      <template #action>
        <Button
          size="sm"
          variant="attention"
          @click="store.showConfirmations()"
        >
          {{ fr.banners.see }}
        </Button>
      </template>
    </AppBanner>
    <AppBanner
      v-if="snapshot.gatewaySuspended"
      tone="neutral"
      data-banner="suspended"
    >
      <template #icon>
        <CirclePause class="size-4" />
      </template>
      {{ fr.banners.suspended }}
      <template #action>
        <Button
          size="sm"
          variant="outline"
          @click="store.resume()"
        >
          {{ fr.banners.resume }}
        </Button>
      </template>
    </AppBanner>
    <AppBanner
      v-for="alert in errors"
      :key="alert.kind + alert.message"
      tone="error"
      :data-banner="alert.kind"
    >
      <template #icon>
        <CircleAlert class="size-4" />
      </template>
      {{ alert.message }}
    </AppBanner>
    <AppBanner
      v-if="store.state.commandError"
      tone="error"
      data-banner="command"
    >
      <template #icon>
        <CircleAlert class="size-4" />
      </template>
      {{ fr.banners.commandFailed(store.state.commandError) }}
      <template #action>
        <Button
          size="sm"
          variant="ghost"
          @click="store.dismissError()"
        >
          {{ fr.banners.dismiss }}
        </Button>
      </template>
    </AppBanner>
  </div>
</template>
