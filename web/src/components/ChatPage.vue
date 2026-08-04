<script setup>
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { Icon } from "@iconify/vue";
import MessageBubble from "./MessageBubble.vue";
import AttachmentViewer from "./AttachmentViewer.vue";
import { runPython } from "../lib/pyodide";
import { renderPath, toolIcon } from "../lib/messages";

const props = defineProps({
  kernel: { type: Object, required: true },
  ready: { type: Boolean, default: false },
});
const emit = defineEmits(["status"]);

const inputText = ref("");
const busy = ref(false);
const toolStatus = ref(null); // { entry, message, icon }
const bubbles = ref([]);
const editingId = ref(null);
const editingText = ref("");
const currentStreamId = ref(null);
const branchPointers = ref({});
const tools = ref([]); // 用户可见工具（list_tools，供输入候选）
const suggestions = ref([]);
const activeSuggestion = ref(-1);
const armedTool = ref(null); // Tab 确认的待调用工具 { entry, title, icon }
const inputEl = ref(null);

let unsubscribe = null;
let assistantIndex = -1;
let reasoningText = "";
let reasoningIndex = -1;
let pendingSendId = null;

const canSend = computed(
  () => props.ready && !busy.value && (inputText.value.trim() || armedTool.value),
);

const TOOL_NAME_RE = /^([a-z][a-z0-9_]*::[a-z][a-z0-9_]*)/i;
const GROUP_ORDER = ["批改", "学习", "记忆", "其它", "调试"];

const quickActions = [
  { id: "upload", label: "上传作业批改", desc: "图片或 PDF，自动判分并归档错题", icon: "mdi:upload", action: "upload" },
  { id: "mistakes", label: "查看错题本", desc: "按学科与知识点回顾错因", icon: "mdi:format-list-bulleted", action: "navigate" },
  { id: "settings", label: "配置模型", desc: "设置主模型与视觉模型密钥", icon: "mdi:cog-outline", action: "navigate" },
];

function onQuickAction(a) {
  if (a.action === "upload") {
    pickHomework(true);
  } else {
    emit("navigate", a.id);
  }
}

/** 输入时计算候选工具：匹配第一个 token（工具名/标题前缀）。 */
function computeSuggestions() {
  const firstToken = (inputText.value.match(/\S+/) || [""])[0];
  if (armedTool.value || !firstToken || firstToken.length < 2) {
    suggestions.value = [];
    return;
  }
  const q = firstToken.toLowerCase();
  const hasNs = firstToken.includes("::");
  suggestions.value = tools.value
    .filter(
      (t) =>
        t.entry.toLowerCase().includes(q) ||
        (t.title || "").toLowerCase().includes(q) ||
        (hasNs && t.entry.toLowerCase().startsWith(q)),
    )
    .slice(0, 8);
  activeSuggestion.value = suggestions.value.length ? 0 : -1;
}

/** 加载用户可见工具（候选数据全部来自后端 list_tools）。 */
async function loadTools() {
  if (!props.ready) return;
  try {
    const r = await props.kernel.listTools();
    tools.value = (r.tools || []).sort((a, b) => {
      const ga = GROUP_ORDER.indexOf(a.group || "其它");
      const gb = GROUP_ORDER.indexOf(b.group || "其它");
      return (
        (ga === -1 ? 99 : ga) - (gb === -1 ? 99 : gb) ||
        (a.title || a.entry).localeCompare(b.title || b.entry, "zh")
      );
    });
  } catch {
    tools.value = [];
  }
}

/** Tab 确认：补全工具名并进入待调用状态（工具名保留在输入框，后面接参数）。 */
function armTool(tool) {
  const entry = tool.entry;
  const m = inputText.value.match(TOOL_NAME_RE);
  if (m) {
    // 把触发联想的片段补全为完整工具名（后面的文本保留）。
    inputText.value = entry + inputText.value.slice(m[0].length);
  } else {
    // 工具栏按钮点击：把工具名放到输入框开头。
    const rest = inputText.value.trim();
    inputText.value = rest ? `${entry} ${rest}` : entry;
  }
  armedTool.value = {
    entry,
    title: tool.title || tool.entry,
    icon: tool.icon || toolIcon(tool.entry),
  };
  suggestions.value = [];
}

