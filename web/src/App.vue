<script setup>
import { onMounted, provide, ref } from "vue";
import { Icon } from "@iconify/vue";
import { useKernel } from "./composables/useKernel";
import ChatPage from "./components/ChatPage.vue";
import MistakesPage from "./components/MistakesPage.vue";
import SessionsPage from "./components/SessionsPage.vue";
import SettingsPage from "./components/SettingsPage.vue";
import OobePage from "./components/OobePage.vue";

const kernel = useKernel();
provide("kernel", kernel);

const ready = ref(false);
const busy = ref(false);
const status = ref("准备中");
const view = ref("chat");
const oobeOpen = ref(false);
const sidebarOpen = ref(true);

const navItems = [
  { id: "chat", label: "聊天", icon: "mdi:chat-processing-outline" },
  { id: "mistakes", label: "错题本", icon: "mdi:format-list-bulleted" },
  { id: "sessions", label: "会话", icon: "mdi:history" },
  { id: "settings", label: "设置", icon: "mdi:cog-outline" },
];

const viewMeta = {
  chat: { sub: "和 Agent 对话，上传作业自动批改" },
  mistakes: { sub: "错题自动归档，随时回顾错因" },
  sessions: { sub: "历史会话与消息树分支回放" },
  settings: { sub: "模型接入与本地数据配置" },
};

function onStatus(s) {
  busy.value = s.busy;
  status.value = s.text;
}

function navigate(viewId) {
  view.value = viewId;
}

function toggleSidebar() {
  sidebarOpen.value = !sidebarOpen.value;
}

onMounted(async () => {
  try {
    await kernel.start();
    ready.value = true;
    status.value = "就绪";
    try {
      const s = await kernel.call("get_settings", {}, 8000);
      if (!s.main_model?.key_set || !s.vision_model?.key_set) {
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
</script>

<template>
  <div class="app">
    <OobePage v-if="oobeOpen" :kernel="kernel" @done="oobeOpen = false" />

    <aside class="sidebar" :class="{ collapsed: !sidebarOpen }">
      <div class="brand">
        <span class="brand-mark">
          <Icon icon="mdi:book-education-outline" width="24" />
        </span>
        <span class="brand-text">
          <span class="brand-name">错题 Agent</span>
          <span class="brand-sub">本地智能错题助手</span>
        </span>
      </div>
      <nav class="nav" aria-label="主导航">
        <span class="nav-label">工作台</span>
        <button
          v-for="item in navItems"
          :key="item.id"
          class="nav-item"
          :class="{ active: view === item.id }"
          :aria-current="view === item.id ? 'page' : undefined"
          @click="view = item.id"
        >
          <Icon :icon="item.icon" width="20" />
          <span>{{ item.label }}</span>
        </button>
      </nav>
      <div class="sidebar-foot">
        <div class="status-pill" :class="{ busy, ready: ready && !busy }">
          <span class="dot"></span><span class="status-text">{{ status }}</span>
        </div>
        <p class="privacy-note">数据与密钥只保存在本机</p>
      </div>
    </aside>

    <section class="main">
      <header class="topbar">
        <div class="topbar-title">
          <h1>{{ navItems.find((n) => n.id === view)?.label }}</h1>
          <span class="topbar-sub">{{ viewMeta[view]?.sub }}</span>
        </div>
        <span class="topbar-tag">
          <Icon icon="mdi:shield-lock-outline" width="14" />本地优先
        </span>
      </header>

      <div class="view-host">
        <Transition name="view" mode="out-in">
          <ChatPage
            v-if="view === 'chat'"
            :key="'chat'"
            :kernel="kernel"
            :ready="ready"
            @status="onStatus"
            @navigate="navigate"
          />
          <MistakesPage v-else-if="view === 'mistakes'" :key="'mistakes'" :kernel="kernel" />
          <SessionsPage v-else-if="view === 'sessions'" :key="'sessions'" :kernel="kernel" />
          <SettingsPage v-else :key="'settings'" :kernel="kernel" />
        </Transition>
      </div>
    </section>
  </div>
</template>
