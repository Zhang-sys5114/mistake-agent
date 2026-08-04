<script setup>
import { reactive, ref } from "vue";
import { Icon } from "@iconify/vue";

const props = defineProps({
  kernel: { type: Object, required: true },
});
const emit = defineEmits(["done"]);

const saving = ref(false);
const testing = ref(false);
const error = ref("");
const notice = ref("");
const testResult = ref(null);

const form = reactive({
  log_level: "info",
  main: { api_url: "", model: "", transport: "responses", api_key: "" },
  vision: { api_url: "", model: "", transport: "", api_key: "" },
});

async function load() {
  try {
    const v = await props.kernel.call("get_settings", {}, 10000);
    form.log_level = v.log_level || "info";
    form.main.api_url = v.main_model?.api_url || "";
    form.main.model = v.main_model?.model || "";
    form.main.transport = v.main_model?.transport || "responses";
    form.vision.api_url = v.vision_model?.api_url || "";
    form.vision.model = v.vision_model?.model || "";
    form.vision.transport = v.vision_model?.transport || "";
  } catch (e) {
    error.value = `读取设置失败：${e.message}`;
  }
}

async function testConnection() {
  testing.value = true;
  testResult.value = null;
  error.value = "";
  try {
    const r = await props.kernel.call("test_connection", {}, 30000);
    testResult.value = { ok: true, latency: r.latency_ms };
  } catch (e) {
    testResult.value = { ok: false, error: e.message };
  } finally {
    testing.value = false;
  }
}

async function save() {
  saving.value = true;
  error.value = "";
  notice.value = "";
  const patch = {
    log_level: form.log_level,
    main_model: {
      api_url: form.main.api_url.trim(),
      api_key: form.main.api_key,
      model: form.main.model.trim(),
      transport: form.main.transport || null,
    },
    vision_model: {
      api_url: form.vision.api_url.trim(),
      api_key: form.vision.api_key,
      model: form.vision.model.trim(),
      transport: form.vision.transport || null,
    },
  };
  try {
    const v = await props.kernel.call("set_settings", { patch }, 15000);
    form.main.api_key = "";
    form.vision.api_key = "";
    const ready =
      Boolean(v.main_model?.key_set) && Boolean(v.vision_model?.key_set);
    if (ready) {
      emit("done");
    } else {
      notice.value = "已保存。请继续填写未配置的模型密钥。";
      await load();
    }
  } catch (e) {
    error.value = `保存失败：${e.message}`;
  } finally {
    saving.value = false;
  }
}

load();
</script>

<template>
  <div class="oobe-overlay">
    <section class="oobe-card" role="dialog" aria-modal="true" aria-label="首次配置">
      <div class="oobe-head">
        <span class="brand-mark">
          <Icon icon="mdi:book-education-outline" width="24" />
        </span>
        <h1>欢迎使用错题 Agent</h1>
        <p>先配置两个模型，之后就能上传作业自动批改。密钥只保存在本机。</p>
      </div>

      <p v-if="error" class="alert" role="alert">
        <Icon icon="mdi:alert-circle-outline" width="18" />{{ error }}
      </p>
      <p v-if="notice" class="alert success" role="status">
        <Icon icon="mdi:check-circle-outline" width="18" />{{ notice }}
      </p>
      <div v-if="testResult" class="alert" :class="{ success: testResult.ok }" role="status">
        <Icon :icon="testResult.ok ? 'mdi:check-circle-outline' : 'mdi:alert-circle-outline'" width="18" />
        <span v-if="testResult.ok">连接成功（{{ testResult.latency }}ms）</span>
        <span v-else>连接失败：{{ testResult.error }}</span>
      </div>

      <form class="settings-form oobe-form" @submit.prevent="save">
        <section class="card">
          <h3><span class="section-icon"><Icon icon="mdi:robot-outline" width="18" /></span>主模型（对话与调度）</h3>
          <label class="field">
            <span>API 地址</span>
            <input v-model="form.main.api_url" type="url" required placeholder="https://api.deepseek.com" />
          </label>
          <label class="field">
            <span>模型 ID</span>
            <input v-model="form.main.model" placeholder="deepseek-v4-flash" />
          </label>
          <label class="field">
            <span>接入方式</span>
            <select v-model="form.main.transport">
              <option value="responses">Responses API（DeepSeek 官方）</option>
              <option value="chat_completions">Chat Completions（OpenAI 兼容）</option>
            </select>
          </label>
          <label class="field">
            <span>API Key</span>
            <input v-model="form.main.api_key" type="password" autocomplete="off" required placeholder="粘贴 DeepSeek API Key" />
          </label>
        </section>

        <section class="card">
          <h3><span class="section-icon"><Icon icon="mdi:image-search-outline" width="18" /></span>视觉模型（识别作业图片）</h3>
          <label class="field">
            <span>API 地址</span>
            <input v-model="form.vision.api_url" type="url" required placeholder="https://api.siliconflow.cn/v1" />
          </label>
          <label class="field">
            <span>模型 ID</span>
            <input v-model="form.vision.model" placeholder="Qwen/Qwen3-VL-32B-Instruct" />
          </label>
          <label class="field">
            <span>API Key</span>
            <input v-model="form.vision.api_key" type="password" autocomplete="off" required placeholder="粘贴 SiliconFlow API Key" />
          </label>
        </section>

        <div class="oobe-actions">
          <button type="button" class="btn ghost" :disabled="testing" @click="testConnection">
            <Icon :icon="testing ? 'mdi:loading' : 'mdi:connection'" :class="{ spin: testing }" width="18" />
            {{ testing ? "测试中…" : "测试连接" }}
          </button>
          <button type="submit" class="btn primary" :disabled="saving">
            <Icon icon="mdi:check" width="18" />{{ saving ? "保存中…" : "完成配置" }}
          </button>
        </div>
      </form>
    </section>
  </div>
</template>
