import { createApp } from "vue";
import { createPinia } from "pinia";
import App from "./App.vue";
import { router } from "./router";
import { useSettingsStore } from "./stores/settings";
import "./style.css";
import { library } from "@fortawesome/fontawesome-svg-core";
import { FontAwesomeIcon } from "@fortawesome/vue-fontawesome";
import {
  faArrowsRotate,
  faBroom,
  faCheck,
  faCheckDouble,
  faChevronDown,
  faChevronRight,
  faClone,
  faFileCsv,
  faFolderOpen,
  faFolderPlus,
  faGear,
  faGaugeHigh,
  faMagnifyingGlass,
  faWandMagicSparkles,
  faThumbtack,
  faTrashCan,
  faTriangleExclamation,
  faXmark,
} from "@fortawesome/free-solid-svg-icons";
import { faDocker } from "@fortawesome/free-brands-svg-icons";

library.add(
  faArrowsRotate,
  faBroom,
  faCheck,
  faCheckDouble,
  faChevronDown,
  faChevronRight,
  faClone,
  faDocker,
  faFileCsv,
  faFolderOpen,
  faFolderPlus,
  faGear,
  faGaugeHigh,
  faMagnifyingGlass,
  faWandMagicSparkles,
  faThumbtack,
  faTrashCan,
  faTriangleExclamation,
  faXmark,
);

const app = createApp(App);
app.component("font-awesome-icon", FontAwesomeIcon);
app.use(createPinia());
app.use(router);

// Load settings before mount so theme/locale apply immediately.
const settings = useSettingsStore();
settings.load().finally(() => app.mount("#app"));
