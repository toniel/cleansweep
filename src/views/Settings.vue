<script setup lang="ts">
import { computed, ref } from "vue";
import { t } from "../i18n";
import { useSettingsStore } from "../stores/settings";

const store = useSettingsStore();
const saved = ref(false);
const error = ref("");

const excludesText = computed({
  get: () => store.settings.excludes.join("\n"),
  set: (v: string) => {
    store.settings.excludes = v
      .split("\n")
      .map((s) => s.trim())
      .filter(Boolean);
  },
});

async function save() {
  error.value = "";
  try {
    await store.save();
    saved.value = true;
    setTimeout(() => (saved.value = false), 2000);
  } catch (e) {
    error.value = String(e);
  }
}
</script>

<template>
  <div style="max-width: 560px">
    <h1 class="page-title">{{ t("set.title") }}</h1>

    <div class="card stack">
      <div class="field">
        <label>{{ t("set.language") }}</label>
        <select class="input" v-model="store.settings.language">
          <option value="id">Indonesia</option>
          <option value="en">English</option>
        </select>
      </div>

      <div class="field">
        <label>{{ t("set.theme") }}</label>
        <select class="input" v-model="store.settings.theme">
          <option value="system">{{ t("set.themeSystem") }}</option>
          <option value="light">{{ t("set.themeLight") }}</option>
          <option value="dark">{{ t("set.themeDark") }}</option>
        </select>
      </div>

      <div class="field">
        <label>{{ t("set.deleteMode") }}</label>
        <select class="input" v-model="store.settings.defaultDeleteMode">
          <option value="trash">{{ t("set.trash") }}</option>
          <option value="permanent">{{ t("set.permanent") }}</option>
        </select>
      </div>

      <div class="field">
        <label>{{ t("set.excludes") }}</label>
        <textarea class="input" v-model="excludesText"></textarea>
      </div>

      <div class="row">
         <button class="btn btn-primary" @click="save">
           <font-awesome-icon icon="fa-solid fa-check" aria-hidden="true" />
           {{ t("set.save") }}
         </button>
         <span v-if="saved" class="text-ok">
           <font-awesome-icon icon="fa-solid fa-check" aria-hidden="true" />
           {{ t("set.saved") }}
         </span>
        <span v-if="error" class="text-danger">{{ error }}</span>
      </div>
    </div>
  </div>
</template>
