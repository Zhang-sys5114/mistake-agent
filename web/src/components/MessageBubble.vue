<script setup>
import { ref, watch } from "vue";
import { Icon } from "@iconify/vue";
import PracticeQuestion from "./PracticeQuestion.vue";
import WeakPointList from "./WeakPointList.vue";

const props = defineProps({
  bubble: { type: Object, required: true },
  streaming: { type: Boolean, default: false },
  editing: { type: Boolean, default: false },
});

const emit = defineEmits([
  "edit",
  "switch-branch",
  "copy",
  "open-attachment",
  "save-edit",
  "cancel-edit",
  "tool-interact",
]);

const editText = ref("");
watch(
  () => props.editing,
  (on) => {
    if (on) editText.value = props.bubble.text || "";
  },
);

function saveEdit() {
  const text = editText.value.trim();
  if (!text) return;
  emit("save-edit", text);
}

function attachmentIcon(att) {
  if (/\.pdf$/i.test(att.name || att.path)) return "mdi:file-pdf-box";
  if (/\.(png|jpe?g|webp|bmp)$/i.test(att.name || att.path)) return "mdi:image";
  return "mdi:file-outline";
}

/* ──── 工具结果辅助 ──── */

/** 从 practice::gaps 结果中提取薄弱点数组（兼容数组/包裹对象）。 */
function extractWeakPoints(result) {
  if (!result) return [];
  if (Array.isArray(result)) return result;
  for (const k of ["gaps", "weaknesses", "points", "items"]) {
    if (Array.isArray(result[k])) return result[k];
  }
  return [];
}

/** 检测结果是否像薄弱点数据 */
function isGapsResult(bubble) {
  const e = (bubble.entry || "").toLowerCase();
  if (e === "practice::gaps" || e.includes("gaps")) return true;
  if (bubble.result && extractWeakPoints(bubble.result).length) return true;
  return false;
}

/** 检测结果是否像练习题数据 */
function isPracticeQuestionResult(bubble) {
  const e = (bubble.entry || "").toLowerCase();
  if (e === "practice::generate" || e.includes("generate")) return true;
  const r = bubble.result;
  if (!r) return false;
  // 按数据结构判断：有 question_text 的就是练习题
  if (r.question_text || r.item?.question_text) return true;
  return false;
}

/** 从 bubble 中提取知识点名 */
function getKnowledgePoint(bubble) {
  if (bubble.params?.knowledge_point) return bubble.params.knowledge_point;
  const r = bubble.result;
  if (!r) return "";
  if (r.knowledge_point) return r.knowledge_point;
  if (r.item?.knowledge_point) return r.item.knowledge_point;
  return "";
}

/** 提取练习题 item（兼容 item 包裹和直接字段） */
function getQuestionItem(bubble) {
  const r = bubble.result;
  if (!r) return null;
  if (r.item) return r.item;
  if (r.question_text) return r;
  return null;
}

/** 通用工具结果 → Markdown / JSON 文本，供气泡回退渲染。 */
function formatToolResultText(result) {
  if (result == null) return "";
  if (typeof result === "string") return result;
  try {
    return "```json\n" + JSON.stringify(result, null, 2) + "\n```";
  } catch {
    return String(result);
  }
}

/** 通用工具结果 → 一行摘要。 */
function toolResultSummary(result) {
  if (!result) return null;
  if (typeof result === "string") return result.slice(0, 80);
  if (Array.isArray(result)) return `${result.length} 条`;
  const keys = Object.keys(result);
  if (keys.length === 1) return `${keys[0]}: ${JSON.stringify(result[keys[0]]).slice(0, 60)}`;
  return keys.slice(0, 3).join(" · ");
}
</script>