function unarmTool() {
  armedTool.value = null;
}

/** 输入变化：工具名被删除/改写时自动解除待调用状态。 */
function onInput() {
  if (armedTool.value) {
    const re = new RegExp(`^${armedTool.value.entry}(?:\\s|$)`);
    if (!re.test(inputText.value)) {
      armedTool.value = null;
    }
  }
  computeSuggestions();
}

/** 工具栏点击：选中/取消工具，进入待调用状态并聚焦输入框。 */
function pickTool(tool) {
  if (busy.value) return;
  if (armedTool.value?.entry === tool.entry) {
    unarmTool();
  } else {
    armTool(tool);
  }
  inputEl.value?.focus();
}

function onKeydown(e) {
  if (e.key === "Tab") {
    e.preventDefault();
    if (suggestions.value.length) {
      const t =
        suggestions.value[
          activeSuggestion.value >= 0 ? activeSuggestion.value : 0
        ];
      armTool(t);
    } else if (!armedTool.value) {
      // 候选框未弹出时兜底：输入的工具名精确匹配也直接确认。
      const firstToken = (inputText.value.match(/\S+/) || [""])[0];
      const exact = tools.value.find(
        (t) => t.entry.toLowerCase() === firstToken.toLowerCase(),
      );
      if (exact) armTool(exact);
    }
  } else if (e.key === "ArrowDown" && suggestions.value.length) {
    e.preventDefault();
    activeSuggestion.value =
      (activeSuggestion.value + 1) % suggestions.value.length;
  } else if (e.key === "ArrowUp" && suggestions.value.length) {
    e.preventDefault();
    activeSuggestion.value =
      (activeSuggestion.value - 1 + suggestions.value.length) %
      suggestions.value.length;
  }
}

function scrollBottom() {
  requestAnimationFrame(() => {
    const el = document.getElementById("messages");
    if (el) el.scrollTop = el.scrollHeight;
  });
}

function addBubble(b) {
  bubbles.value.push(b);
  scrollBottom();
}

function setStatus(b, text) {
  emit("status", { busy: b, text });
}

function ensureAssistant(messageId) {
  if (assistantIndex >= 0 && bubbles.value[assistantIndex]?.messageId === messageId) {
    return bubbles.value[assistantIndex];
  }
  assistantIndex = bubbles.value.length;
  addBubble({ type: "assistant", text: "", messageId });
  currentStreamId.value = messageId;
  return bubbles.value[assistantIndex];
}

function ensureReasoning(delta) {
  reasoningText += delta;
  if (reasoningIndex < 0) {
    reasoningIndex = bubbles.value.length;
    addBubble({ type: "reasoning", text: reasoningText });
  } else {
    bubbles.value[reasoningIndex].text = reasoningText;
  }
}

function finalize() {
  if (assistantIndex >= 0 && !bubbles.value[assistantIndex]?.text) {
    bubbles.value.splice(assistantIndex, 1);
  }
  assistantIndex = -1;
  reasoningIndex = -1;
  reasoningText = "";
  currentStreamId.value = null;
}

/** 全量历史：所有会话的消息按时间合并成一条大树（副本按消息 id 去重）。 */
async function refreshAllHistory() {
  try {
    const list = await props.kernel.call("list_sessions", {}, 8000);
    const arr = list.sessions || [];
    const seen = new Set();
    const all = [];
    for (const s of arr) {
      try {
        const detail = await props.kernel.call("read_session", { key: s.key }, 8000);
        for (const m of detail.messages || []) {
          const id = String(m.id);
          if (seen.has(id)) continue;
          seen.add(id);
          all.push(m);
        }
      } catch {
        // 单个会话读取失败不阻断整体历史。
      }
    }
    all.sort((a, b) => new Date(a.created_at) - new Date(b.created_at));
    if (all.length) {
      bubbles.value = renderPath(all);
      scrollBottom();
    }
  } catch (e) {
    // list_sessions/read_session 尚未接通时，聊天仍可用，只是没有分支/编辑入口。
    if (e.code !== "not_implemented") console.warn("会话回读失败：", e);
  }
}

