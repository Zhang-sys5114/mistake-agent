<script setup>
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { Icon } from "@iconify/vue";
import { invoke } from "@tauri-apps/api/core";
import { attachmentUrl } from "../lib/attachments";

const props = defineProps({
  attachment: { type: Object, required: true },
});
const emit = defineEmits(["close"]);

const isPdf = computed(() =>
  /\.pdf$/i.test(props.attachment.name || props.attachment.path),
);
const data = ref(null);
const loading = ref(true);
const error = ref("");
const openError = ref("");

async function load() {
  loading.value = true;
  error.value = "";
  data.value = null;
  try {
    data.value = await attachmentUrl(
      props.attachment.path,
      props.attachment.name,
    );
  } catch (e) {
    error.value = String(e?.message || e);
  } finally {
    loading.value = false;
  }
}

watch(() => props.attachment, load, { immediate: true });

async function openWithSystem() {
  openError.value = "";
  try {
    await invoke("open_attachment", { path: props.attachment.path });
  } catch (e) {
    openError.value = String(e?.message || e);
  }
}

function onKeydown(e) {
  if (e.key === "Escape") emit("close");
}
onMounted(() => window.addEventListener("keydown", onKeydown));
onUnmounted(() => window.removeEventListener("keydown", onKeydown));
</script>

<template>
  <div class="viewer-overlay" @click.self="emit('close')">
    <div class="viewer-card" role="dialog" aria-modal="true" aria-label="附件预览">
      <header class="viewer-head">
        <span class="viewer-name">
          <Icon :icon="isPdf ? 'mdi:file-pdf-box' : 'mdi:image' " width="18" />
          {{ attachment.name }}
        </span>
        <button class="icon-btn" aria-label="关闭预览" @click="emit('close')">
          <Icon icon="mdi:close" width="20" />
        </button>
      </header>
      <div v-if="loading" class="viewer-empty">
        <Icon icon="mdi:loading" width="28" class="spin" />
        <p>正在打开…</p>
      </div>
      <div v-else-if="error" class="viewer-empty">
        <Icon icon="mdi:file-alert-outline" width="32" />
        <p>打不开这个附件：{{ error }}</p>
        <button class="btn ghost" @click="load">重试</button>
      </div>
      <img
        v-else-if="!isPdf && data"
        class="viewer-img"
        :src="data.url"
        alt="作业大图"
      />
      <iframe
        v-else-if="isPdf && data"
        class="viewer-pdf"
        :src="data.url"
        title="PDF 预览"
      ></iframe>
      <footer v-if="isPdf && data" class="viewer-foot">
        <p v-if="openError" class="muted">{{ openError }}</p>
        <button class="btn ghost" @click="openWithSystem">
          <Icon icon="mdi:open-in-new" width="18" />打不开？用系统程序打开
        </button>
      </footer>
    </div>
  </div>
</template>
