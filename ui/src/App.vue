<script setup lang="ts">
import { onBeforeUnmount, onMounted, provide } from 'vue'
import AppHeader from '@/components/AppHeader.vue'
import GlobalBanners from '@/components/banners/GlobalBanners.vue'
import ConnectionsTab from '@/components/connections/ConnectionsTab.vue'
import JournalTab from '@/components/journal/JournalTab.vue'
import McpTab from '@/components/mcp/McpTab.vue'
import FirstLaunchDialog from '@/components/profile/FirstLaunchDialog.vue'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { TooltipProvider } from '@/components/ui/tooltip'
import { fr } from '@/i18n/fr'
import { STORE, type Store } from '@/store/snapshot'

// `store` is `null` outside the application (no IPC, no simulated backend): nothing to show.
const props = defineProps<{ store: Store | null }>()
if (props.store) provide(STORE, props.store)

onMounted(() => {
  void props.store?.start()
})
onBeforeUnmount(() => {
  props.store?.close()
})
</script>

<template>
  <TooltipProvider :delay-duration="300">
    <div class="flex h-dvh flex-col">
      <template v-if="store && store.state.phase === 'ready' && store.state.snapshot">
        <AppHeader :snapshot="store.state.snapshot" />
        <main class="min-h-0 flex-1 overflow-y-auto">
          <div class="mx-auto flex min-h-full max-w-4xl flex-col gap-4 px-6 py-5">
            <GlobalBanners :snapshot="store.state.snapshot" />
            <Tabs
              default-value="connections"
              class="flex-1 gap-5"
            >
              <TabsList
                :aria-label="fr.tabs.label"
                class="w-fit"
              >
                <TabsTrigger
                  value="connections"
                  class="px-4"
                >
                  {{ fr.tabs.connections }}
                </TabsTrigger>
                <TabsTrigger
                  value="mcp"
                  class="px-4"
                >
                  {{ fr.tabs.mcp }}
                </TabsTrigger>
                <TabsTrigger
                  value="journal"
                  class="px-4"
                >
                  {{ fr.tabs.journal }}
                </TabsTrigger>
              </TabsList>
              <TabsContent value="connections">
                <ConnectionsTab :snapshot="store.state.snapshot" />
              </TabsContent>
              <TabsContent value="mcp">
                <McpTab :snapshot="store.state.snapshot" />
              </TabsContent>
              <!-- The journal fills the window: its list scrolls by itself. -->
              <TabsContent
                value="journal"
                class="flex min-h-0 flex-col"
              >
                <JournalTab :snapshot="store.state.snapshot" />
              </TabsContent>
            </Tabs>
          </div>
        </main>
        <FirstLaunchDialog :snapshot="store.state.snapshot" />
      </template>
      <p
        v-else
        class="text-muted-foreground m-auto max-w-md px-6 text-center text-sm"
        :role="store && store.state.phase === 'loading' ? 'status' : 'alert'"
        data-phase
      >
        <template v-if="!store">
          {{ fr.app.outsideApp }}
        </template>
        <template v-else-if="store.state.phase === 'loading'">
          {{ fr.app.loading }}
        </template>
        <template v-else-if="store.state.phase === 'invalid'">
          {{ fr.app.invalidState }}
        </template>
        <template v-else>
          {{ fr.app.unavailable }}
        </template>
      </p>
    </div>
  </TooltipProvider>
</template>
