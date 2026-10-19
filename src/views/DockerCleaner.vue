<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { api } from "../api";
import { formatBytes, formatDate, shortId } from "../format";
import { t } from "../i18n";
import ConfirmDialog from "../components/ConfirmDialog.vue";
import ProgressBar from "../components/ProgressBar.vue";
import type {
  DeleteOutcome,
  DockerOverview,
  DockerResourceKind,
  ScanProgress,
} from "../types";

const overview = ref<DockerOverview | null>(null);
const loading = ref(true);
const deleting = ref(false);
const progress = ref<ScanProgress | null>(null);
const error = ref("");
const outcomes = ref<DeleteOutcome[]>([]);
const tab = ref<"images" | "containers" | "volumes" | "networks">("images");
const dryRun = ref(false);
const showConfirm = ref(false);
const showVolumeWarn = ref(false);
const sortBySize = ref(true);

const checked = ref<Set<string>>(new Set());

const removableImages = computed(
  () => overview.value?.images.filter((i) => !i.pinned && !i.inUseRunning) ?? [],
);
const stoppedContainers = computed(
  () =>
    overview.value?.containers.filter((c) =>
      ["exited", "created", "dead"].includes(c.state),
    ) ?? [],
);
const unusedVolumes = computed(
  () => overview.value?.volumes.filter((v) => !v.inUse) ?? [],
);
const unusedNetworks = computed(
  () => overview.value?.networks.filter((n) => !n.inUse && !n.builtin) ?? [],
);

const sorted = computed(() => {
  const bySize = (a: { sizeBytes?: number | null }, b: { sizeBytes?: number | null }) =>
    (b.sizeBytes ?? 0) - (a.sizeBytes ?? 0);
  const byName = <T extends { name?: string; tags?: string[] }>(a: T, b: T) =>
    (a.name ?? a.tags?.[0] ?? "").localeCompare(b.name ?? b.tags?.[0] ?? "");
  const cmp = sortBySize.value ? bySize : byName;
  return {
    images: [...removableImages.value].sort(cmp as never),
    containers: [...stoppedContainers.value].sort(cmp as never),
    volumes: [...unusedVolumes.value].sort(cmp as never),
    networks: [...unusedNetworks.value],
  };
});

function toggle(id: string, on: boolean) {
  const s = new Set(checked.value);
  on ? s.add(id) : s.delete(id);
  checked.value = s;
}

function currentKind(): DockerResourceKind {
  return (
    { images: "image", containers: "container", volumes: "volume", networks: "network" } as const
  )[tab.value];
}

function selectSuggested() {
  // Suggested: dangling images, all stopped containers, unused volumes/networks.
  const s = new Set(checked.value);
  for (const i of removableImages.value) if (i.dangling) s.add(i.id);
  for (const c of stoppedContainers.value) s.add(c.id);
  checked.value = s;
}

