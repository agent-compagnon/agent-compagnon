<script setup lang="ts">
import { KeyRound } from '@lucide/vue'
import { computed } from 'vue'
import AppBanner from '@/components/banners/AppBanner.vue'
import { Button } from '@/components/ui/button'
import { fr } from '@/i18n/fr'
import type { AppSnapshot } from '@/api/types'
import { useNow } from '@/lib/now'
import { useStore } from '@/store/snapshot'
import SessionCard from './SessionCard.vue'
import { loginBanners, sessionGroups } from './model'

const props = defineProps<{ snapshot: AppSnapshot; now?: number }>()
const store = useStore()
const clock = useNow()
const now = computed(() => props.now ?? clock.value)
const groups = computed(() => sessionGroups(props.snapshot))
const banners = computed(() => loginBanners(props.snapshot))
</script>

<template>
  <div class="flex flex-col gap-6">
    <div
      v-if="banners.length > 0"
      class="flex flex-col gap-2"
    >
      <AppBanner
        v-for="banner in banners"
        :key="banner.session"
        tone="attention"
        data-banner="login"
      >
        <template #icon>
          <KeyRound class="size-4" />
        </template>
        {{ banner.text }}
        <template #action>
          <Button
            size="sm"
            variant="attention"
            @click="store.connect(banner.session)"
          >
            {{ fr.banners.connect }}
          </Button>
        </template>
      </AppBanner>
    </div>

    <section
      aria-labelledby="sessions-global"
      class="flex flex-col gap-2.5"
    >
      <h2
        id="sessions-global"
        class="text-muted-foreground text-xs font-medium tracking-wide uppercase"
      >
        {{ fr.connections.global }}
      </h2>
      <ul class="grid grid-cols-1 gap-2.5 sm:grid-cols-2">
        <SessionCard
          v-for="session in groups.global"
          :key="session.id"
          :session="session"
          :snapshot="snapshot"
          :now="now"
        />
      </ul>
    </section>

    <section
      aria-labelledby="sessions-env"
      class="flex flex-col gap-2.5"
    >
      <h2
        id="sessions-env"
        class="text-muted-foreground text-xs font-medium tracking-wide uppercase"
      >
        {{ fr.connections.perEnvironment }}
      </h2>
      <ul
        v-if="groups.perEnvironment.length > 0"
        class="grid grid-cols-1 gap-2.5 sm:grid-cols-2"
      >
        <SessionCard
          v-for="session in groups.perEnvironment"
          :key="session.id"
          :session="session"
          :snapshot="snapshot"
          :now="now"
        />
      </ul>
      <p
        v-else
        class="text-muted-foreground rounded-lg border border-dashed px-4 py-6 text-center text-sm"
        data-empty="environments"
      >
        {{ fr.connections.noEnvironment }}
      </p>
    </section>
  </div>
</template>
