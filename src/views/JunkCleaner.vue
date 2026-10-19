<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { api } from "../api";
import { formatBytes, formatDuration } from "../format";
import { t } from "../i18n";
import ConfirmDialog from "../components/ConfirmDialog.vue";
import ProgressBar from "../components/ProgressBar.vue";
import { useSettingsStore } from "../stores/settings";
import type {
  CategoryInfo,
  CategoryResult,
  DeleteResult,
  JunkScanResult,
  ScanProgress,
} from "../types";

const settings = useSettingsStore();

const categories = ref<CategoryInfo[]>([]);
const enabled = ref<Set<string>>(new Set());
const scanning = ref(false);
const deleting = ref(false);
const progress = ref<ScanProgress | null>(null);
const result = ref<JunkScanResult | null>(null);
const deleteResult = ref<DeleteResult | null>(null);
const showConfirm = ref(false);
const dryRun = ref(false);
const error = ref("");

/** Selected junk items: set of item ids → checked. */
const checked = ref<Set<string>>(new Set());
const expanded = ref<Set<string>>(new Set());

const allItems = computed(() =>
  (result.value?.categories ?? []).flatMap((c) =>
    c.items.map((i) => ({ cat: c, item: i })),
  ),
);
const selectedTargets = computed(() =>
  allItems.value
    .filter((x) => checked.value.has(x.item.id))
    .map((x) => ({ path: x.item.path, sizeBytes: x.item.sizeBytes })),
);
const selectedBytes = computed(() =>
  selectedTargets.value.reduce((s, x) => s + x.sizeBytes, 0),
);

function catChecked(c: CategoryResult): boolean {
  return c.items.length > 0 && c.items.every((i) => checked.value.has(i.id));
}
function toggleCat(c: CategoryResult, on: boolean) {
  const s = new Set(checked.value);
  for (const i of c.items) on ? s.add(i.id) : s.delete(i.id);
  checked.value = s;
}
function toggleItem(id: string, on: boolean) {
  const s = new Set(checked.value);
  on ? s.add(id) : s.delete(id);
  checked.value = s;
}
function toggleExpand(id: string) {
  const s = new Set(expanded.value);
  s.has(id) ? s.delete(id) : s.add(id);
  expanded.value = s;
}

async function scan() {
  scanning.value = true;
  error.value = "";
  result.value = null;
  deleteResult.value = null;
  checked.value = new Set();
  try {
    const cats = categories.value
      .filter((c) => enabled.value.has(c.id))
      .map((c) => c.id);
    result.value = await api.scanJunk(cats.length ? cats : null, (p) => (progress.value = p));
    // Default: check all *safe* items; caution items left unchecked.
    const sel = new Set<string>();
    for (const c of result.value.categories)
      if (c.risk === "safe") for (const i of c.items) sel.add(i.id);
    checked.value = sel;
  } catch (e) {
    error.value = String(e);
  } finally {
    scanning.value = false;
  }
}

async function doDelete() {
  showConfirm.value = false;
  deleting.value = true;
  deleteResult.value = null;
  try {
    const mode = settings.settings.defaultDeleteMode === "permanent" ? "permanent" : "trash";
    deleteResult.value = await api.deleteItems(
      selectedTargets.value,
      mode,
      dryRun.value,
      (p) => (progress.value = p),
    );
    // Drop successfully removed items from the result view.
    if (result.value && !dryRun.value) {
      const okPaths = new Set(
        deleteResult.value.outcomes.filter((o) => o.ok).map((o) => o.path),
      );
      for (const c of result.value.categories) {
        c.items = c.items.filter((i) => !okPaths.has(i.path));
        c.totalBytes = c.items.reduce((s, i) => s + i.sizeBytes, 0);
      }
      checked.value = new Set();
      result.value.totalBytes = result.value.categories.reduce(
        (s, c) => s + c.totalBytes,
        0,
      );
    }
  } catch (e) {
    error.value = String(e);
  } finally {
    deleting.value = false;
  }
}

onMounted(async () => {
  categories.value = await api.listJunkCategories();
  enabled.value = new Set(categories.value.map((c) => c.id));
});
</script>

