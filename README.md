# CleanSweep

A cross-platform desktop system cleaner built with **Tauri v2** (Rust + Vue 3), aimed at developers. Safe, fast, and transparent: preview first, confirm, then delete (Trash by default).

## Features

- **Junk Cleaner** — scans system junk, browser caches, package manager caches, and developer toolchain caches (npm/pnpm/yarn, Composer, pip, Cargo, Gradle/Maven, Go). Category rules are declarative in `src-tauri/rules/junk_rules.toml`.
- **Docker Cleaner** — detects the Docker daemon and cleans unused images, containers, volumes, build cache, and networks. Includes pin/whitelist and dry-run mode.
- **Duplicate Finder** — staged duplicate file detection (size → partial hash → full BLAKE3 hash) with previews and report export.
- **Dashboard** — disk usage overview, Smart Scan, and cleanup history.
- **Settings** — language (ID/EN), theme, default delete mode, exclude/whitelist.

See the [PRD — Tauri System Cleaner (CleanSweep)](./PRD%20—%20Tauri%20System%20Cleaner%20%28CleanSweep%29.md) for the full specification.

## Stack

| Layer | Technology |
| --- | --- |
| Desktop shell | Tauri v2 |
| Frontend | Vue 3, TypeScript, Vite 6, Pinia, Vue Router |
| Backend | Rust (2021, rust-version 1.85) |

Frontend–backend communication uses Tauri commands (one-shot actions) and events/channels (streaming progress).

## Prerequisites

- **Node.js** 18+ and npm
- **Rust** (stable, ≥ 1.85) — install via [rustup](https://rustup.rs)
- Tauri v2 system dependencies for your platform (see [Tauri prerequisites](https://tauri.app/start/prerequisites/)):
  - **Linux (Debian/Ubuntu):** `libwebkit2gtk-4.1-dev`, `build-essential`, `curl`, `wget`, `file`, `libxdo-dev`, `libssl-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`
  - **Windows:** Microsoft C++ Build Tools + WebView2
  - **macOS:** Xcode Command Line Tools

The priority platform is Linux (Debian/Ubuntu/KDE/GNOME).

## Running in Development

Install dependencies once:

```bash
npm install
```

**Full desktop app** (Rust + frontend, hot reload):

```bash
npm run tauri dev
```

**Frontend only** (no Rust backend, opened in the browser at http://localhost:1420). Useful for UI work; some IPC-dependent features will not function:

```bash
npm run dev
```

The dev port is pinned to `1420` (strict) by Vite.

## Building

**Build the desktop app** (installer bundle for your platform):

```bash
npm run tauri build
```

Bundles are produced in `src-tauri/target/release/bundle/` (e.g. `.deb` / `.AppImage`, `.msi` / `.exe`, `.dmg`). The release profile is already tuned for a small installer (`opt-level = "z"`, LTO, strip).

**Build the frontend only** (runs typecheck, then outputs `dist/`):

```bash
npm run build
```

Preview the frontend build:

```bash
npm run preview
```

## Other Scripts

| Command | Purpose |
| --- | --- |
| `npm run typecheck` | TypeScript type check (`vue-tsc --noEmit`) |
| `npm run tauri <cmd>` | Access the Tauri CLI directly (e.g. `npm run tauri info`) |

## Project Structure

```
src/                 # Vue 3 frontend
  views/             # Dashboard, JunkCleaner, DockerCleaner, DuplicateFinder, Settings
  components/        # ConfirmDialog, ProgressBar
  stores/            # Pinia store (settings)
  api.ts             # Tauri invoke wrapper
src-tauri/           # Rust backend (Tauri)
  src/               # scanner, rules, docker, dedupe, fsops, store, overview, ...
  rules/             # Junk category rules (TOML)
  capabilities/      # Tauri permissions (minimal; FS access stays in Rust)
```

## Security

- Deletion defaults to moving to Trash; permanent deletion must be explicitly enabled.
- All deletions go through the `fsops` module with canonical path validation (prevents path traversal/symlink escape).
- Built-in forbidden path list (root, `/usr`, `/etc`, `C:\Windows`, etc.).
- The frontend only uses allowlisted commands; filesystem access is restricted on the Rust side.
