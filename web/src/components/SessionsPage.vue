<script setup>
import { computed, onMounted, ref } from "vue";
import { Icon } from "@iconify/vue";
import MessageBubble from "./MessageBubble.vue";
import {
  buildSessionView,
  getActiveChain,
  navigateBranch,
  renderPath,
} from "../lib/messages";
import { loadToolCatalog } from "../lib/tools";

const props = defineProps({ kernel: { type: Object, required: true } });

const loading = ref(false);
const error = ref("");
const sessions = ref([]);
const detail = ref(null); // { meta, messages }
const sessionView = ref(null); // buildSessionView（含逐节点版本指针）

const chainBubbles = computed(() =>
  sessionView.value ? renderPath(sessionView.value, { history: true }) : [],
);

/** < / > 切换版本：本地改版本指针；活跃会话同步服务端（继续发送从所选版本走）。 */
function switchBranch(bubble, dir = 1) {
  if (!sessionView.value) return;
  navigateBranch(sessionView.value, bubble.messageId, dir);
  if (detail.value?.meta?.status === "active") {
    const chain = getActiveChain(sessionView.value);
    const end = chain.length ? String(chain[chain.length - 1].id) : null;
    if (end) {
      props.kernel
        .call("switch_branch", { message_id: end })
        .catch(() => {});
    }
  }
}

async function loadSessions() {
  loading.value = true;
  error.value = "";
  detail.value = null;
  try {
    const r = await props.kernel.call("list_sessions", {}, 10000);
    // 用客户端 localStorage 增强的时间排序（比后端 last_activity_at 更实时）
    sessions.value = (r.sessions || []).sort((a, b) => {
      const ta = sessionActivityTime(a);
      const tb = sessionActivityTime(b);
      return tb - ta;
    });
  } catch (e) {
    error.value =
      e.code === "not_implemented"
        ? "会话历史接口尚未接通，请稍后再试"
        : `加载失败：${e.message}`;
    sessions.value = [];
  } finally {
    loading.value = false;
  }
}

async function openSession(key) {
  loading.value = true;
  error.value = "";
  try {
    // 工具目录与会话详情并行拉取：历史消息里的工具标题/图标来自 list_tools。
    const [r] = await Promise.all([
      props.kernel.call("read_session", { key }, 10000),
      loadToolCatalog(props.kernel),
    ]);
    // 把 key 注入 meta 以便 formatSessionTime 查 localStorage
    if (r.meta) r.meta._key = key;
    detail.value = r;
    sessionView.value = buildSessionView(
      r.messages,
      r.meta?.active_path || null,
    );
  } catch (e) {
    error.value = `读取会话失败：${e.message}`;
  } finally {
    loading.value = false;
  }
}

function formatTime(iso) {
  if (!iso) return "";
  try {
    return new Date(iso).toLocaleString("zh-CN", { hour12: false });
  } catch {
    return "";
  }
}

/** 获取会话最近活动时间：优先 localStorage 里的客户端记录（更实时），回退后端时间 */
const LS_ACTIVITY_PREFIX = "ma:last-activity:";
function sessionActivityTime(s) {
  if (!s) return null;
  const key = s.key || s._key;
  const backendDate = new Date(s.last_activity_at || s.created_at || 0);
  try {
    if (key) {
      const stored = localStorage.getItem(LS_ACTIVITY_PREFIX + key);
      if (stored) {
        const storedDate = new Date(stored);
        // 只有客户端记录比后端时间新时才用（说明后端滞后）
        if (!isNaN(storedDate) && storedDate > backendDate) return storedDate;
      }
    }
  } catch { /* quota 满时静默 */ }
  return backendDate;
}

function formatSessionTime(s, iso) {
  const t = sessionActivityTime(s);
  if (!t || isNaN(t.getTime())) return formatTime(iso);
  return t.toLocaleString("zh-CN", { hour12: false });
}

function goalText(s) {
  if (!s?.goal) return "（无目标）";
  if (typeof s.goal === "string") return s.goal;
  return s.goal.text || "（无目标）";
}

onMounted(loadSessions);
</script>

<template>
  <div class="page">
    <div class="page-head">
      <h2>会话历史</h2>
      <button class="btn ghost" :disabled="loading" @click="loadSessions">
        <Icon icon="mdi:refresh" width="18" />刷新
      </button>
    </div>

    <p v-if="error" class="alert" role="alert">
      <Icon icon="mdi:alert-circle-outline" width="18" />{{ error }}
    </p>

    <div v-if="loading" class="empty">
      <Icon icon="mdi:loading" width="28" class="spin" />
      <p>正在读取…</p>
    </div>

    <template v-else-if="detail">
      <button class="btn ghost back" @click="detail = null">
        <Icon icon="mdi:arrow-left" width="18" />返回列表
      </button>
      <div class="session-meta">
        <span class="badge">目标：{{ goalText(detail.meta) }}</span>
        <span class="muted">最后活动：{{ formatSessionTime(detail.meta) }}</span>
      </div>
      <div class="session-detail">
        <TransitionGroup name="msg" tag="div" class="bubbles">
          <MessageBubble
            v-for="b in chainBubbles"
            :key="b.messageId"
            :bubble="b"
            :streaming="false"
            @switch-branch="switchBranch"
          />
        </TransitionGroup>
      </div>
    </template>

    <template v-else>
      <div v-if="!sessions.length" class="empty">
        <Icon icon="mdi:history" width="36" />
        <p>还没有会话记录。</p>
      </div>
      <div v-else class="session-list">
        <button v-for="s in sessions" :key="s.key" class="card session-row" @click="openSession(s.key)">
          <div class="session-row-main">
          <span class="session-icon"><Icon icon="mdi:chat-outline" width="20" /></span>
            <div>
              <div class="session-goal">{{ goalText(s) }}</div>
              <div class="muted">
                {{ formatSessionTime(s) }}
                <span class="badge weak" style="margin-left: 8px">{{ s.status }}</span>
              </div>
            </div>
          </div>
          <Icon icon="mdi:chevron-right" width="20" />
        </button>
      </div>
    </template>
  </div>
</template>