let historyLoaded = false;
async function ensureHistory() {
  if (historyLoaded || !props.ready) return;
  historyLoaded = true;
  await refreshAllHistory();
}

async function handleComputeRequest(req) {
  toolStatus.value = {
    entry: "compute::verify",
    message: "正在运行 Python 验算…",
    icon: "mdi:calculator-variant",
  };
  try {
    const r = await runPython(req.code);
    await props.kernel.call("compute_result", {
      compute_id: req.id,
      stdout: r.stdout,
      stderr: r.stderr,
      duration_ms: r.durationMs,
    });
  } catch (e) {
    await props.kernel.call("compute_result", {
      compute_id: req.id,
      stdout: "",
      stderr: String(e?.message ?? e),
      duration_ms: 0,
    });
  } finally {
    if (toolStatus.value?.entry === "compute::verify") toolStatus.value = null;
  }
}

function handleFrame(frame) {
  if (frame.type === "response") {
    if (frame.id === pendingSendId && frame.error) {
      addBubble({ type: "error", text: `请求失败：${frame.error.message}` });
      busy.value = false;
      finalize();
      setStatus(false, "就绪");
    }
    return;
  }
  if (frame.type !== "event") return;
  const e = frame.event;
  switch (e.event) {
    case "message_delta":
      ensureAssistant(e.message_id).text += e.delta;
      scrollBottom();
      break;
    case "reasoning_delta":
      ensureReasoning(e.delta);
      break;
    case "tool_start":
      toolStatus.value = { entry: e.entry, message: "执行中", icon: e.icon };
      break;
    case "tool_progress":
      toolStatus.value = { entry: e.entry, message: e.message, icon: e.icon };
      break;
    case "tool_end":
      toolStatus.value = { entry: e.entry, message: e.ok ? "完成" : "失败", icon: toolIcon(e.entry) };
      break;
    case "compute_request":
      handleComputeRequest(e);
      break;
    case "turn_end":
      finalize();
      toolStatus.value = null;
      busy.value = false;
      setStatus(false, "就绪");
      refreshAllHistory();
      break;
    case "error":
      finalize();
      addBubble({ type: "error", text: e.message });
      busy.value = false;
      setStatus(false, "异常");
      refreshAllHistory();
      break;
  }
}

async function sendMessage() {
  const text = inputText.value.trim();
  if (busy.value) return;
  if (!text && !armedTool.value) return;
  if (armedTool.value) {
    const tool = armedTool.value;
    armedTool.value = null;
    const raw = inputText.value;
    inputText.value = "";
    const m = raw.match(TOOL_NAME_RE);
    const hint = (m ? raw.slice(m[0].length) : raw).trim();
    addBubble({
      type: "user",
      text: tool.title + (hint ? `：${hint}` : ""),
      toolIcon: tool.icon,
    });
    busy.value = true;
    setStatus(true, "正在调用工具");
    try {
      pendingSendId = await props.kernel.sendLine("send_user_message", {
        text: hint,
        force_tool: { entry: tool.entry, hint },
      });
    } catch (err) {
      addBubble({ type: "error", text: `发送失败：${err}` });
      busy.value = false;
      setStatus(false, "异常");
    }
    return;
  }
  inputText.value = "";
  addBubble({ type: "user", text });
  busy.value = true;
  setStatus(true, "正在回答");
  try {
    pendingSendId = await props.kernel.sendLine("send_user_message", { text });
  } catch (err) {
    addBubble({ type: "error", text: `发送失败：${err}` });
    busy.value = false;
    setStatus(false, "异常");
  }
}

