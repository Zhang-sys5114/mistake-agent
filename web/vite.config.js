import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";

// Tauri 静态嵌入：build 产物 web/dist 由 tauri.conf.json frontendDist 加载。
export default defineConfig({
  plugins: [vue()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
  },
});
