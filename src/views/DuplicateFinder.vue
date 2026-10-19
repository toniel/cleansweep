<script setup lang="ts">
import { computed, ref } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { api } from "../api";
import { formatBytes, formatDate, formatDuration } from "../format";
import { t } from "../i18n";
import ConfirmDialog from "../components/ConfirmDialog.vue";
import ProgressBar from "../components/ProgressBar.vue";
import { useSettingsStore } from "../stores/settings";
import type {
  DeleteResult,
  DuplicateGroup,
  DuplicateScanResult,
  ScanProgress,
} from "../types";

const settings = useSettingsStore();

const roots = ref<string[]>([]);
const excludes = ref("");
const minSizeMb = ref(1);
const scanning = ref(false);
const deleting = ref(false);
const progress = ref<ScanProgress | null>(null);
const result = ref<DuplicateScanResult | null>(null);
const deleteResult = ref<DeleteResult | null>(null);
const showConfirm = ref(false);
const dryRun = ref(false);
const error = ref("");
const selectRule = ref<"oldest" | "newest" | "shortest">("oldest");
const expanded = ref<Set<string>>(new Set());

/**
 * Checked = files marked for removal within each group.
 * `keepMap`: groupId → path to preserve. `removeSet`: path → marked.
 */
const keepMap = ref<Map<string, string>>(new Map());
const removeSet = ref<Set<string>>(new Set());

const markedCount = computed(() => removeSet.value.size);
const markedBytes = computed(() => {
  let s = 0;
  for (const g of result.value?.groups ?? [])
    for (const f of g.files) if (removeSet.value.has(f.path)) s += f.sizeBytes;
  return s;
});

async function addFolder() {
  const picked = await open({ directory: true, multiple: true });
  if (!picked) return;
  const list = Array.isArray(picked) ? picked : [picked];
  roots.value = [...new Set([...roots.value, ...list])];
}
function removeRoot(r: string) {
  roots.value = roots.value.filter((x) => x !== r);
}

function autoSelect() {
  const keep = new Map<string, string>();
  const rem = new Set<string>();
  for (const g of result.value?.groups ?? []) {
    const files = [...g.files];
    let keepFile = files[0];
    if (selectRule.value === "oldest")
      keepFile = files.reduce((a, b) => (a.modifiedMs <= b.modifiedMs ? a : b));
    else if (selectRule.value === "newest")
      keepFile = files.reduce((a, b) => (a.modifiedMs >= b.modifiedMs ? a : b));
    else keepFile = files.reduce((a, b) => (a.path.length <= b.path.length ? a : b));
    keep.set(g.id, keepFile.path);
    for (const f of files) if (f.path !== keepFile.path) rem.add(f.path);
  }
  keepMap.value = keep;
  removeSet.value = rem;
}

function toggleFile(group: DuplicateGroup, path: string, on: boolean) {
  const rem = new Set(removeSet.value);
  const keep = new Map(keepMap.value);
  if (on) rem.add(path);
  else rem.delete(path);
  // Never mark every copy — keep at least one file unmarked per group.
  const markedInGroup = group.files.filter((f) => rem.has(f.path));
  if (markedInGroup.length >= group.files.length) rem.delete(group.files[0].path);
  // Choose/refresh the kept file if the current keep was marked.
  if (!keep.has(group.id) || rem.has(keep.get(group.id)!)) {
    const unmarked = group.files.find((f) => !rem.has(f.path));
    keep.set(group.id, unmarked?.path ?? group.files[0].path);
  }
  removeSet.value = rem;
  keepMap.value = keep;
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
  removeSet.value = new Set();
  keepMap.value = new Map();
  try {
    result.value = await api.scanDuplicates(
      {
        roots: roots.value,
        excludes: [
          ...settings.settings.excludes,
          ...excludes.value.split(",").map((s) => s.trim()).filter(Boolean),
        ],
        minSizeBytes: Math.max(1, minSizeMb.value) * 1024 * 1024,
      },
      (p) => (progress.value = p),
    );
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
    const groups = (result.value?.groups ?? [])
      .map((g) => ({
        keep: [keepMap.value.get(g.id) ?? g.files[0].path],
        remove: g.files
          .filter((f) => removeSet.value.has(f.path))
          .map((f) => ({ path: f.path, sizeBytes: f.sizeBytes })),
      }))
      .filter((g) => g.remove.length > 0);
    const mode = settings.settings.defaultDeleteMode === "permanent" ? "permanent" : "trash";
    deleteResult.value = await api.deleteDuplicates(
      { groups, mode, dryRun: dryRun.value },
      (p) => (progress.value = p),
    );
    if (!dryRun.value) await scan();
  } catch (e) {
    error.value = String(e);
  } finally {
    deleting.value = false;
  }
}