<template>
  <div class="bubble-row" :class="bubble.type">
    <details v-if="bubble.type === 'reasoning'" class="reasoning">
      <summary>
        <Icon icon="mdi:brain" width="18" />
        思考过程（点击折叠）
      </summary>
      <div class="reasoning-body">{{ bubble.text }}</div>
    </details>

    <div
      v-else
      class="bubble"
      :class="[bubble.type, { streaming }]"
      :data-message-id="bubble.messageId"
    >
      <div
        v-if="bubble.type === 'assistant'"
        class="md-body"
        v-html-smiles="bubble.text"
      ></div>
      <div v-else-if="bubble.type === 'system'" class="md-body system-note">{{ bubble.text }}</div>
      <div v-else-if="bubble.type === 'tool'" class="tool-card">
        <div class="tool-card-head">
          <Icon v-if="bubble.toolIcon" :icon="bubble.toolIcon" width="18" />
          <span class="tool-entry">{{ bubble.title || bubble.entry }}</span>
          <span
            class="tool-result"
            :class="{ ok: bubble.toolOk, fail: bubble.toolOk === false }"
          >
            {{ bubble.toolOk === true ? "完成" : bubble.toolOk === false ? "失败" : "进行中" }}
          </span>
        </div>

        <!-- 工具结果交互式内容 -->
        <div v-if="bubble.result && bubble.toolOk !== false" class="tool-card-body">
          <!-- practice::gaps → 薄弱点列表 -->
          <WeakPointList
            v-if="isGapsResult(bubble) && extractWeakPoints(bubble.result).length"
            :points="extractWeakPoints(bubble.result)"
            @generate="(p) => emit('tool-interact', { action: 'generate', ...p })"
          />
          <!-- practice::gaps 结果为空 -->
          <p v-else-if="isGapsResult(bubble)" class="muted" style="text-align:center;padding:12px 0;">
            <Icon icon="mdi:emoticon-happy-outline" width="18" /> 近期没有发现薄弱知识点，继续保持！
          </p>

          <!-- practice::generate 成功 → 练习卡片 -->
          <PracticeQuestion
            v-else-if="isPracticeQuestionResult(bubble) && getQuestionItem(bubble)"
            :item="getQuestionItem(bubble)"
            :knowledge-point="getKnowledgePoint(bubble)"
            @practice-again="(p) => emit('tool-interact', { action: 'practice-again', ...p })"
          />
          <!-- practice::generate 未命中 → 展示后端 message -->
          <div
            v-else-if="isPracticeQuestionResult(bubble)"
            class="tool-unmatched"
          >
            <Icon icon="mdi:information-outline" width="18" />
            <span>{{ bubble.result.message || "暂未找到合适的题目，换个知识点试试。" }}</span>
          </div>

          <!-- 其他工具：通用回退（折叠 JSON / Markdown） -->
          <details v-else class="tool-card-detail" :open="true">
            <summary>{{ toolResultSummary(bubble.result) }}</summary>
            <div class="tool-result-md md-body" v-html-smiles="formatToolResultText(bubble.result)"></div>
          </details>
        </div>
      </div>
      <template v-else>
        <template v-if="editing">
          <textarea
            v-model="editText"
            class="edit-inline"
            rows="3"
            aria-label="编辑消息内容"
            @keydown.esc="emit('cancel-edit')"
            @keydown.ctrl.enter="saveEdit()"
          ></textarea>
          <div class="edit-actions">
            <button class="btn ghost" @click="emit('cancel-edit')">取消</button>
            <button class="btn primary" :disabled="!editText.trim()" @click="saveEdit">
              保存并派生新分支
            </button>
          </div>
        </template>
        <template v-else>
          <Icon v-if="bubble.toolIcon" :icon="bubble.toolIcon" width="16" class="user-tool-icon" />
          <span>{{ bubble.text }}</span>
          <div v-if="bubble.attachments?.length" class="bubble-attachments">
            <button
              v-for="att in bubble.attachments"
              :key="att.path"
              class="attachment-chip"
              :aria-label="`查看附件 ${att.name}`"
              :title="att.name"
              @click="emit('open-attachment', att)"
            >
              <Icon :icon="attachmentIcon(att)" width="16" />
              <span class="attachment-chip-name">{{ att.name }}</span>
            </button>
          </div>
        </template>
      </template>
    </div>

    <div class="bubble-actions">
      <button
        v-if="bubble.type === 'user' && bubble.messageId && !editing"
        class="icon-btn"
        aria-label="编辑这条消息"
        title="编辑"
        @click="emit('edit', bubble)"
      >
        <Icon icon="mdi:pencil-outline" />
      </button>
      <span v-if="bubble.versionCount > 1" class="version-nav">
        <button
          class="icon-btn"
          aria-label="上一个版本"
          title="查看上一个版本"
          @click="emit('switch-branch', bubble, -1)"
        >
          <Icon icon="mdi:chevron-left" />
        </button>
        <span class="version-count" aria-hidden="true">
          {{ bubble.versionIndex + 1 }}/{{ bubble.versionCount }}
        </span>
        <button
          class="icon-btn"
          aria-label="下一个版本"
          title="查看下一个版本"
          @click="emit('switch-branch', bubble, 1)"
        >
          <Icon icon="mdi:chevron-right" />
        </button>
      </span>
      <button
        v-if="bubble.type === 'assistant'"
        class="icon-btn"
        aria-label="复制回答"
        title="复制"
        @click="emit('copy', bubble.text)"
      >
        <Icon icon="mdi:content-copy" />
      </button>
    </div>
  </div>
</template>
