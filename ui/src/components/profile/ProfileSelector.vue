<script setup lang="ts">
import { ChevronDown } from '@lucide/vue'
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
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { fr } from '@/i18n/fr'
import type { AppSnapshot } from '@/api/types'
import { useStore } from '@/store/snapshot'
import ProfileEffect from './ProfileEffect.vue'
import { activeChoice, orderedChoices, profileEffect } from './model'

// The profile in force; picking another one asks first, saying what it does.
const props = defineProps<{ snapshot: AppSnapshot }>()
const store = useStore()
const active = computed(() => activeChoice(props.snapshot))
const choices = computed(() => orderedChoices(props.snapshot))
const asked = ref<string | null>(null)
const askedChoice = computed(() => props.snapshot.profile.choices.find((p) => p.id === asked.value))
const askedEffect = computed(() =>
  askedChoice.value === undefined ? null : profileEffect(props.snapshot, askedChoice.value),
)

function pick(value: unknown) {
  if (typeof value === 'string' && value !== props.snapshot.profile.active) asked.value = value
}

function close(open: boolean) {
  if (!open) asked.value = null
}

async function confirm() {
  const profile = asked.value
  asked.value = null
  if (profile !== null) await store.setProfile(profile)
}
</script>

<template>
  <DropdownMenu>
    <DropdownMenuTrigger as-child>
      <Button
        variant="ghost"
        size="sm"
        :aria-label="fr.profile.trigger(active?.label ?? '—')"
        data-action="profile-menu"
      >
        <span class="text-muted-foreground font-normal">{{ fr.profile.label }}</span>
        <span>{{ active?.label ?? '—' }}</span>
        <ChevronDown class="text-muted-foreground" />
      </Button>
    </DropdownMenuTrigger>
    <DropdownMenuContent
      align="end"
      class="w-80"
    >
      <DropdownMenuLabel>{{ fr.profile.label }}</DropdownMenuLabel>
      <DropdownMenuSeparator />
      <DropdownMenuRadioGroup
        :model-value="snapshot.profile.active ?? ''"
        @update:model-value="pick"
      >
        <DropdownMenuRadioItem
          v-for="choice in choices"
          :key="choice.id"
          :value="choice.id"
          class="items-start py-2"
        >
          <span class="grid gap-0.5">
            <span class="font-medium">{{ choice.label }}</span>
            <ProfileEffect
              :snapshot="snapshot"
              :choice="choice"
            />
          </span>
        </DropdownMenuRadioItem>
      </DropdownMenuRadioGroup>
    </DropdownMenuContent>
  </DropdownMenu>

  <AlertDialog
    :open="askedChoice !== undefined"
    @update:open="close"
  >
    <AlertDialogContent data-dialog="profile-change">
      <AlertDialogHeader>
        <AlertDialogTitle>{{ fr.profile.change.title(askedChoice?.label ?? '') }}</AlertDialogTitle>
        <AlertDialogDescription as="div">
          <div class="grid gap-2">
            <p v-if="askedChoice?.description">
              {{ askedChoice.description }}
            </p>
            <p>{{ fr.profile.change.exposed(askedEffect?.exposed ?? []) }}</p>
            <p v-if="askedEffect && askedEffect.sensitive.length > 0">
              {{ fr.profile.change.sensitive(askedEffect.sensitive) }}
            </p>
            <p>{{ fr.profile.change.kept }}</p>
          </div>
        </AlertDialogDescription>
      </AlertDialogHeader>
      <AlertDialogFooter>
        <AlertDialogCancel>{{ fr.profile.change.cancel }}</AlertDialogCancel>
        <Button
          data-action="confirm-profile"
          @click="confirm"
        >
          {{ fr.profile.change.confirm }}
        </Button>
      </AlertDialogFooter>
    </AlertDialogContent>
  </AlertDialog>
</template>