function exportCsv() {
  if (!result.value) return;
  const rows = [["group", "hash", "path", "size_bytes", "modified_ms"].join(",")];
  for (const g of result.value.groups)
    for (const f of g.files)
      rows.push([g.id.slice(0, 8), g.hash, `"${f.path.replaceAll('"', '""')}"`, f.sizeBytes, f.modifiedMs].join(","));
  const blob = new Blob([rows.join("\n")], { type: "text/csv" });
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = `cleansweep-duplicates-${Date.now()}.csv`;
  a.click();
  URL.revokeObjectURL(a.href);
}

async function reveal(path: string) {
  try {
    await revealItemInDir(path);
  } catch {
    await api.revealPath(path).catch(() => {});
  }
}
</script>

<template>
  <div>
    <h1 class="page-title">{{ t("dup.title") }}</h1>

    <div class="card stack">
      <div class="row-between">
        <strong>{{ t("dup.roots") }}</strong>
        <button class="btn btn-sm" @click="addFolder">
          <font-awesome-icon icon="fa-solid fa-folder-plus" aria-hidden="true" />
          {{ t("dup.addFolder") }}
        </button>
      </div>
      <div v-if="!roots.length" class="muted">{{ t("dup.noRoots") }}</div>
      <div v-for="r in roots" :key="r" class="row mono" style="font-size: 12.5px">
        <span class="ellipsis" style="flex: 1">{{ r }}</span>
        <button class="btn btn-sm" :aria-label="`Remove ${r}`" @click="removeRoot(r)">
          <font-awesome-icon icon="fa-solid fa-xmark" aria-hidden="true" />
        </button>
      </div>
      <div class="grid grid-2">
        <div class="field">
          <label>{{ t("dup.excludes") }}</label>
          <input class="input" v-model="excludes" placeholder="*.tmp, cache" />
        </div>
        <div class="field">
          <label>{{ t("dup.minSize") }} (MB)</label>
          <input class="input" type="number" min="0" v-model.number="minSizeMb" />
        </div>
      </div>
      <div class="row">
        <button class="btn btn-primary" :disabled="scanning || !roots.length" @click="scan">
          <span v-if="scanning" class="spinner"></span>
          <font-awesome-icon
            v-else
            icon="fa-solid fa-magnifying-glass"
            aria-hidden="true"
          />
          {{ result ? t("dup.rescan") : t("dup.scan") }}
        </button>
        <label class="checkbox muted">
          <input type="checkbox" v-model="dryRun" /> {{ t("dup.dryRun") }}
        </label>
      </div>
    </div>

    <div v-if="scanning || deleting" class="card mt">
      <div class="row muted">
        <span class="spinner"></span>
        <span class="ellipsis">{{ progress?.current || "…" }}</span>
        <span class="nowrap">{{ progress?.entriesSeen ?? 0 }} {{ t("dup.scanned") }}</span>
      </div>
      <ProgressBar class="mt" :percent="progress?.percent ?? null" />
    </div>

    <p v-if="error" class="text-danger mt">{{ error }}</p>

    <div v-if="deleteResult" class="card mt">
      <strong>{{ t("junk.done") }}:</strong>
      {{ t("junk.freed") }} = <strong>{{ formatBytes(deleteResult.freedBytes) }}</strong>
      <span v-if="deleteResult.outcomes.some((o) => !o.ok)" class="text-danger">
        · {{ deleteResult.outcomes.filter((o) => !o.ok).length }} {{ t("junk.failed") }}
      </span>
    </div>

    <template v-if="result">
      <div class="card mt row-between" style="position: sticky; top: 0; z-index: 5">
        <div>
          <strong>{{ result.groups.length }}</strong> {{ t("dup.groups") }} ·
          {{ t("dup.wasted") }}: <strong>{{ formatBytes(result.wastedBytes) }}</strong>
          <span class="muted"> · {{ result.scannedFiles }} {{ t("dup.scanned") }} · {{ formatDuration(result.durationMs) }}</span>
        </div>
        <div class="row">
          <select class="input" style="width: auto" v-model="selectRule">
            <option value="oldest">{{ t("dup.selectOldest") }}</option>
            <option value="newest">{{ t("dup.selectNewest") }}</option>
            <option value="shortest">{{ t("dup.selectShortest") }}</option>
          </select>
           <button class="btn btn-sm" @click="autoSelect">
             <font-awesome-icon icon="fa-solid fa-check-double" aria-hidden="true" />
             {{ t("dup.select") }}
           </button>
           <button class="btn btn-sm" @click="exportCsv">
             <font-awesome-icon icon="fa-solid fa-file-csv" aria-hidden="true" />
             {{ t("dup.export") }}
           </button>
          <button
            class="btn btn-danger"
            :disabled="!markedCount || deleting"
            @click="showConfirm = true"
          >
            <font-awesome-icon icon="fa-solid fa-trash-can" aria-hidden="true" />
            {{ t("dup.delete") }} ({{ markedCount }} · {{ formatBytes(markedBytes) }})
          </button>
        </div>
      </div>

      <div v-for="g in result.groups" :key="g.id" class="group-card mt">
        <div class="row" style="cursor: pointer" @click="toggleExpand(g.id)">
          <div style="flex: 1">
            <strong>{{ formatBytes(g.sizeBytes) }}</strong>
            <span class="muted"> × {{ g.files.length }}</span>
            <span class="muted mono" style="font-size: 11px"> · {{ g.hash }}</span>
          </div>
          <span class="badge badge-caution">−{{ formatBytes(g.wastedBytes) }}</span>
           <font-awesome-icon
             class="muted"
             :icon="expanded.has(g.id) ? 'fa-solid fa-chevron-down' : 'fa-solid fa-chevron-right'"
             aria-hidden="true"
           />
        </div>
        <div v-if="expanded.has(g.id)" class="mt">
          <div v-for="f in g.files" :key="f.path" class="row mono" style="padding: 3px 0; font-size: 12.5px">
            <input
              type="checkbox"
              :checked="removeSet.has(f.path)"
              @change="(e) => toggleFile(g, f.path, (e.target as HTMLInputElement).checked)"
            />
            <span v-if="keepMap.get(g.id) === f.path" class="badge badge-safe">{{ t("dup.keepBadge") }}</span>
            <span class="ellipsis" style="flex: 1" :title="f.path">{{ f.path }}</span>
            <span class="muted nowrap">{{ formatDate(f.modifiedMs) }}</span>
             <button class="btn btn-sm" @click.stop="reveal(f.path)">
               <font-awesome-icon icon="fa-solid fa-folder-open" aria-hidden="true" />
               {{ t("dup.reveal") }}
             </button>
          </div>
        </div>
      </div>
    </template>

    <ConfirmDialog
      v-if="showConfirm"
      :title="t('dup.confirmTitle')"
      :danger="settings.settings.defaultDeleteMode === 'permanent' && !dryRun"
      :busy="deleting"
      @cancel="showConfirm = false"
      @confirm="doDelete"
    >
      <p>
        {{
          t("dup.confirmBody", {
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
        <strong>{{ markedCount }}</strong> {{ t("junk.items") }} ·
        <strong>{{ formatBytes(markedBytes) }}</strong>
      </p>
    </ConfirmDialog>
  </div>
</template>
