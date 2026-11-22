<script setup lang="ts">
import { RotateCw } from '@lucide/vue'
import { computed, ref } from 'vue'
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'
import { fr } from '@/i18n/fr'
import type { AppSnapshot } from '@/api/types'
import { useStore } from '@/store/snapshot'
import ProfileSelector from './profile/ProfileSelector.vue'

// An update put in place (spec 020): restarting applies it, and closes every session, which the dialog says first.
const props = defineProps<{ snapshot: AppSnapshot }>()
const store = useStore()
const asking = ref(false)
const update = computed(() => props.snapshot.update)
const help = computed(() => {
  const ready = update.value
  if (ready === null) return ''
  return ready.critical ? fr.app.updateCriticalHelp(ready.version) : fr.app.updateHelp(ready.version)
})

async function restart() {
  asking.value = false
  await store.restartForUpdate()
}
</script>

<template>
  <header class="shrink-0 border-b">
    <div class="mx-auto flex h-12 max-w-4xl items-center gap-3 px-6">
      <h1 class="text-sm font-semibold tracking-tight">
        {{ fr.app.name }}
      </h1>
      <span class="text-muted-foreground text-xs tabular-nums">{{ fr.app.version(snapshot.version) }}</span>
      <div class="-mr-2.5 ml-auto flex items-center gap-2">
        <template v-if="update">
          <span
            v-if="update.critical"
            class="text-xs font-medium"
            data-update-critical
          >{{ fr.app.updateCritical }}</span>
          <span
            id="update-help"
            class="sr-only"
          >{{ help }}</span>
          <Button
            size="sm"
            :variant="update.critical ? 'default' : 'outline'"
            aria-describedby="update-help"
            data-action="update"
            @click="asking = true"
          >
            <RotateCw />
            {{ fr.app.update }}
          </Button>
        </template>
        <ProfileSelector :snapshot="snapshot" />
      </div>
    </div>
  </header>

  <AlertDialog
    :open="asking && update !== null"
    @update:open="(open) => (asking = open)"
  >
    <AlertDialogContent data-dialog="restart">
      <AlertDialogHeader>
        <AlertDialogTitle>{{ fr.app.restart.title(update?.version ?? '') }}</AlertDialogTitle>
        <AlertDialogDescription as="div">
          <div class="grid gap-2">
            <p>{{ help }}</p>
            <p>{{ fr.app.restart.sessions }}</p>
          </div>
        </AlertDialogDescription>
      </AlertDialogHeader>
      <AlertDialogFooter>
        <AlertDialogCancel>{{ fr.app.restart.cancel }}</AlertDialogCancel>
        <Button
          data-action="confirm-restart"
          @click="restart"
        >
          {{ fr.app.restart.confirm }}
        </Button>
      </AlertDialogFooter>
    </AlertDialogContent>
  </AlertDialog>
</template>
