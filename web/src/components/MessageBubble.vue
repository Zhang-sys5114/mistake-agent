<script setup>
import { ref, watch } from "vue";
import { Icon } from "@iconify/vue";
import { renderMarkdown } from "../lib/markdown";
import { attachmentUrl } from "../lib/attachments";

defineProps({
  bubble: { type: Object, required: true },
  streaming: { type: Boolean, default: false },
});

const emit = defineEmits(["edit", "switch-branch", "copy", "open-attachment"]);

const attachmentData = ref(null);
const attachmentLoading = ref(false);
const attachmentError = ref("");

async function loadAttachment() {
  const a = props.bubble?.attachment;
  attachmentData.value = null;
  attachmentError.value = "";
  attachmentLoading.value = !!a;
  if (!a) return;
  try {
    attachmentData.value = await attachmentUrl(a.path, a.name);
  } catch (e) {
    attachmentError.value = String(e?.message || e);
  } finally {
    attachmentLoading.value = false;
  }
}

watch(() => props.bubble?.attachment, loadAttachment, { immediate: true });
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
      <div v-if="bubble.type === 'assistant'" class="md-body" v-html="renderMarkdown(bubble.text)"></div>
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
        <details
          v-if="bubble.params || bubble.result"
          class="tool-card-detail"
        >
          <summary>查看详情{{ bubble.entry ? `（${bubble.entry}）` : "" }}</summary>
          <pre class="tool-card-body">{{
            JSON.stringify(
              {
                params: bubble.params,
                result: bubble.result,
              },
              null,
              2,
            )
          }}</pre>
        </details>
      </div>
      <template v-else>
        <Icon v-if="bubble.toolIcon" :icon="bubble.toolIcon" width="16" class="user-tool-icon" />
        <span>{{ bubble.text }}</span>
        <button
          v-if="bubble.attachment"
          class="attachment"
          :aria-label="`查看附件 ${bubble.attachment.name}`"
          @click="
            attachmentError
              ? loadAttachment()
              : emit('open-attachment', bubble.attachment)
          "
        >
          <template v-if="attachmentLoading">
            <Icon icon="mdi:loading" width="18" class="spin" />
            <span class="attachment-name">正在加载…</span>
          </template>
          <template v-else-if="attachmentError">
            <Icon icon="mdi:file-alert-outline" width="18" />
            <span class="attachment-name">附件加载失败，点击重试</span>
          </template>
          <img
            v-else-if="attachmentData?.kind === 'image'"
            class="attachment-thumb"
            :src="attachmentData.url"
            alt="作业图片"
          />
          <template v-else-if="attachmentData?.kind === 'pdf'">
            <Icon icon="mdi:file-pdf-box" width="26" />
            <span class="attachment-name">{{ bubble.attachment.name }}</span>
          </template>
        </button>
      </template>
    </div>

    <div class="bubble-actions">
      <button
        v-if="bubble.type === 'user' && bubble.messageId"
        class="icon-btn"
        aria-label="编辑这条消息"
        title="编辑"
        @click="emit('edit', bubble)"
      >
        <Icon icon="mdi:pencil-outline" />
      </button>
      <button
        v-if="bubble.siblingIds && bubble.siblingIds.length"
        class="icon-btn"
        aria-label="切换分支"
        title="查看其它分支"
        @click="emit('switch-branch', bubble)"
      >
        <Icon icon="mdi:source-branch" />
      </button>
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