<template>
  <div>
    <div class="row-between">
      <h1 class="page-title" style="margin: 0">{{ t("junk.title") }}</h1>
      <div class="row">
        <label class="checkbox muted">
          <input type="checkbox" v-model="dryRun" /> {{ t("junk.dryRun") }}
        </label>
        <button class="btn btn-primary" :disabled="scanning || deleting" @click="scan">
          <span v-if="scanning" class="spinner"></span>
          <font-awesome-icon
            v-else
            icon="fa-solid fa-magnifying-glass"
            aria-hidden="true"
          />
          {{ result ? t("junk.rescan") : t("junk.scan") }}
        </button>
      </div>
    </div>

    <p v-if="!result && !scanning" class="muted">{{ t("junk.scanHint") }}</p>

    <!-- Category picker before scan -->
    <div v-if="!result" class="grid grid-2 mt">
      <label v-for="c in categories" :key="c.id" class="card checkbox" style="cursor: pointer">
        <input
          type="checkbox"
          :checked="enabled.has(c.id)"
          @change="(e) => (e.target as HTMLInputElement).checked ? enabled.add(c.id) : enabled.delete(c.id)"
        />
        <div style="flex: 1">
          <div class="row-between">
            <strong>{{ c.label }}</strong>
            <span class="badge" :class="c.risk === 'safe' ? 'badge-safe' : 'badge-caution'">
              {{ c.risk === "safe" ? t("junk.riskSafe") : t("junk.riskCaution") }}
            </span>
          </div>
          <div class="muted" style="font-size: 12.5px">{{ c.description }}</div>
        </div>
      </label>
    </div>

    <!-- progress -->
    <div v-if="scanning || deleting" class="card mt">
      <div class="row muted">
        <span class="spinner"></span>
        <span class="ellipsis">{{ progress?.current || "…" }}</span>
        <span class="nowrap">{{ formatBytes(progress?.bytesFound ?? 0) }}</span>
      </div>
      <ProgressBar class="mt" :percent="progress?.percent ?? null" />
    </div>

    <p v-if="error" class="text-danger mt">{{ error }}</p>

    <!-- delete summary -->
    <div v-if="deleteResult" class="card mt">
      <strong>{{ t("junk.done") }}:</strong>
      {{ t("junk.freed") }} = <strong>{{ formatBytes(deleteResult.freedBytes) }}</strong>
      <span v-if="deleteResult.outcomes.some((o) => !o.ok)" class="text-danger">
        · {{ deleteResult.outcomes.filter((o) => !o.ok).length }} {{ t("junk.failed") }}
      </span>
      <ul v-if="deleteResult.outcomes.some((o) => !o.ok)" class="muted" style="margin: 8px 0 0">
        <li v-for="o in deleteResult.outcomes.filter((o) => !o.ok)" :key="o.path" class="mono">
          {{ o.path }} — {{ o.error }}
        </li>
      </ul>
    </div>

    <!-- results -->
    <template v-if="result">
      <div class="card mt row-between" style="position: sticky; top: 0; z-index: 5">
        <div>
          <strong>{{ formatBytes(result.totalBytes) }}</strong>
          <span class="muted">
            · {{ selectedTargets.length }} {{ t("junk.selected") }} ({{ formatBytes(selectedBytes) }})
            · {{ formatDuration(result.durationMs) }}
          </span>
        </div>
        <button
          class="btn btn-danger"
          :disabled="!selectedTargets.length || deleting"
          @click="showConfirm = true"
        >
          <font-awesome-icon icon="fa-solid fa-trash-can" aria-hidden="true" />
          {{ t("junk.clean") }} ({{ formatBytes(selectedBytes) }})
        </button>
      </div>

      <div v-for="c in result.categories" :key="c.id" class="card mt">
        <div class="row" style="cursor: pointer" @click="toggleExpand(c.id)">
          <input
            type="checkbox"
            :checked="catChecked(c)"
            :disabled="!c.items.length"
            @click.stop
            @change="(e) => toggleCat(c, (e.target as HTMLInputElement).checked)"
          />
          <div style="flex: 1">
            <div class="row">
              <strong>{{ c.label }}</strong>
              <span class="badge" :class="c.risk === 'safe' ? 'badge-safe' : 'badge-caution'">
                {{ c.risk === "safe" ? t("junk.riskSafe") : t("junk.riskCaution") }}
              </span>
            </div>
            <div class="muted" style="font-size: 12.5px">
              {{ c.items.length }} {{ t("junk.items") }} · {{ formatBytes(c.totalBytes) }}
            </div>
          </div>
          <font-awesome-icon
            class="muted"
            :icon="expanded.has(c.id) ? 'fa-solid fa-chevron-down' : 'fa-solid fa-chevron-right'"
            aria-hidden="true"
          />
        </div>

        <div v-if="expanded.has(c.id)" class="mt">
          <div v-if="!c.items.length" class="muted">{{ t("junk.empty") }}</div>
          <label
            v-for="i in c.items"
            :key="i.id"
            class="row mono muted"
            style="padding: 3px 0; font-size: 12.5px"
          >
            <input
              type="checkbox"
              :checked="checked.has(i.id)"
              @change="(e) => toggleItem(i.id, (e.target as HTMLInputElement).checked)"
            />
            <span class="ellipsis" style="flex: 1" :title="i.path">{{ i.path }}</span>
            <span class="nowrap">{{ formatBytes(i.sizeBytes) }}</span>
          </label>
        </div>
      </div>
    </template>

    <ConfirmDialog
      v-if="showConfirm"
      :title="t('junk.confirmTitle')"
      :danger="settings.settings.defaultDeleteMode === 'permanent' && !dryRun"
      :busy="deleting"
      @cancel="showConfirm = false"
      @confirm="doDelete"
    >
      <p>
        {{
          t("junk.confirmBody", {
            mode: dryRun
              ? "(dry-run)"
              : t(
                  settings.settings.defaultDeleteMode === "permanent"
                    ? "junk.modePermanent"
                    : "junk.modeTrash",
                ),
          })
        }}
      </p>
      <p>
        <strong>{{ selectedTargets.length }}</strong> {{ t("junk.items") }} ·
        <strong>{{ formatBytes(selectedBytes) }}</strong>
      </p>
    </ConfirmDialog>
  </div>
</template>
