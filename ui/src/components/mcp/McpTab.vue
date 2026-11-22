<script setup lang="ts">
import { ChevronDown, Info, PackagePlus, Search, X } from '@lucide/vue'
import { computed, ref } from 'vue'
import type { AppSnapshot, EnvironmentView } from '@/api/types'
import AppBanner from '@/components/banners/AppBanner.vue'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { TooltipProvider } from '@/components/ui/tooltip'
import { fr } from '@/i18n/fr'
import { terms } from '@/lib/search'
import { useStore } from '@/store/snapshot'
import EnvironmentPills from './EnvironmentPills.vue'
import McpTable from './McpTable.vue'
import ToolCounter from './ToolCounter.vue'
import { type McpRow, environmentChoices, filterRows, rowName, rows, updateBanners } from './model'

const props = defineProps<{ snapshot: AppSnapshot }>()
const store = useStore()

const ALL = 'all'
const query = ref('')
const environment = ref<string>(ALL)
const focus = ref<string | null>(null)

const allRows = computed(() => rows(props.snapshot))
const words = computed(() => terms(query.value))
const visible = computed(() =>
  filterRows(allRows.value, {
    environment: environment.value === ALL ? null : environment.value,
    focus: focus.value,
    query: query.value,
  }),
)
const choices = computed(() => environmentChoices(props.snapshot))
const chosen = computed(() => choices.value.find((c) => (c.id ?? ALL) === environment.value) ?? choices.value[0])
const banners = computed(() => updateBanners(props.snapshot))
const focused = computed(() => allRows.value.find((r) => r.id === focus.value) ?? null)
const notices = computed(() => props.snapshot.alerts.filter((a) => a.level === 'info'))
const noneExposed = computed(
  () => props.snapshot.profile.chosen !== null && props.snapshot.environments.every((e) => !e.exposed),
)
const activeProfile = computed(
  () => props.snapshot.profile.choices.find((p) => p.id === props.snapshot.profile.active)?.label ?? '',
)

/** « Voir »: the table shows this row alone, unfolded. */
function see(row: McpRow) {
  query.value = ''
  environment.value = ALL
  focus.value = row.id
}

// Exposing a sensitive environment is confirmed first; withdrawing one never is (it only hardens).
const pending = ref<EnvironmentView | null>(null)
const asking = ref(false)
function toggleEnvironment(env: EnvironmentView) {
  if (!env.exposed && env.sensitive) {
    pending.value = env
    asking.value = true
    return
  }
  void store.setEnvironmentExposed(env.id, !env.exposed)
}
// The dialog closes before the action's click handler runs: `pending` stays until the next question.
function confirmExposure() {
  const env = pending.value
  asking.value = false
  if (env) void store.setEnvironmentExposed(env.id, true)
}
</script>

