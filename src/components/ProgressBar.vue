<script setup lang="ts">
defineProps<{ percent: number | null }>();
</script>

<template>
  <div class="progress" :class="{ 'progress-indeterminate': percent == null }" role="progressbar">
    <div v-if="percent != null" :style="{ width: `${Math.min(100, percent)}%` }"></div>
    <div v-else></div>
  </div>
</template>
