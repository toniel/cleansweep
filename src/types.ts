/** Types mirroring the Rust backend payloads (serde camelCase). */

export type RiskLevel = "safe" | "caution";
export type JobPhase = "scanning" | "deleting" | "done" | "cancelled" | "error";
export type DeleteMode = "trash" | "permanent";

export interface ScanProgress {
  phase: JobPhase;
  current: string;
  entriesSeen: number;
  bytesFound: number;
  percent: number | null;
}

export interface JunkItem {
  id: string;
  path: string;
  sizeBytes: number;
  fileCount: number | null;
}

export interface CategoryInfo {
  id: string;
  label: string;
  labelKey: string | null;
  description: string;
  descKey: string | null;
  risk: RiskLevel;
}

export interface CategoryResult {
  id: string;
  label: string;
  description: string;
  risk: RiskLevel;
  totalBytes: number;
  items: JunkItem[];
}

export interface JunkScanResult {
  categories: CategoryResult[];
  totalBytes: number;
  durationMs: number;
  cancelled: boolean;
}

export interface DeleteTarget {
  path: string;
  sizeBytes: number;
}

export interface DeleteOutcome {
  path: string;
  ok: boolean;
  dryRun: boolean;
  freedBytes: number;
  error: string | null;
}

export interface DeleteResult {
  outcomes: DeleteOutcome[];
  freedBytes: number;
  cancelled: boolean;
}

export interface DriveInfo {
  name: string;
  mountPoint: string;
  kind: string;
  fsType: string;
  totalBytes: number;
  usedBytes: number;
  freeBytes: number;
}

export interface HistoryEntry {
  id: string;
  at: string;
  category: string;
  freedBytes: number;
  itemCount: number;
  mode: string;
}

export interface DockerStatus {
  available: boolean;
  version: string | null;
  apiVersion: string | null;
  error: string | null;
}

export interface DockerImageInfo {
  id: string;
  tags: string[];
  sizeBytes: number;
  created: number;
  dangling: boolean;
  usedBy: string[];
  inUseRunning: boolean;
  pinned: boolean;
}

export interface DockerContainerInfo {
  id: string;
  name: string;
  image: string;
  imageId: string;
  state: string;
  status: string;
  created: number;
  sizeBytes: number;
}

export interface DockerVolumeInfo {
  name: string;
  driver: string;
  inUse: boolean;
  sizeBytes: number | null;
  createdAt: string | null;
  pinned: boolean;
}

export interface DockerNetworkInfo {
  id: string;
  name: string;
  driver: string;
  inUse: boolean;
  builtin: boolean;
}

export interface DockerOverview {
  status: DockerStatus;
  images: DockerImageInfo[];
  containers: DockerContainerInfo[];
  volumes: DockerVolumeInfo[];
  networks: DockerNetworkInfo[];
  buildCacheRecords: number;
  buildCacheBytes: number;
  reclaimableBytes: number;
}

export type DockerResourceKind =
  | "image"
  | "container"
  | "volume"
  | "network"
  | "build_cache";

export interface DockerDeleteRequest {
  kind: DockerResourceKind;
  ids: string[];
  dryRun?: boolean;
}

export interface FileEntry {
  path: string;
  sizeBytes: number;
  modifiedMs: number;
}

export interface DuplicateGroup {
  id: string;
  hash: string;
  sizeBytes: number;
  wastedBytes: number;
  files: FileEntry[];
}

export interface DuplicateScanResult {
  groups: DuplicateGroup[];
  scannedFiles: number;
  scannedBytes: number;
  wastedBytes: number;
  durationMs: number;
  cancelled: boolean;
}

export interface DedupeOptions {
  roots: string[];
  excludes?: string[];
  minSizeBytes?: number;
  includeHidden?: boolean;
  followLinks?: boolean;
}

export interface GroupDeletePlan {
  keep: string[];
  remove: DeleteTarget[];
}

export interface DuplicateDeleteRequest {
  groups: GroupDeletePlan[];
  mode: DeleteMode;
  dryRun?: boolean;
}

export interface Settings {
  language: string;
  theme: string;
  defaultDeleteMode: string;
  excludes: string[];
}
