<script setup>
import { computed, onMounted, ref } from "vue";
import { marked } from "marked";
import DOMPurify from "dompurify";
import katex from "katex";
import "katex/dist/katex.min.css";
import markedKatex from "marked-katex-extension";
import { useKernel } from "./composables/useKernel";
import GeometryFigure from "./components/GeometryFigure.vue";
import { SAMPLE_TRIANGLE } from "./lib/geometry.js";

marked.use(
  markedKatex({
    throwOnError: false,
    output: "htmlAndMathml",
  }),
);

// XSS 防线：Markdown + KaTeX 渲染后的 HTML 一律经 DOMPurify 净化；
// 禁掉脚本/事件/危险标签，默认策略同时拦截 javascript: 等危险 URL。
function renderMarkdown(text) {
  const html = marked.parse(text, { async: false });
  return DOMPurify.sanitize(html, {
    FORBID_TAGS: [
      "script",
      "style",
      "iframe",
      "object",
      "embed",
      "form",
      "input",
      "textarea",
      "button",
      "svg",
      "math",
    ],
    FORBID_ATTR: ["onerror", "onload", "onclick", "onmouseover", "onfocus"],
  });
}

const kernel = useKernel();

const status = ref("准备中");
const ready = ref(false);
const busy = ref(false);
const inputText = ref("");
const toolStatus = ref(null); // { entry, message, icon }
const showGeometryDemo = ref(false);

const bubbles = ref([]); // { type: user|assistant|error|reasoning, text, messageId }
let assistantIndex = -1;
let reasoningText = "";
let reasoningIndex = -1;
const currentStreamId = ref(null);

const canSend = computed(() => ready.value && !busy.value && inputText.value.trim());

function scrollBottom() {
  requestAnimationFrame(() => {
    const el = document.getElementById("messages");
    if (el) el.scrollTop = el.scrollHeight;
  });
}

function addBubble(type, text, messageId = null) {
  bubbles.value.push({ type, text, messageId });
  scrollBottom();
}

function ensureAssistant(messageId) {
  if (assistantIndex >= 0 && bubbles.value[assistantIndex]?.messageId === messageId) {
    return bubbles.value[assistantIndex];
  }
  assistantIndex = bubbles.value.length;
  addBubble("assistant", "", messageId);
  currentStreamId.value = messageId;
  return bubbles.value[assistantIndex];
}

function ensureReasoning(delta) {
  reasoningText += delta;
  if (reasoningIndex < 0) {
    reasoningIndex = bubbles.value.length;
    addBubble("reasoning", reasoningText);
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

function handleFrame(frame) {
  if (frame.type === "response") {
    if (frame.error) {
      addBubble("error", `请求失败：${frame.error.message}`);
      busy.value = false;
      finalize();
    } else {
      // get_state 等回执到达 → 链路就绪。
      ready.value = true;
      status.value = "就绪";
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
      toolStatus.value = { entry: e.entry, message: e.ok ? "完成" : "失败", icon: e.icon };
      break;
    case "turn_end":
      finalize();
      toolStatus.value = null;
      busy.value = false;
      status.value = "就绪";
      break;
    case "error":
      finalize();
      addBubble("error", e.message);
      busy.value = false;
      break;
  }
}

async function sendMessage() {
  const text = inputText.value.trim();
  if (!text || busy.value) return;
  inputText.value = "";
  addBubble("user", text);
  busy.value = true;
  status.value = "正在回答";
  try {
    await kernel.sendLine("send_user_message", { text });
  } catch (err) {
    addBubble("error", `发送失败：${err}`);
    busy.value = false;
  }
}

async function abortTurn() {
  await kernel.sendLine("abort");
}

async function pickHomework() {
  const path = await kernel.pickHomeworkFile();
  if (!path) return;
  inputText.value = `请批改这份作业：${path}`;
  await sendMessage();
}

onMounted(() => {
  kernel.onFrame(handleFrame);
  kernel
    .start()
    .then(() => {
      status.value = "自检中…";
    })
    .catch((err) => {
      addBubble("error", `内核启动失败：${err}`);
      status.value = "异常";
    });
});
</script>

<template>
  <header>
    <div class="brand">
      <img class="icon" src="/icons/book-open-variant.svg" alt="" />
      错题 Agent
    </div>
    <div class="status" :class="{ busy }">
      <span class="dot"></span>{{ status }}
    </div>
  </header>

  <main id="messages">
    <TransitionGroup name="msg" tag="div" class="bubbles">
      <div
        v-for="(b, i) in bubbles"
        :key="i"
        :class="[
          'bubble',
          b.type,
          {
            streaming:
              b.type === 'assistant' &&
              currentStreamId &&
              b.messageId === currentStreamId,
          },
        ]"
      >
        <details v-if="b.type === 'reasoning'" class="reasoning" open>
          <summary>
            <img class="icon" src="/icons/brain.svg" alt="" />
            思考过程（点击折叠）
          </summary>
          <div class="reasoning-body">{{ b.text }}</div>
        </details>
        <div
          v-else-if="b.type === 'assistant'"
          class="md-body"
          v-html="renderMarkdown(b.text)"
        ></div>
        <template v-else>{{ b.text }}</template>
      </div>
    </TransitionGroup>
  </main>

  <footer>
    <Transition name="fade">
      <div v-if="toolStatus" class="tool-status">
        <img
          v-if="toolStatus.icon"
          class="icon"
          :src="`/icons/${toolStatus.icon.replace(':', '-')}.svg`"
          alt=""
        />
        {{ toolStatus.entry }}：{{ toolStatus.message }}
      </div>
    </Transition>
    <div v-if="showGeometryDemo" class="geometry-demo">
      <div class="geometry-demo-head">
        <span>几何渲染器示例（diagram_spec → SVG，场景二预研）</span>
        <button class="ghost" @click="showGeometryDemo = false">收起</button>
      </div>
      <GeometryFigure :spec="SAMPLE_TRIANGLE" />
    </div>
    <div class="input-row">
      <input
        v-model="inputText"
        type="text"
        placeholder="发消息，或点「作业」选择文件让 Agent 批改"
        autocomplete="off"
        @keydown.enter="sendMessage"
      />
      <button id="pickBtn" @click="pickHomework">
        <img class="icon" src="/icons/upload.svg" alt="" />作业
      </button>
      <button class="ghost" @click="showGeometryDemo = !showGeometryDemo">
        图形
      </button>
      <button id="sendBtn" :disabled="!canSend" @click="sendMessage">
        <img class="icon" src="/icons/send.svg" alt="" />发送
      </button>
      <button v-if="busy" id="stopBtn" class="danger" @click="abortTurn">
        <img class="icon" src="/icons/stop-circle.svg" alt="" />停止
      </button>
    </div>
  </footer>
</template>