async function abortTurn() {
  await props.kernel.sendLine("abort");
}

async function pickHomework(forced = false) {
  const picked = await props.kernel.pickHomeworkFile();
  if (!picked) return;
  if (forced) {
    addBubble({
      type: "user",
      text: "请批改这份作业",
      attachment: { path: picked.asset_path, name: picked.name },
    });
    busy.value = true;
    setStatus(true, "正在调用工具");
    try {
      pendingSendId = await props.kernel.sendLine("send_user_message", {
        text: picked.temp_path,
        force_tool: {
          entry: "grading::upload",
          hint: picked.temp_path,
          asset: { path: picked.asset_path, name: picked.name },
        },
      });
    } catch (err) {
      addBubble({ type: "error", text: `发送失败：${err}` });
      busy.value = false;
      setStatus(false, "异常");
    }
    return;
  }
  inputText.value = `请批改这份作业：${picked.temp_path}`;
  await sendMessage();
}

const viewer = ref(null);
function openAttachment(attachment) {
  viewer.value = attachment;
}

function startEdit(bubble) {
  editingId.value = bubble.messageId;
  editingText.value = bubble.text;
}

async function saveEdit() {
  const id = editingId.value;
  const text = editingText.value.trim();
  editingId.value = null;
  if (!id || !text) return;
  try {
    const r = await props.kernel.call("edit_message", { message_id: id, text });
    if (r.messages) bubbles.value = renderPath(r.messages);
    scrollBottom();
  } catch (e) {
    addBubble({ type: "error", text: `编辑失败：${e.message}` });
  }
}

async function switchBranch(bubble) {
  const ids = bubble.siblingIds || [];
  if (!ids.length) return;
  const key = bubble.parentId || "__root__";
  const idx = (branchPointers.value[key] ?? 0) % ids.length;
  const target = ids[idx];
  branchPointers.value = { ...branchPointers.value, [key]: (idx + 1) % ids.length };
  try {
    const r = await props.kernel.call("switch_branch", { message_id: target });
    if (r.messages) bubbles.value = renderPath(r.messages);
    scrollBottom();
  } catch (e) {
    addBubble({ type: "error", text: `分支切换失败：${e.message}` });
  }
}

async function copyText(text) {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    // 剪贴板不可用时静默（不阻断主流程）。
  }
}

onMounted(() => {
  unsubscribe = props.kernel.onFrame(handleFrame);
  loadTools();
  ensureHistory();
});
watch(
  () => props.ready,
  (v) => {
    if (v) ensureHistory();
    if (v) loadTools();
  },
);
onUnmounted(() => unsubscribe?.());
</script>

