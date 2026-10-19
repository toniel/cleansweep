<script setup lang="ts">
import { t } from "../i18n";

defineProps<{
  title: string;
  danger?: boolean;
  busy?: boolean;
}>();

const emit = defineEmits<{ confirm: []; cancel: [] }>();
</script>

<template>
  <div class="modal-backdrop" @click.self="emit('cancel')">
    <div class="modal" role="dialog" :aria-label="title">
      <h3>{{ title }}</h3>
      <div class="muted"><slot /></div>
      <div class="modal-actions">
        <button class="btn" :disabled="busy" @click="emit('cancel')">
          {{ t("common.cancel") }}
        </button>
        <button
          class="btn"
          :class="danger ? 'btn-danger' : 'btn-primary'"
          :disabled="busy"
          @click="emit('confirm')"
        >
          <span v-if="busy" class="spinner"></span>
          {{ t("common.confirm") }}
        </button>
      </div>
    </div>
  </div>
</template>
