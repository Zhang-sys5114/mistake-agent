import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import { cpSync, existsSync, readFileSync } from "node:fs";
import { extname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL(".", import.meta.url));
const pyodideDir = join(root, "node_modules/pyodide");

const mime = {
  ".wasm": "application/wasm",
  ".js": "text/javascript",
  ".mjs": "text/javascript",
  ".zip": "application/zip",
  ".json": "application/json",
  ".map": "application/json",
  ".html": "text/html",
  ".txt": "text/plain",
  ".css": "text/css",
  ".png": "image/png",
  ".svg": "image/svg+xml",
  ".whl": "application/octet-stream",
};

/**
 * Pyodide 本地资源：dev 经 /pyodide/ 中间件直出 node_modules，
 * build 时整目录拷进 dist/pyodide（本地优先，不依赖 CDN）。
 */
function pyodideAssets() {
  return {
    name: "pyodide-assets",
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        if (!req.url?.startsWith("/pyodide/")) return next();
        const rel = decodeURIComponent(req.url.slice("/pyodide/".length));
        const file = join(pyodideDir, rel);
        if (!file.startsWith(pyodideDir) || !existsSync(file)) {
          res.statusCode = 404;
          res.end("not found");
          return;
        }
        res.setHeader("Content-Type", mime[extname(file)] ?? "application/octet-stream");
        res.setHeader("Cache-Control", "no-cache");
        res.end(readFileSync(file));
      });
    },
    closeBundle() {
      cpSync(pyodideDir, join(root, "dist/pyodide"), { recursive: true });
    },
  };
}

// Tauri 静态嵌入：build 产物 web/dist 由 tauri.conf.json frontendDist 加载。
export default defineConfig({
  plugins: [vue(), pyodideAssets()],
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
