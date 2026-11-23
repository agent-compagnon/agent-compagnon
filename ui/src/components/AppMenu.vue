<script setup lang="ts">
import { Menu } from '@lucide/vue'
import { ref } from 'vue'
import ClientsDialog from '@/components/clients/ClientsDialog.vue'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { fr } from '@/i18n/fr'

// The menu of the header: the tools that are not a tab. « Clients LLM… » (spec 023); « Auto-diagnostic » comes next
// (spec 024). Each entry opens its own dialog.
const clients = ref(false)
</script>

<template>
  <DropdownMenu :modal="false">
    <DropdownMenuTrigger as-child>
      <Button
        variant="ghost"
        size="icon-sm"
        :aria-label="fr.menu.trigger"
        data-action="app-menu"
      >
        <Menu />
      </Button>
    </DropdownMenuTrigger>
    <DropdownMenuContent
      align="end"
      class="w-48"
    >
      <DropdownMenuItem
        data-menu="clients"
        @select="clients = true"
      >
        {{ fr.menu.clients }}
      </DropdownMenuItem>
    </DropdownMenuContent>
  </DropdownMenu>
  <ClientsDialog v-model:open="clients" />
</template>
