import { defineStore } from "pinia";
import { api } from "../api";
import { i18nState } from "../i18n";
import type { Settings } from "../types";

function applyTheme(theme: string) {
  const dark =
    theme === "dark" ||
    (theme === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches);
  document.documentElement.dataset.theme = dark ? "dark" : "light";
}

export const useSettingsStore = defineStore("settings", {
  state: () => ({
    settings: {
      language: "id",
      theme: "system",
      defaultDeleteMode: "trash",
      excludes: ["node_modules", ".git", "target", "*.iso"],
    } as Settings,
    loaded: false,
  }),
  actions: {
    async load() {
      try {
        this.settings = await api.getSettings();
      } catch {
        // keep defaults
      }
      this.loaded = true;
      this.apply();
    },
    apply() {
      i18nState.locale = this.settings.language === "en" ? "en" : "id";
      applyTheme(this.settings.theme);
    },
    async save() {
      await api.saveSettings(this.settings);
      this.apply();
    },
  },
});

// React to OS theme changes when in "system" mode.
window
  .matchMedia("(prefers-color-scheme: dark)")
  .addEventListener("change", () => {
    const s = useSettingsStore();
    if (s.settings.theme === "system") applyTheme("system");
  });