<template>
  <TooltipProvider :delay-duration="300">
    <div class="flex flex-col gap-4">
      <div
        v-if="banners.length > 0 || notices.length > 0"
        class="flex flex-col gap-2"
      >
        <AppBanner
          v-for="banner in banners"
          :key="banner.row.id"
          tone="neutral"
          data-banner="mcp-update"
          :data-update="banner.row.id"
        >
          <template #icon>
            <PackagePlus class="size-4" />
          </template>
          {{ banner.text }}
          <template #action>
            <Button
              size="sm"
              variant="outline"
              :aria-label="fr.mcp.update.seeLabel(rowName(banner.row))"
              data-action="see"
              @click="see(banner.row)"
            >
              {{ fr.mcp.update.see }}
            </Button>
            <Button
              size="sm"
              variant="ghost"
              :aria-label="fr.mcp.update.seenLabel(rowName(banner.row))"
              data-action="seen"
              @click="store.acknowledgeMcpUpdate(banner.row.environment.id, banner.row.mcp.id)"
            >
              {{ fr.mcp.update.seen }}
            </Button>
          </template>
        </AppBanner>
        <AppBanner
          v-for="notice in notices"
          :key="notice.kind + notice.message"
          tone="neutral"
          :data-banner="notice.kind"
        >
          <template #icon>
            <Info class="size-4" />
          </template>
          {{ notice.message }}
        </AppBanner>
      </div>

      <section
        aria-labelledby="mcp-environments"
        class="flex flex-col gap-2"
      >
        <div class="flex items-center justify-between gap-3">
          <h2
            id="mcp-environments"
            class="text-muted-foreground text-xs font-medium tracking-wide uppercase"
          >
            {{ fr.mcp.environments.heading }}
          </h2>
          <ToolCounter :snapshot="snapshot" />
        </div>
        <EnvironmentPills
          :snapshot="snapshot"
          @toggle="toggleEnvironment"
        />
        <p
          v-if="noneExposed"
          class="text-muted-foreground text-sm"
          data-empty="exposed"
        >
          {{ fr.mcp.environments.noneExposed }}
        </p>
      </section>

      <section
        class="flex flex-col gap-2"
        :aria-label="fr.tabs.mcp"
      >
        <div class="flex flex-wrap items-center gap-2">
          <div class="relative min-w-56 flex-1">
            <Search
              class="text-muted-foreground pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2"
              aria-hidden="true"
            />
            <Input
              v-model="query"
              type="text"
              role="searchbox"
              :aria-label="fr.mcp.search.label"
              :placeholder="fr.mcp.search.placeholder"
              class="h-8 pr-8 pl-8"
              data-field="search"
              @keydown.escape="query = ''"
            />
            <button
              v-if="query"
              type="button"
              class="text-muted-foreground hover:text-foreground focus-visible:ring-ring/50 absolute top-1/2 right-1.5 flex size-5 -translate-y-1/2 items-center justify-center rounded-sm outline-none focus-visible:ring-3"
              :aria-label="fr.mcp.search.clear"
              @click="query = ''"
            >
              <X
                class="size-3.5"
                aria-hidden="true"
              />
            </button>
          </div>
          <DropdownMenu>
            <DropdownMenuTrigger as-child>
              <Button
                size="sm"
                variant="outline"
                class="w-48 justify-between font-normal"
                :aria-label="`${fr.mcp.filter.label} : ${chosen ? fr.mcp.filter.choice(chosen.label, chosen.count) : ''}`"
                data-field="environment-filter"
              >
                <span class="truncate">{{ chosen ? fr.mcp.filter.choice(chosen.label, chosen.count) : '' }}</span>
                <ChevronDown class="opacity-50" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent
              align="end"
              class="w-48"
            >
              <DropdownMenuLabel class="text-muted-foreground text-xs font-normal">
                {{ fr.mcp.filter.label }}
              </DropdownMenuLabel>
              <DropdownMenuSeparator />
              <DropdownMenuRadioGroup v-model="environment">
                <DropdownMenuRadioItem
                  v-for="choice in choices"
                  :key="choice.id ?? ALL"
                  :value="choice.id ?? ALL"
                  :data-choice="choice.id ?? ALL"
                >
                  <span class="flex-1">{{ choice.label }}</span>
                  <span class="text-muted-foreground text-xs tabular-nums">{{ choice.count }}</span>
                </DropdownMenuRadioItem>
              </DropdownMenuRadioGroup>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
        <div
          v-if="focused"
          class="flex items-center gap-2 text-xs"
        >
          <span
            class="bg-foreground text-background inline-flex h-6 items-center gap-1 rounded-full pr-1 pl-2.5 font-medium"
            data-field="focus"
          >
            {{ fr.mcp.filter.focus(rowName(focused)) }}
            <button
              type="button"
              class="hover:bg-background/20 focus-visible:ring-ring/50 flex size-4 items-center justify-center rounded-full outline-none focus-visible:ring-3"
              :aria-label="fr.mcp.filter.clearFocus"
              data-action="clear-focus"
              @click="focus = null"
            >
              <X
                class="size-3"
                aria-hidden="true"
              />
            </button>
          </span>
        </div>

        <div class="rounded-lg border">
          <McpTable
            :snapshot="snapshot"
            :visible="visible"
            :words="words"
            :focus="focus"
            @set-mcp-exposed="(mcp, exposed) => store.setMcpExposed(mcp, exposed)"
            @reload="(env, mcp) => store.reloadMcp(env, mcp)"
          >
            <template #empty>
              <p class="text-muted-foreground text-sm">
                {{ words.length > 0 ? fr.mcp.empty.noResult(query.trim()) : fr.mcp.empty.noRow }}
              </p>
              <Button
                v-if="words.length > 0"
                size="sm"
                variant="outline"
                class="mt-3"
                data-action="clear-search"
                @click="query = ''"
              >
                {{ fr.mcp.search.clear }}
              </Button>
            </template>
          </McpTable>
        </div>
      </section>

      <AlertDialog
        v-model:open="asking"
      >
        <AlertDialogContent v-if="pending">
          <AlertDialogHeader>
            <AlertDialogTitle>{{ fr.mcp.expose.title(pending.label) }}</AlertDialogTitle>
            <AlertDialogDescription class="grid gap-2">
              <span>{{ fr.mcp.expose.sensitive(pending.label) }}</span>
              <span>{{ fr.mcp.expose.rules(activeProfile) }}</span>
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{{ fr.mcp.expose.cancel }}</AlertDialogCancel>
            <AlertDialogAction
              data-action="confirm-exposure"
              @click="confirmExposure"
            >
              {{ fr.mcp.expose.confirm }}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  </TooltipProvider>
</template>
