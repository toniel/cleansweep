import { createRouter, createWebHashHistory } from "vue-router";

export const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: "/", name: "dashboard", component: () => import("./views/Dashboard.vue") },
    { path: "/junk", name: "junk", component: () => import("./views/JunkCleaner.vue") },
    { path: "/docker", name: "docker", component: () => import("./views/DockerCleaner.vue") },
    { path: "/duplicates", name: "duplicates", component: () => import("./views/DuplicateFinder.vue") },
    { path: "/settings", name: "settings", component: () => import("./views/Settings.vue") },
  ],
});
