// Tauri ↔ sidecar 桥接：Channel 收帧，kernel_send 发请求。
import { Channel, invoke } from "@tauri-apps/api/core";

export function useKernel() {
  let nextId = 1;
  const listeners = new Set();

  function onFrame(cb) {
    listeners.add(cb);
  }

  async function start() {
    const channel = new Channel();
    channel.onmessage = (payload) => {
      let frame = payload;
      if (typeof payload === "string") {
        try {
          frame = JSON.parse(payload);
        } catch {
          return;
        }
      }
      listeners.forEach((cb) => cb(frame));
    };
    await invoke("start_kernel", { onFrame: channel });
    // 全链路自检：get_state 回执到达后才解锁发送。
    await sendLine("get_state");
  }

  async function sendLine(method, extra = {}) {
    await invoke("kernel_send", {
      line: JSON.stringify({ id: nextId++, method, ...extra }),
    });
  }

  function pickHomeworkFile() {
    return invoke("pick_homework_file");
  }

  return { onFrame, start, sendLine, pickHomeworkFile };
}
