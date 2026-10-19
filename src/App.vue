<script setup lang="ts">
import { t } from "./i18n";

const nav = [
  { to: "/", key: "nav.dashboard" as const, icon: "fa-solid fa-gauge-high" },
  { to: "/junk", key: "nav.junk" as const, icon: "fa-solid fa-broom" },
  { to: "/docker", key: "nav.docker" as const, icon: "fa-brands fa-docker" },
  { to: "/duplicates", key: "nav.duplicates" as const, icon: "fa-solid fa-clone" },
  { to: "/settings", key: "nav.settings" as const, icon: "fa-solid fa-gear" },
];
</script>

<template>
  <div class="app-shell">
    <nav class="sidebar" aria-label="Main">
      <div class="brand">
        <font-awesome-icon icon="fa-solid fa-wand-magic-sparkles" aria-hidden="true" />
        <span>{{ t("app.name") }}</span>
      </div>
      <RouterLink
        v-for="item in nav"
        :key="item.to"
        :to="item.to"
        class="nav-item"
        exact-active-class="active"
        active-class="active"
      >
        <font-awesome-icon :icon="item.icon" aria-hidden="true" />
        <span>{{ t(item.key) }}</span>
      </RouterLink>
    </nav>
    <main class="main">
      <RouterView />
    </main>
  </div>
</template>
