<script setup lang="ts">
import { ref, watch } from 'vue'
import {
  AlertDialog,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'
import { reason } from '@/api/backend'
import type { DoctorReport } from '@/api/types'
import { fr } from '@/i18n/fr'
import { useStore } from '@/store/snapshot'
import { mark, shownFix, statusClass, summaryText, typography } from './model'

// « Auto-diagnostic » (spec 024): the application runs every check, read only, then the window shows the report and
// lets the user copy its text. Nothing here changes anything; « Relancer » runs the checks again.
const props = defineProps<{ open: boolean }>()
const emit = defineEmits<{ 'update:open': [open: boolean] }>()
const store = useStore()

const report = ref<DoctorReport | null>(null)
const running = ref(false)
const error = ref<string | null>(null)
const copy = ref<boolean | null>(null)
// An answer that arrives after the dialog was closed, or after a new run, is not shown.
let run = 0

async function diagnose() {
  const mine = ++run
  running.value = true
  error.value = null
  copy.value = null
  report.value = null
  try {
    const found = await store.runDoctor()
    if (mine === run) report.value = found
  } catch (e) {
    if (mine === run) error.value = fr.doctor.failed(reason(e))
  } finally {
    if (mine === run) running.value = false
  }
}

watch(
  () => props.open,
  (open) => {
    if (open) {
      void diagnose()
    } else {
      run += 1
    }
  },
  { immediate: true },
)

// The clipboard of the WebView (a click is a user gesture); refused, the text stays shown to copy by hand.
async function copyReport() {
  if (report.value === null) return
  try {
    await navigator.clipboard.writeText(report.value.text)
    copy.value = true
  } catch {
    copy.value = false
  }
}
</script>

<template>
  <AlertDialog
    :open="open"
    @update:open="(value) => emit('update:open', value)"
  >
    <!-- The header and the buttons stay in view; the report scrolls between them. -->
    <AlertDialogContent
      class="flex max-h-[calc(100dvh-2rem)] flex-col sm:max-w-2xl"
      data-dialog="doctor"
      :data-phase="running ? 'running' : report ? 'done' : 'failed'"
    >
      <AlertDialogHeader class="shrink-0">
        <AlertDialogTitle>{{ fr.doctor.title }}</AlertDialogTitle>
        <AlertDialogDescription class="text-sm">
          {{ fr.doctor.intro }}
        </AlertDialogDescription>
      </AlertDialogHeader>

      <div
        class="-mx-1 min-h-0 overflow-y-auto px-1"
        data-scroll
      >
        <div
          v-if="running"
          class="grid gap-1 py-6 text-center text-sm"
          role="status"
          aria-live="polite"
          data-running
        >
          <p class="font-medium">
            {{ fr.doctor.running }}
          </p>
          <p class="text-muted-foreground text-xs">
            {{ fr.doctor.runningHint }}
          </p>
        </div>

        <div
          v-else-if="report"
          class="grid min-w-0 gap-3 text-sm"
        >
          <p
            class="font-medium"
            :class="report.status === 'ok' ? '' : statusClass(report.status)"
            role="status"
            data-summary
            :data-status="report.status"
          >
            {{ typography(summaryText(report)) }}
          </p>
          <ul class="divide-y rounded-md border">
            <li
              v-for="check in report.checks"
              :key="check.id"
              class="grid min-w-0 gap-1 p-3"
              :data-check="check.id"
              :data-status="check.status"
            >
              <div class="flex items-baseline gap-3">
                <p class="min-w-0 flex-1 font-medium">
                  {{ check.name }}
                </p>
                <span
                  class="shrink-0 text-xs font-medium"
                  :class="statusClass(check.status)"
                  data-check-status
                >{{ fr.doctor.status[check.status] }}</span>
              </div>
              <div
                v-for="(finding, i) in check.findings"
                :key="i"
                class="grid min-w-0 grid-cols-[1rem_1fr] gap-x-1 text-xs"
                :data-finding="finding.status"
              >
                <span
                  aria-hidden="true"
                  class="font-semibold"
                  :class="statusClass(finding.status)"
                >{{ mark(finding.status) }}</span>
                <p class="min-w-0 break-words">
                  {{ typography(finding.message) }}
                </p>
                <p
                  v-if="shownFix(check, i)"
                  class="text-muted-foreground col-start-2 min-w-0 break-words"
                  data-fix
                >
                  <span class="text-foreground font-medium">{{ fr.doctor.fix }}</span>
                  {{ typography(shownFix(check, i) ?? '') }}
                </p>
              </div>
            </li>
          </ul>
          <p
            v-if="report.file"
            class="text-muted-foreground text-xs"
            data-file
          >
            {{ fr.doctor.saved }}
            <code class="text-foreground font-mono break-all">{{ report.file }}</code>
          </p>
          <p
            v-else-if="report.fileError"
            class="text-destructive-text text-xs"
            data-file-error
          >
            {{ fr.doctor.notSaved(report.fileError) }}
          </p>
          <p
            v-if="copy !== null"
            class="text-xs"
            role="status"
            data-copy
          >
            {{ copy ? fr.doctor.copied : fr.doctor.copyFailed }}
          </p>
          <pre
            v-if="copy === false"
            class="bg-muted rounded-md p-3 font-mono text-xs break-all whitespace-pre-wrap select-all"
            data-copied
          >{{ report.text }}</pre>
        </div>

        <p
          v-if="error"
          class="text-destructive-text text-sm"
          role="alert"
          data-error
        >
          {{ error }}
        </p>
      </div>

      <AlertDialogFooter class="shrink-0">
        <Button
          variant="outline"
          :disabled="running"
          data-action="doctor-again"
          @click="diagnose"
        >
          {{ fr.doctor.again }}
        </Button>
        <Button
          variant="outline"
          :disabled="report === null"
          data-action="doctor-copy"
          @click="copyReport"
        >
          {{ fr.doctor.copy }}
        </Button>
        <Button
          data-action="doctor-close"
          @click="emit('update:open', false)"
        >
          {{ fr.doctor.close }}
        </Button>
      </AlertDialogFooter>
    </AlertDialogContent>
  </AlertDialog>
</template>
