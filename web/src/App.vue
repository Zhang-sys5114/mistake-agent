<script setup>
import { onBeforeUnmount, onMounted, provide, ref } from "vue";
import { Icon } from "@iconify/vue";
import { useKernel } from "./composables/useKernel";
import ChatPage from "./components/ChatPage.vue";
import MistakesPage from "./components/MistakesPage.vue";
import SettingsPage from "./components/SettingsPage.vue";
import SessionListPanel from "./components/SessionListPanel.vue";
import OobePage from "./components/OobePage.vue";

const kernel = useKernel();
provide("kernel", kernel);

/* ──── 跨页面跳转：错题本"追问" → 聊天页 ──── */
const navigateToChatMessage = ref("");
provide("navigateToChatMessage", navigateToChatMessage);

function navigateToChatWithMessage(msg) {
  navigateToChatMessage.value = msg;
  view.value = "chat";
}
provide("navigateToChatWithMessage", navigateToChatWithMessage);

const ready = ref(false);
const busy = ref(false);
const status = ref("准备中");
const view = ref("chat");
const oobeOpen = ref(false);
const sidebarLocked = ref(false);
// 会话列表是应用级侧栏（开关图标在「设置」下方，列表展开在同一列的下方），
// 所以「当前会话」也归 App 持有：ChatPage 只读它、不再自己推导。
const sessionPanelOpen = ref(true);
const activeSessionKey = ref(null);
const sessionList = ref(null);

const navItems = [
  { id: "chat", label: "聊天", icon: "mdi:chat-processing-outline" },
  { id: "mistakes", label: "错题本", icon: "mdi:format-list-bulleted" },
  { id: "settings", label: "设置", icon: "mdi:cog-outline" },
];

const viewMeta = {
  chat: { sub: "和 Agent 对话，上传作业自动批改" },
  mistakes: { sub: "错题自动归档，随时回顾错因" },
  settings: { sub: "模型接入与本地数据配置" },
};

function onStatus(s) {
  busy.value = s.busy;
  status.value = s.text;
}

function navigate(viewId) {
  view.value = viewId;
}

function toggleSidebarLock() {
  sidebarLocked.value = !sidebarLocked.value;
}

/** 会话列表开关（图标在「设置」正下方）：列表就展开在侧栏这一列里。 */
function toggleSessionPanel() {
  sessionPanelOpen.value = !sessionPanelOpen.value;
}

/** 用户点列表里的会话：服务端归档旧 Active 并激活目标（ADR-0044）。 */
async function onSelectSession(key) {
  if (!key || busy.value) return;
  if (key !== activeSessionKey.value) {
    try {
      await kernel.call("open_session", { key }, 15000);
    } catch (e) {
      status.value = `切换会话失败：${e.message}`;
      return;
    }
  }
  activeSessionKey.value = key;
  view.value = "chat"; // 面板常驻侧栏：在别的页面点会话也回到聊天
}

/** 会话列表被改动（新建/重命名/删除）或回合结束后：让面板自己重列。 */
function refreshSessionList() {
  sessionList.value?.refreshList();
}

/** 服务端唯一 Active 会话即当前会话（单 Active 不变量），用它兜底同步选中项。 */
async function syncActiveSession() {
  if (!ready.value) return;
  try {
    const r = await kernel.call("list_sessions", {}, 8000);
    const arr = r.sessions || [];
    const active = arr.find((s) => s.status === "active");
    if (active?.key) activeSessionKey.value = active.key;
    else if (!arr.some((s) => s.key === activeSessionKey.value)) activeSessionKey.value = null;
  } catch {
    // 列表读不到不阻塞界面（会话为空时聊天区显示空状态）。
  }
}

/* ---- Ripple effect ---- */
let rippleCanvas = null;
let rippleCtx = null;
let ripples = [];
let rippleRaf = null;

function initRipple() {
  rippleCanvas = document.getElementById("ripple-canvas");
  if (!rippleCanvas) return;
  rippleCtx = rippleCanvas.getContext("2d");
  resizeRipple();
  window.addEventListener("resize", resizeRipple);
  document.addEventListener("click", onRippleClick);
  rippleRaf = requestAnimationFrame(animateRipples);
}

function resizeRipple() {
  if (!rippleCanvas) return;
  rippleCanvas.width = window.innerWidth;
  rippleCanvas.height = window.innerHeight;
}

function onRippleClick(e) {
  const x = e.clientX;
  const y = e.clientY;
  // spawn 6 rings
  for (let i = 0; i < 6; i++) {
    ripples.push({
      x,
      y,
      radius: 2,
      maxRadius: 30 + i * 14,
      opacity: 0.5,
      startTime: performance.now() + i * 40,
      speed: 0.6 + i * 0.08,
    });
  }
}

