import { createApp } from "vue";
import { addCollection } from "@iconify/vue";
import mdiIcons from "@iconify-json/mdi/icons.json";
import App from "./App.vue";
import "./style.css";
import "@fontsource/baloo-2/400.css";
import "@fontsource/baloo-2/600.css";
import "@fontsource/baloo-2/700.css";

// 离线图标：把 mdi 图标集注册进 Iconify 运行时，避免 WebView 在线拉取。
addCollection(mdiIcons);

createApp(App).mount("#app");
