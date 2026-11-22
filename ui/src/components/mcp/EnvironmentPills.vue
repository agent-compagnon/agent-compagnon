<script setup lang="ts">
import { ShieldAlert } from '@lucide/vue'
import type { AppSnapshot, EnvironmentView } from '@/api/types'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { fr } from '@/i18n/fr'
import { cn } from '@/lib/utils'

// One pastille per environment: pressed when exposed to the LLM clients. Before a profile is chosen, nothing can
// be exposed, and the pastilles say so.
defineProps<{ snapshot: AppSnapshot }>()
const emit = defineEmits<{ toggle: [environment: EnvironmentView] }>()
</script>

<template>
  <ul
    class="flex flex-wrap items-center gap-1.5"
    :aria-label="fr.mcp.environments.heading"
  >
    <li
      v-for="environment in snapshot.environments"
      :key="environment.id"
    >
      <Tooltip>
        <TooltipTrigger as-child>
          <button
            type="button"
            :aria-pressed="environment.exposed"
            :aria-disabled="snapshot.profile.chosen === null ? 'true' : undefined"
            :class="
              cn(
                'focus-visible:ring-ring/50 inline-flex h-7 items-center gap-1 rounded-full border px-3 text-xs font-medium outline-none transition-colors focus-visible:ring-3',
                environment.exposed
                  ? 'bg-foreground text-background border-foreground'
                  : 'text-muted-foreground hover:text-foreground hover:border-foreground/40 border-dashed',
                snapshot.profile.chosen === null && 'cursor-not-allowed opacity-50',
              )
            "
            :data-environment="environment.id"
            :data-exposed="environment.exposed"
            @click="snapshot.profile.chosen !== null && emit('toggle', environment)"
          >
            <ShieldAlert
              v-if="environment.sensitive"
              class="size-3"
              aria-hidden="true"
            />
            {{ environment.label }}
          </button>
        </TooltipTrigger>
        <TooltipContent
          side="bottom"
          class="max-w-xs"
        >
          <p class="font-medium">
            {{ environment.description || environment.label }}
          </p>
          <p v-if="environment.sensitive">
            {{ fr.mcp.environments.sensitive }}
          </p>
          <p>{{ environment.exposed ? fr.mcp.environments.exposed : fr.mcp.environments.notExposed }}</p>
          <p class="opacity-75">
            {{
              snapshot.profile.chosen === null
                ? fr.mcp.environments.noProfile
                : environment.exposed
                  ? fr.mcp.environments.toggleOff(environment.label)
                  : fr.mcp.environments.toggleOn(environment.label)
            }}
          </p>
        </TooltipContent>
      </Tooltip>
    </li>
  </ul>
</template>