function animateRipples(now) {
  if (!rippleCtx || !rippleCanvas) {
    rippleRaf = requestAnimationFrame(animateRipples);
    return;
  }
  rippleCtx.clearRect(0, 0, rippleCanvas.width, rippleCanvas.height);

  ripples = ripples.filter((r) => {
    if (now < r.startTime) return true;
    const elapsed = now - r.startTime;
    const progress = elapsed / 800; // 0.8s lifetime
    if (progress >= 1) return false;

    r.radius += r.speed;
    r.opacity = 0.5 * (1 - progress);

    rippleCtx.beginPath();
    rippleCtx.arc(r.x, r.y, r.radius, 0, Math.PI * 2);
    rippleCtx.strokeStyle = `rgba(37,99,235,${r.opacity.toFixed(3)})`;
    rippleCtx.lineWidth = 1.5;
    rippleCtx.stroke();
    return true;
  });

  rippleRaf = requestAnimationFrame(animateRipples);
}

function destroyRipple() {
  if (rippleRaf) cancelAnimationFrame(rippleRaf);
  window.removeEventListener("resize", resizeRipple);
  document.removeEventListener("click", onRippleClick);
}

onMounted(async () => {
  initRipple();
  try {
    await kernel.start();
    ready.value = true;
    status.value = "就绪";
    await syncActiveSession();
    try {
      const s = await kernel.call("get_settings", {}, 8000);
      if (!s.main_model?.key_set) {
        oobeOpen.value = true;
      }
    } catch {
      // 设置读取失败不阻塞主界面。
    }
  } catch (e) {
    status.value = "内核异常";
    console.error("内核启动失败：", e);
  }
});

onBeforeUnmount(() => {
  destroyRipple();
});
</script>

<template>
  <div class="app">
    <OobePage v-if="oobeOpen" :kernel="kernel" @done="oobeOpen = false" />

    <aside
      class="sidebar"
      :class="{ expanded: sidebarLocked || sessionPanelOpen, 'session-open': sessionPanelOpen }"
    >
      <div class="brand">
        <span class="brand-mark">
          <Icon icon="mdi:book-education-outline" width="22" />
        </span>
        <span class="brand-text">
          <span class="brand-name">错题 Agent</span>
          <span class="brand-sub">本地智能错题助手</span>
        </span>
        <button
          class="brand-lock"
          :class="{ locked: sidebarLocked }"
          :title="sidebarLocked ? '折叠侧栏' : '锁定侧栏'"
          @click="toggleSidebarLock"
        >
          <Icon :icon="sidebarLocked ? 'mdi:pin' : 'mdi:pin-outline'" width="16" />
        </button>
      </div>
      <nav class="nav" aria-label="主导航">
        <span class="nav-label">工作台</span>
        <button
          v-for="item in navItems"
          :key="item.id"
          class="nav-item"
          :class="{ active: view === item.id }"
          :aria-current="view === item.id ? 'page' : undefined"
          :title="item.label"
          @click="view = item.id"
        >
          <Icon :icon="item.icon" width="20" />
          <span>{{ item.label }}</span>
        </button>
        <button
          class="nav-item"
          :class="{ 'panel-toggle-on': sessionPanelOpen }"
          :title="sessionPanelOpen ? '收起会话列表' : '展开会话列表'"
          :aria-pressed="sessionPanelOpen"
          aria-label="会话列表"
          @click="toggleSessionPanel"
        >
          <Icon icon="mdi:history" width="20" />
          <span>会话列表</span>
        </button>
      </nav>
      <div v-if="sessionPanelOpen" class="sidebar-sessions">
        <SessionListPanel
          ref="sessionList"
          :kernel="kernel"
          :active-key="activeSessionKey"
          :busy="busy"
          @select="onSelectSession"
          @changed="refreshSessionList"
          @collapse="sessionPanelOpen = false"
        />
      </div>
      <div class="sidebar-foot">
        <div class="status-pill" :class="{ busy, ready: ready && !busy }">
          <span class="dot"></span><span class="status-text">{{ status }}</span>
        </div>
      </div>
    </aside>

    <section class="main">
      <header class="topbar">
        <div class="topbar-title">
          <h1>{{ navItems.find((n) => n.id === view)?.label }}</h1>
          <span class="topbar-sub">{{ viewMeta[view]?.sub }}</span>
        </div>
      </header>

      <div class="view-host">
        <Transition name="view" mode="out-in">
          <ChatPage
            v-if="view === 'chat'"
            :key="'chat'"
            :kernel="kernel"
            :ready="ready"
            v-model:active-key="activeSessionKey"
            @status="onStatus"
            @navigate="navigate"
            @sessions-dirty="refreshSessionList"
          />
          <MistakesPage v-else-if="view === 'mistakes'" :key="'mistakes'" :kernel="kernel" />
          <SettingsPage v-else :key="'settings'" :kernel="kernel" />
        </Transition>
      </div>
    </section>
  </div>
</template>
