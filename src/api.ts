/**
 * Thin typed wrapper over Tauri commands. All deletion flows through
 * backend-validated commands; progress arrives via `Channel` callbacks.
 */
import { Channel, invoke } from "@tauri-apps/api/core";
import type {
  CategoryInfo,
  DedupeOptions,
  DeleteMode,
  DeleteResult,
  DeleteTarget,
  DockerDeleteRequest,
  DockerOverview,
  DockerStatus,
  DriveInfo,
  DuplicateDeleteRequest,
  DuplicateScanResult,
  HistoryEntry,
  JunkScanResult,
  ScanProgress,
  Settings,
} from "./types";

export type ProgressHandler = (p: ScanProgress) => void;

function channel(onProgress?: ProgressHandler): Channel<ScanProgress> {
  const c = new Channel<ScanProgress>();
  c.onmessage = (m) => onProgress?.(m);
  return c;
}

function jobId(): string {
  return crypto.randomUUID();
}

export const api = {
  // Junk scanner
  listJunkCategories: () => invoke<CategoryInfo[]>("list_junk_categories"),
  scanJunk: (categories: string[] | null, onProgress?: ProgressHandler) =>
    invoke<JunkScanResult>("scan_junk", {
      jobId: jobId(),
      categories,
      channel: channel(onProgress),
    }),
  deleteItems: (
    targets: DeleteTarget[],
    mode: DeleteMode,
    dryRun: boolean,
    onProgress?: ProgressHandler,
  ) =>
    invoke<DeleteResult>("delete_items", {
      jobId: jobId(),
      targets,
      mode,
      dryRun,
      channel: channel(onProgress),
    }),

  // Docker
  dockerStatus: () => invoke<DockerStatus>("docker_status"),
  dockerOverview: () => invoke<DockerOverview>("docker_overview"),
  dockerDelete: (req: DockerDeleteRequest, onProgress?: ProgressHandler) =>
    invoke<import("./types").DeleteOutcome[]>("docker_delete", {
      jobId: jobId(),
      req,
      channel: channel(onProgress),
    }),
  dockerSetPin: (id: string, pinned: boolean) =>
    invoke<string[]>("docker_set_pin", { id, pinned }),
  dockerListPins: () => invoke<string[]>("docker_list_pins"),

  // Duplicates
  scanDuplicates: (opts: DedupeOptions, onProgress?: ProgressHandler) =>
    invoke<DuplicateScanResult>("scan_duplicates", {
      jobId: jobId(),
      opts,
      channel: channel(onProgress),
    }),
  deleteDuplicates: (req: DuplicateDeleteRequest, onProgress?: ProgressHandler) =>
    invoke<DeleteResult>("delete_duplicates", {
      jobId: jobId(),
      req,
      channel: channel(onProgress),
    }),

  // Misc
  cancelJob: (jobId: string) => invoke<boolean>("cancel_job", { jobId }),
  listDrives: () => invoke<DriveInfo[]>("list_drives"),
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<void>("save_settings", { settings }),
  listHistory: () => invoke<HistoryEntry[]>("list_history"),
  clearHistory: () => invoke<void>("clear_history"),
  revealPath: (path: string) => invoke<void>("reveal_path", { path }),
};
