<script setup lang="ts">
import { computed } from 'vue'
import { Button } from '@/components/ui/button'
import { fr } from '@/i18n/fr'
import type { AppSnapshot, SessionView } from '@/api/types'
import { cn } from '@/lib/utils'
import { useStore } from '@/store/snapshot'
import StateDot from './StateDot.vue'
import { canDisconnect, servicesText, stateText } from './model'

const props = defineProps<{ session: SessionView; snapshot: AppSnapshot; now: number }>()
const store = useStore()
const state = computed(() => stateText(props.session, props.now))
const services = computed(() => servicesText(props.session, props.snapshot))
const waiting = computed(() => props.session.state.kind === 'loginRequested')
</script>

<template>
  <li
    class="bg-card text-card-foreground flex items-start gap-3 rounded-lg border px-4 py-3"
    :data-session="session.id"
    :data-state="session.state.kind"
  >
    <StateDot
      :kind="session.state.kind"
      class="mt-1.5"
    />
    <div class="grid min-w-0 flex-1 gap-0.5">
      <h3 class="truncate text-sm font-medium">
        {{ session.label }}
      </h3>
      <p
        :class="cn('text-sm', waiting ? 'text-attention-text font-medium' : 'text-muted-foreground')"
        data-field="state"
      >
        {{ state }}
      </p>
      <p
        class="text-muted-foreground text-xs leading-relaxed"
        data-field="services"
      >
        <span class="sr-only">{{ fr.connections.services }} : </span>{{ services }}
      </p>
    </div>
    <Button
      v-if="canDisconnect(session)"
      size="sm"
      variant="outline"
      class="shrink-0"
      :aria-label="fr.connections.disconnectFrom(session.label)"
      data-action="disconnect"
      @click="store.disconnect(session.id)"
    >
      {{ fr.connections.disconnect }}
    </Button>
    <Button
      v-else
      size="sm"
      variant="outline"
      class="shrink-0"
      :aria-label="fr.connections.connectTo(session.label)"
      data-action="connect"
      @click="store.connect(session.id)"
    >
      {{ fr.connections.connect }}
    </Button>
  </li>
</template>
