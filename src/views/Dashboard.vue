<script setup lang="ts">
import { onMounted, ref } from "vue";
import { api } from "../api";
import { formatBytes, formatDate } from "../format";
import { t } from "../i18n";
import ProgressBar from "../components/ProgressBar.vue";
import type { DockerStatus, DriveInfo, HistoryEntry, ScanProgress } from "../types";

const drives = ref<DriveInfo[]>([]);
const history = ref<HistoryEntry[]>([]);
const docker = ref<DockerStatus | null>(null);
const scanning = ref(false);
const progress = ref<ScanProgress | null>(null);
const smartResult = ref<{ junkBytes: number; dockerBytes: number } | null>(null);

function driveClass(d: DriveInfo): string {
  const pct = d.totalBytes ? d.usedBytes / d.totalBytes : 0;
  return pct > 0.9 ? "danger" : pct > 0.75 ? "warn" : "";
}

async function refresh() {
  [drives.value, history.value, docker.value] = await Promise.all([
    api.listDrives(),
    api.listHistory(),
    api.dockerStatus(),
  ]);
}

async function smartScan() {
  scanning.value = true;
  progress.value = null;
  smartResult.value = null;
  try {
    const [junk, dockerOv] = await Promise.all([
      api.scanJunk(null, (p) => (progress.value = p)),
      api.dockerOverview().catch(() => null),
    ]);
    smartResult.value = {
      junkBytes: junk.totalBytes,
      dockerBytes: dockerOv?.reclaimableBytes ?? 0,
    };
  } finally {
    scanning.value = false;
  }
}

onMounted(refresh);
</script>

<template>
  <div>
    <h1 class="page-title">{{ t("dash.title") }}</h1>

    <div class="card">
      <div class="row-between">
        <div>
          <strong>{{ t("dash.smartScan") }}</strong>
          <div class="muted" v-if="smartResult">
            {{ t("dash.potential") }}:
            <strong>{{ formatBytes(smartResult.junkBytes + smartResult.dockerBytes) }}</strong>
            ({{ formatBytes(smartResult.junkBytes) }} junk · {{ formatBytes(smartResult.dockerBytes) }} docker)
          </div>
          <div class="muted" v-else-if="progress">{{ progress.current }}</div>
        </div>
        <button class="btn btn-primary" :disabled="scanning" @click="smartScan">
          <span v-if="scanning" class="spinner"></span>
          <font-awesome-icon
            v-else
            icon="fa-solid fa-magnifying-glass"
            aria-hidden="true"
          />
          {{ scanning ? t("dash.scanning") : t("dash.smartScan") }}
        </button>
      </div>
      <ProgressBar v-if="scanning" class="mt" :percent="progress?.percent ?? null" />
    </div>

    <h2 class="mt mb">{{ t("dash.drives") }}</h2>
    <div class="grid grid-3">
      <div v-for="d in drives" :key="d.mountPoint" class="card">
        <div class="row-between">
          <strong class="ellipsis">{{ d.name || d.mountPoint }}</strong>
          <span class="badge badge-muted">{{ d.fsType }}</span>
        </div>
        <div class="drive-bar" :class="driveClass(d)">
          <div :style="{ width: `${(d.usedBytes / Math.max(1, d.totalBytes)) * 100}%` }"></div>
        </div>
        <div class="muted mt" style="font-size: 12.5px">
          {{ formatBytes(d.usedBytes) }} / {{ formatBytes(d.totalBytes) }} ·
          {{ formatBytes(d.freeBytes) }} free · {{ d.mountPoint }}
        </div>
      </div>
    </div>

    <h2 class="mt mb">{{ t("dash.history") }}</h2>
    <div class="card">
      <div v-if="!history.length" class="empty-state">{{ t("dash.noHistory") }}</div>
      <table v-else class="tbl">
        <thead>
          <tr>
            <th>{{ t("common.status") }}</th>
            <th>{{ t("common.name") }}</th>
            <th>{{ t("common.size") }}</th>
            <th>{{ t("common.created") }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="h in history" :key="h.id">
            <td><span class="badge badge-info">{{ h.category }}</span></td>
            <td class="muted">{{ h.itemCount }} {{ t("junk.items") }} · {{ h.mode }}</td>
            <td class="nowrap">{{ formatBytes(h.freedBytes) }}</td>
            <td class="muted nowrap">{{ formatDate(Date.parse(h.at)) }}</td>
          </tr>
        </tbody>
      </table>
    </div>

    <p v-if="docker && !docker.available" class="muted mt">
      <font-awesome-icon icon="fa-solid fa-triangle-exclamation" aria-hidden="true" />
      {{ t("dash.dockerUnavailable") }}: {{ docker.error }}
    </p>
  </div>
</template>