<template>
  <div class="chat-page">
    <AttachmentViewer
      v-if="viewer"
      :attachment="viewer"
      @close="viewer = null"
    />

    <main id="messages">
      <div v-if="!bubbles.length && !busy" class="empty chat-empty">
        <span class="empty-icon">
          <Icon icon="mdi:school-outline" width="36" />
        </span>
        <h2>开始你的学习吧</h2>
        <p>上传一份作业让 Agent 批改，或直接提问：错题会归档进错题本，跨会话记得住。</p>
        <div class="quick-actions">
          <button
            v-for="a in quickActions"
            :key="a.id"
            class="quick-card"
            @click="onQuickAction(a)"
          >
            <span class="quick-icon">
              <Icon :icon="a.icon" width="22" />
            </span>
            <span>
              <span class="quick-title">{{ a.label }}</span>
              <span class="quick-desc">{{ a.desc }}</span>
            </span>
            <Icon icon="mdi:chevron-right" width="18" class="quick-arrow" />
          </button>
        </div>
      </div>

      <TransitionGroup name="msg" tag="div" class="bubbles">
        <MessageBubble
          v-for="(b, i) in bubbles"
          :key="b.messageId || i"
          :bubble="b"
          :streaming="b.type === 'assistant' && currentStreamId && b.messageId === currentStreamId"
          @edit="startEdit"
          @switch-branch="switchBranch"
          @copy="copyText"
          @open-attachment="openAttachment"
        />
      </TransitionGroup>

      <div v-if="editingId" class="edit-box">
        <textarea
          v-model="editingText"
          rows="3"
          aria-label="编辑消息内容"
          @keydown.esc="editingId = null"
          @keydown.ctrl.enter="saveEdit"
        ></textarea>
        <div class="edit-actions">
          <button class="btn ghost" @click="editingId = null">取消</button>
          <button class="btn primary" :disabled="!editingText.trim()" @click="saveEdit">保存并派生新分支</button>
        </div>
      </div>
    </main>

    <footer class="chat-footer">
      <Transition name="fade">
        <div v-if="toolStatus" class="tool-status">
          <span class="spinner" aria-hidden="true"></span>
          <Icon :icon="toolStatus.icon || 'mdi:toolbox-outline'" width="18" />
          <span>{{ toolStatus.entry }}：{{ toolStatus.message }}</span>
        </div>
      </Transition>

      <div v-if="tools.length && !busy" class="tool-bar" role="toolbar" aria-label="工具">
        <button
          v-for="t in tools"
          :key="t.entry"
          class="tool-chip"
          :class="{ active: armedTool?.entry === t.entry }"
          :title="t.description"
          @click="pickTool(t)"
        >
          <Icon :icon="t.icon || 'mdi:toolbox-outline'" width="15" />
          <span>{{ t.title || t.entry }}</span>
        </button>
      </div>

      <div class="input-wrap">
        <Transition name="fade">
          <div
            v-if="suggestions.length"
            class="tool-suggest"
            role="listbox"
            aria-label="工具候选"
          >
            <button
              v-for="(t, i) in suggestions"
              :key="t.entry"
              class="tool-suggest-item"
              :class="{ active: i === activeSuggestion }"
              role="option"
              :aria-selected="i === activeSuggestion"
              @mousedown.prevent="armTool(t)"
            >
              <Icon :icon="t.icon || 'mdi:toolbox-outline'" width="16" />
              <span class="ts-title">{{ t.title || t.entry }}</span>
              <span class="ts-entry">{{ t.entry }}</span>
            </button>
            <p class="tool-suggest-hint">按 Tab 确认调用 · ↑↓ 选择 · 也可点选</p>
          </div>
        </Transition>

        <div class="input-shell" :class="{ armed: armedTool }">
          <span v-if="armedTool" class="armed-tool">
            <Icon :icon="armedTool.icon" width="16" />
            <span>{{ armedTool.title }}</span>
            <button
              class="armed-tool-x"
              aria-label="取消工具调用"
              @click="unarmTool"
            >
              <Icon icon="mdi:close" width="14" />
            </button>
          </span>
          <input
            ref="inputEl"
            v-model="inputText"
            type="text"
            :placeholder="armedTool ? '<可选参数>' : '发消息，或输入功能名（如：生成练习题）按 Tab 确认'"
            autocomplete="off"
            aria-label="消息输入框"
            @input="onInput"
            @keydown="onKeydown"
            @keydown.enter="sendMessage"
          />
          <span
            v-if="armedTool && inputText.trim() === armedTool.entry"
            class="param-hint"
            aria-hidden="true"
          >&lt;可选参数&gt;</span>
          <button class="btn ghost" id="pickBtn" aria-label="选择作业文件" title="选择作业文件" @click="pickHomework(true)">
            <Icon icon="mdi:paperclip" width="18" />
          </button>
          <button class="btn primary" id="sendBtn" :disabled="!canSend" @click="sendMessage">
            <Icon icon="mdi:send" width="18" />发送
          </button>
          <button v-if="busy" class="btn danger" id="stopBtn" @click="abortTurn">
            <Icon icon="mdi:stop-circle" width="18" />停止
          </button>
        </div>
      </div>
    </footer>
  </div>
</template>