async function refresh() {
  loading.value = true;
  error.value = "";
  try {
    overview.value = await api.dockerOverview();
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}

async function togglePin(id: string, pinned: boolean) {
  await api.dockerSetPin(id, pinned);
  await refresh();
}

async function doDelete() {
  showConfirm.value = false;
  showVolumeWarn.value = false;
  deleting.value = true;
  outcomes.value = [];
  try {
    outcomes.value = await api.dockerDelete(
      { kind: currentKind(), ids: [...checked.value], dryRun: dryRun.value },
      (p) => (progress.value = p),
    );
    if (!dryRun.value) {
      checked.value = new Set();
      await refresh();
    }
  } catch (e) {
    error.value = String(e);
  } finally {
    deleting.value = false;
  }
}

function requestDelete() {
  if (tab.value === "volumes" && !dryRun.value) {
    showVolumeWarn.value = true;
  } else {
    showConfirm.value = true;
  }
}

async function pruneBuildCache() {
  deleting.value = true;
  outcomes.value = [];
  try {
    outcomes.value = await api.dockerDelete(
      { kind: "build_cache", ids: ["all"], dryRun: dryRun.value },
      (p) => (progress.value = p),
    );
    if (!dryRun.value) await refresh();
  } catch (e) {
    error.value = String(e);
  } finally {
    deleting.value = false;
  }
}

onMounted(refresh);
</script>

<template>
  <div>
    <div class="row-between">
      <h1 class="page-title" style="margin: 0">{{ t("docker.title") }}</h1>
      <div class="row">
        <label class="checkbox muted">
          <input type="checkbox" v-model="dryRun" /> {{ t("docker.dryRun") }}
        </label>
        <button class="btn" :disabled="loading || deleting" @click="refresh">
          <font-awesome-icon icon="fa-solid fa-arrows-rotate" aria-hidden="true" />
          {{ t("docker.refresh") }}
        </button>
      </div>
    </div>

    <div v-if="loading" class="card"><span class="spinner"></span></div>

    <div v-else-if="overview && !overview.status.available" class="card">
      <strong>
        <font-awesome-icon icon="fa-solid fa-triangle-exclamation" aria-hidden="true" />
        {{ t("docker.notAvailable") }}
      </strong>
      <p class="muted">{{ t("docker.notAvailableHint") }}</p>
      <p v-if="overview.status.error" class="muted mono" style="font-size: 12px">{{ overview.status.error }}</p>
    </div>

    <template v-else-if="overview">
      <div class="card">
        <div class="row" style="flex-wrap: wrap; gap: 18px">
          <div>
            <div class="muted" style="font-size: 12px">Docker</div>
            <strong>{{ overview.status.version }}</strong>
          </div>
          <div>
            <div class="muted" style="font-size: 12px">{{ t("docker.reclaimable") }}</div>
            <strong class="text-ok">{{ formatBytes(overview.reclaimableBytes) }}</strong>
          </div>
          <div>
            <div class="muted" style="font-size: 12px">{{ t("docker.buildCache") }}</div>
            <strong>{{ formatBytes(overview.buildCacheBytes) }}</strong>
            <button class="btn btn-sm" style="margin-left: 8px" :disabled="deleting || !overview.buildCacheBytes" @click="pruneBuildCache">
              <font-awesome-icon icon="fa-solid fa-broom" aria-hidden="true" />
              {{ t("docker.pruneBuildCache") }}
            </button>
          </div>
        </div>
      </div>

      <div class="tabs mt">
        <button class="tab" :class="{ active: tab === 'images' }" @click="tab = 'images'">
          {{ t("docker.images") }} ({{ removableImages.length }})
        </button>
        <button class="tab" :class="{ active: tab === 'containers' }" @click="tab = 'containers'">
          {{ t("docker.containers") }} ({{ stoppedContainers.length }})
        </button>
        <button class="tab" :class="{ active: tab === 'volumes' }" @click="tab = 'volumes'">
          {{ t("docker.volumes") }} ({{ unusedVolumes.length }})
        </button>
        <button class="tab" :class="{ active: tab === 'networks' }" @click="tab = 'networks'">
          {{ t("docker.networks") }} ({{ unusedNetworks.length }})
        </button>
      </div>

      <div class="row-between mb">
        <div class="row">
          <button class="btn btn-sm" @click="selectSuggested">
            <font-awesome-icon icon="fa-solid fa-check-double" aria-hidden="true" />
            {{ t("dup.select") }}
          </button>
          <label class="checkbox muted" style="font-size: 12.5px">
            <input type="checkbox" v-model="sortBySize" /> {{ t("common.size") }} ↓
          </label>
        </div>
        <button
          class="btn btn-danger"
          :disabled="!checked.size || deleting"
          @click="requestDelete"
        >
          <font-awesome-icon icon="fa-solid fa-trash-can" aria-hidden="true" />
          {{ t("docker.delete") }} ({{ checked.size }})
        </button>
      </div>

      <div v-if="deleting" class="card">
        <ProgressBar :percent="progress?.percent ?? null" />
      </div>

      <div v-if="outcomes.length" class="card">
        <div v-for="o in outcomes" :key="o.path" class="row mono" style="font-size: 12.5px; padding: 2px 0">
          <span :class="o.ok ? 'text-ok' : 'text-danger'">
            <font-awesome-icon
              :icon="o.ok ? 'fa-solid fa-check' : 'fa-solid fa-xmark'"
              :aria-label="o.ok ? 'Success' : 'Failed'"
            />
          </span>
          <span class="ellipsis" style="flex: 1" :title="o.path">{{ shortId(o.path, 24) }}</span>
          <span class="muted">{{ o.error }}</span>
        </div>
      </div>

      <p v-if="error" class="text-danger">{{ error }}</p>

      <!-- Images -->
      <div v-if="tab === 'images'" class="card">
        <table class="tbl">
          <thead>
            <tr>
              <th style="width: 30px"></th>
              <th>{{ t("common.tags") }}</th>
              <th>{{ t("common.size") }}</th>
              <th>{{ t("common.created") }}</th>
              <th>{{ t("common.status") }}</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="i in sorted.images" :key="i.id">
              <td>
                <input type="checkbox" :checked="checked.has(i.id)" :disabled="i.inUseRunning || i.pinned"
                  @change="(e) => toggle(i.id, (e.target as HTMLInputElement).checked)" />
              </td>
              <td>
                <div v-for="tag in i.tags.slice(0, 3)" :key="tag" class="mono" style="font-size: 12px">{{ tag }}</div>
                <span class="muted" style="font-size: 11px">{{ shortId(i.id) }}</span>
              </td>
              <td class="nowrap">{{ formatBytes(i.sizeBytes) }}</td>
              <td class="muted nowrap">{{ formatDate(i.created, true) }}</td>
              <td>
                <span v-if="i.dangling" class="badge badge-muted">{{ t("docker.dangling") }}</span>
                <span v-if="i.usedBy.length" class="badge badge-info" :title="i.usedBy.join(', ')">
                  {{ t("docker.usedBy") }} {{ i.usedBy.length }}
                </span>
                <span v-if="i.inUseRunning" class="badge badge-caution">{{ t("docker.running") }}</span>
                <span v-if="i.pinned" class="badge badge-info" :title="t('docker.pin')">
                  <font-awesome-icon icon="fa-solid fa-thumbtack" aria-hidden="true" />
                </span>
              </td>
              <td>
                <button class="btn btn-sm" @click="togglePin(i.id, !i.pinned)">
                  {{ i.pinned ? t("docker.unpin") : t("docker.pin") }}
                </button>
              </td>
            </tr>
          </tbody>
        </table>
        <div v-if="!sorted.images.length" class="empty-state">{{ t("common.none") }}</div>
      </div>

      <!-- Containers -->
      <div v-if="tab === 'containers'" class="card">
        <table class="tbl">
          <thead>
            <tr>
              <th style="width: 30px"></th>
              <th>{{ t("common.name") }}</th>
              <th>Image</th>
              <th>{{ t("common.state") }}</th>
              <th>{{ t("common.size") }}</th>
              <th>{{ t("common.created") }}</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="c in sorted.containers" :key="c.id">
              <td>
                <input type="checkbox" :checked="checked.has(c.id)"
                  @change="(e) => toggle(c.id, (e.target as HTMLInputElement).checked)" />
              </td>
              <td class="mono" style="font-size: 12.5px">{{ c.name }}</td>
              <td class="mono muted" style="font-size: 12px">{{ c.image }}</td>
              <td><span class="badge badge-muted">{{ c.state }}</span></td>
              <td class="nowrap">{{ formatBytes(c.sizeBytes) }}</td>
              <td class="muted nowrap">{{ formatDate(c.created, true) }}</td>
            </tr>
          </tbody>
        </table>
        <div v-if="!sorted.containers.length" class="empty-state">{{ t("common.none") }}</div>
      </div>

      <!-- Volumes -->
      <div v-if="tab === 'volumes'" class="card">
        <p class="badge badge-caution mb">{{ t("docker.volumeWarn") }}</p>
        <table class="tbl">
          <thead>
            <tr>
              <th style="width: 30px"></th>
              <th>{{ t("common.name") }}</th>
              <th>{{ t("common.driver") }}</th>
              <th>{{ t("common.size") }}</th>
              <th>{{ t("common.created") }}</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="v in sorted.volumes" :key="v.name">
              <td>
                <input type="checkbox" :checked="checked.has(v.name)" :disabled="v.inUse || v.pinned"
                  @change="(e) => toggle(v.name, (e.target as HTMLInputElement).checked)" />
              </td>
              <td class="mono ellipsis" style="font-size: 12px" :title="v.name">{{ v.name }}</td>
              <td class="muted">{{ v.driver }}</td>
              <td class="nowrap">{{ v.sizeBytes != null ? formatBytes(v.sizeBytes) : "—" }}</td>
              <td class="muted nowrap">{{ v.createdAt ? formatDate(Date.parse(v.createdAt)) : "—" }}</td>
              <td>
                <button class="btn btn-sm" @click="togglePin(v.name, !v.pinned)">
                  {{ v.pinned ? t("docker.unpin") : t("docker.pin") }}
                </button>
              </td>
            </tr>
          </tbody>
        </table>
        <div v-if="!sorted.volumes.length" class="empty-state">{{ t("common.none") }}</div>
      </div>

      <!-- Networks -->
      <div v-if="tab === 'networks'" class="card">
        <table class="tbl">
          <thead>
            <tr>
              <th style="width: 30px"></th>
              <th>{{ t("common.name") }}</th>
              <th>{{ t("common.driver") }}</th>
              <th>{{ t("common.status") }}</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="n in sorted.networks" :key="n.id">
              <td>
                <input type="checkbox" :checked="checked.has(n.id)" :disabled="n.builtin || n.inUse"
                  @change="(e) => toggle(n.id, (e.target as HTMLInputElement).checked)" />
              </td>
              <td class="mono" style="font-size: 12.5px">{{ n.name }}</td>
              <td class="muted">{{ n.driver }}</td>
              <td>
                <span v-if="n.builtin" class="badge badge-muted">{{ t("docker.builtin") }}</span>
                <span v-else-if="n.inUse" class="badge badge-info">{{ t("docker.inUse") }}</span>
              </td>
            </tr>
          </tbody>
        </table>
        <div v-if="!sorted.networks.length" class="empty-state">{{ t("common.none") }}</div>
      </div>
    </template>

    <!-- Volume warning (hard confirm) -->
    <ConfirmDialog
      v-if="showVolumeWarn"
      :title="t('docker.confirmTitle')"
      danger
      @cancel="showVolumeWarn = false"
      @confirm="showConfirm = true; showVolumeWarn = false"
    >
      <p class="text-danger"><strong>{{ t("docker.volumeWarn") }}</strong></p>
      <ul class="mono" style="font-size: 12px; max-height: 200px; overflow-y: auto">
        <li v-for="id in [...checked]" :key="id">{{ id }}</li>
      </ul>
    </ConfirmDialog>

    <ConfirmDialog
      v-if="showConfirm"
      :title="t('docker.confirmTitle')"
      danger
      :busy="deleting"
      @cancel="showConfirm = false"
      @confirm="doDelete"
    >
      <p>{{ t("docker.confirmBody") }}</p>
      <ul class="mono" style="font-size: 12px; max-height: 200px; overflow-y: auto">
        <li v-for="id in [...checked]" :key="id">{{ shortId(id, 32) }}</li>
      </ul>
      <p v-if="dryRun" class="badge badge-info">{{ t("docker.dryRun") }}</p>
    </ConfirmDialog>
  </div>
</template>
