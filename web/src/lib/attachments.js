// 附件持久副本（数据根目录 uploads/）→ 可渲染 URL。
// 经 Tauri 命令 read_upload 读取 base64，图片转 data URL、PDF 转 blob URL，
// 不依赖 asset 协议，任何情况下都能渲染（白名单在 Rust 侧校验）。

import { invoke } from "@tauri-apps/api/core";

const cache = new Map();

function mimeFor(name) {
  if (/\.png$/i.test(name)) return "image/png";
  if (/\.jpe?g$/i.test(name)) return "image/jpeg";
  if (/\.webp$/i.test(name)) return "image/webp";
  if (/\.bmp$/i.test(name)) return "image/bmp";
  return "application/octet-stream";
}

export async function attachmentUrl(path, name = "") {
  const key = path;
  if (cache.has(key)) return cache.get(key);
  const promise = (async () => {
    const b64 = await invoke("read_upload", { path });
    const isPdf = /\.pdf$/i.test(name || path);
    if (!isPdf) {
      return {
        kind: "image",
        url: `data:${mimeFor(name || path)};base64,${b64}`,
      };
    }
    const bin = atob(b64);
    const bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
    const blob = new Blob([bytes], { type: "application/pdf" });
    return { kind: "pdf", url: URL.createObjectURL(blob) };
  })();
  promise.catch(() => cache.delete(key));
  cache.set(key, promise);
  return promise;
}
