<script setup lang="ts">
import { computed, ref } from 'vue'
import {
  AlertDialog,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'
import { RadioGroup, RadioGroupItem } from '@/components/ui/radio-group'
import { fr } from '@/i18n/fr'
import type { AppSnapshot } from '@/api/types'
import { useStore } from '@/store/snapshot'
import ProfileEffect from './ProfileEffect.vue'
import { orderedChoices } from './model'

// Blocking: no profile chosen yet (first launch, or settings reset), nothing is exposed until one is. Neither
// Escape nor a click outside closes it.
const props = defineProps<{ snapshot: AppSnapshot }>()
const store = useStore()
const selected = ref<string>('')
const sending = ref(false)
const choices = computed(() => orderedChoices(props.snapshot))
const reset = computed(() => props.snapshot.alerts.find((a) => a.kind === 'settingsReset'))

async function confirm() {
  if (selected.value === '' || sending.value) return
  sending.value = true
  await store.setProfile(selected.value)
  sending.value = false
}
</script>

<template>
  <AlertDialog :open="snapshot.profile.firstLaunch">
    <AlertDialogContent
      class="sm:max-w-xl"
      data-dialog="first-launch"
      @escape-key-down.prevent
    >
      <AlertDialogHeader>
        <AlertDialogTitle>{{ fr.profile.firstLaunch.title }}</AlertDialogTitle>
        <AlertDialogDescription>{{ fr.profile.firstLaunch.description }}</AlertDialogDescription>
      </AlertDialogHeader>
      <p
        v-if="reset"
        class="border-destructive/40 bg-destructive-muted text-destructive-text rounded-md border px-3 py-2 text-sm"
        role="alert"
      >
        {{ reset.message }}
      </p>
      <RadioGroup
        v-model="selected"
        :aria-label="fr.profile.firstLaunch.choices"
        class="gap-2"
      >
        <label
          v-for="choice in choices"
          :key="choice.id"
          :for="`profile-${choice.id}`"
          class="hover:bg-accent/60 has-[[data-state=checked]]:border-foreground flex cursor-pointer items-start gap-3 rounded-md border px-3 py-2.5"
        >
          <RadioGroupItem
            :id="`profile-${choice.id}`"
            :value="choice.id"
            :aria-labelledby="`profile-${choice.id}-label`"
            :aria-describedby="`profile-${choice.id}-effect`"
            class="mt-0.5"
          />
          <span class="grid gap-0.5">
            <span
              :id="`profile-${choice.id}-label`"
              class="text-sm font-medium"
            >{{ choice.label }}</span>
            <span
              :id="`profile-${choice.id}-effect`"
              class="grid gap-0.5"
            >
              <span
                v-if="choice.description"
                class="text-sm"
              >{{ choice.description }}</span>
              <ProfileEffect
                :snapshot="snapshot"
                :choice="choice"
              />
            </span>
          </span>
        </label>
      </RadioGroup>
      <AlertDialogFooter>
        <Button
          :disabled="selected === '' || sending"
          data-action="choose-profile"
          @click="confirm"
        >
          {{ fr.profile.firstLaunch.confirm }}
        </Button>
      </AlertDialogFooter>
    </AlertDialogContent>
  </AlertDialog>
</template>
