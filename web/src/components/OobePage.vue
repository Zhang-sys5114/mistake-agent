<script setup>
import { computed, reactive, ref } from "vue";
import { Icon } from "@iconify/vue";

const props = defineProps({
  kernel: { type: Object, required: true },
});
const emit = defineEmits(["done"]);

const step = ref(0);
const saving = ref(false);
const testing = ref(false);
const error = ref("");
const notice = ref("");
const testResult = ref(null);

const form = reactive({
  log_level: "info",
  main: { api_url: "", model: "", transport: "responses", api_key: "", key_set: false },
  vision: { api_url: "", model: "", transport: "", api_key: "", key_set: false },
});

const steps = ["欢迎", "主模型", "视觉模型", "完成"];

async function load() {
  try {
    const v = await props.kernel.call("get_settings", {}, 10000);
    form.log_level = v.log_level || "info";
    form.main.api_url = v.main_model?.api_url || "";
    form.main.model = v.main_model?.model || "";
    form.main.transport = v.main_model?.transport || "responses";
    form.main.key_set = Boolean(v.main_model?.key_set);
    form.vision.api_url = v.vision_model?.api_url || "";
    form.vision.model = v.vision_model?.model || "";
    form.vision.transport = v.vision_model?.transport || "";
    form.vision.key_set = Boolean(v.vision_model?.key_set);
  } catch (e) {
    error.value = `读取设置失败：${e.message}`;
  }
}

/** 校验当前页字段，通过才允许下一步。 */
function validateStep(stepIndex) {
  error.value = "";
  if (stepIndex === 1) {
    if (!/^https?:\/\//.test(form.main.api_url.trim())) {
      error.value = "请填写主模型 API 地址（https:// 开头）";
      return false;
    }
    if (!form.main.model.trim()) {
      error.value = "请填写主模型 ID";
      return false;
    }
    if (!form.main.key_set && !form.main.api_key.trim()) {
      error.value = "请填写主模型 API Key（或保留已配置的密钥）";
      return false;
    }
  }
  if (stepIndex === 2) {
    if (!/^https?:\/\//.test(form.vision.api_url.trim())) {
      error.value = "请填写视觉模型 API 地址（https:// 开头）";
      return false;
    }
    if (!form.vision.model.trim()) {
      error.value = "请填写视觉模型 ID";
      return false;
    }
    if (!form.vision.key_set && !form.vision.api_key.trim()) {
      error.value = "请填写视觉模型 API Key（或保留已配置的密钥）";
      return false;
    }
  }
  return true;
}

async function next() {
  if (!validateStep(step.value)) return;
  // 每走一步自动测对应模型的连通性（失败不阻止，结果就地展示）。
  if (step.value === 1) await testConnection("main");
  if (step.value === 2) await testConnection("vision");
  step.value += 1;
}

async function save({ skipDone = false } = {}) {
  saving.value = true;
  error.value = "";
  notice.value = "";
  // 数据根目录与 AGENTS.md 由 kernel 引导初始化（bootstrap::init_data_root），前端不直接写文件系统。
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
    form.main.key_set = Boolean(v.main_model?.key_set);
    form.vision.key_set = Boolean(v.vision_model?.key_set);
    const ready = form.main.key_set && form.vision.key_set;
    if (ready && !skipDone) {
      emit("done");
    }
    return true;
  } catch (e) {
    error.value = `保存失败：${e.message}`;
    return false;
  } finally {
    saving.value = false;
  }
}

async function testConnection(model = "main") {
  const isVision = model === "vision";
  const cfg = isVision ? form.vision : form.main;
  error.value = "";
  testResult.value = null;
  if (!cfg.key_set && !cfg.api_key.trim()) {
    error.value = `请先填写${isVision ? "视觉" : "主"}模型 API Key`;
    return false;
  }
  testing.value = true;
  try {
    // 直接把表单里的 key 带给后端做一次临时测试（不落盘），
    // 不依赖"先保存再测试"的顺序——前端填什么，后端就测什么。
    const r = await props.kernel.call(
      "test_connection",
      { api_key: cfg.api_key, model: isVision ? "vision" : undefined },
      30000,
    );
    testResult.value = { ok: true, latency: r.latency_ms, model };
  } catch (e) {
    testResult.value = { ok: false, error: e.message, model };
  } finally {
    testing.value = false;
  }
  return Boolean(testResult.value?.ok);
}

async function finish() {
  error.value = "";
  if (!form.main.key_set && !form.main.api_key.trim()) {
    error.value = "主模型还没配置密钥，请返回填写。";
    step.value = 1;
    return;
  }
  if (!form.vision.key_set && !form.vision.api_key.trim()) {
    error.value = "视觉模型还没配置密钥，请返回填写。";
    step.value = 2;
    return;
  }
  await save();
}

const summary = computed(() => ({
  main: {
    label: "主模型",
    apiUrl: form.main.api_url,
    model: form.main.model || "deepseek-v4-flash",
    transport: form.main.transport === "chat_completions" ? "Chat Completions" : "Responses API",
    keySet: form.main.key_set || Boolean(form.main.api_key),
  },
  vision: {
    label: "视觉模型",
    apiUrl: form.vision.api_url,
    model: form.vision.model || "Qwen/Qwen3-VL-32B-Instruct",
    transport: "Chat Completions",
    keySet: form.vision.key_set || Boolean(form.vision.api_key),
  },
}));

load();
</script>

<template>
  <div class="oobe-overlay">
    <section class="oobe-card oobe-wizard" role="dialog" aria-modal="true" aria-label="首次配置向导">
      <aside class="oobe-steps" aria-label="配置步骤">
        <div
          v-for="(s, i) in steps"
          :key="s"
          class="oobe-step"
          :class="{ active: step === i, done: i < step }"
        >
          <span class="oobe-step-num">
            <Icon v-if="i < step" icon="mdi:check" width="14" />
            <template v-else>{{ i + 1 }}</template>
          </span>
          <span>{{ s }}</span>
        </div>
      </aside>

      <div class="oobe-body">
        <Transition name="fade" mode="out-in">
          <div v-if="step === 0" key="welcome" class="oobe-page oobe-welcome">
            <span class="brand-mark">
              <Icon icon="mdi:book-education-outline" width="28" />
            </span>
            <h1>欢迎使用错题 Agent</h1>
            <p>上传作业照片或 PDF，自动批改、归档错题、生成练习。下面花一分钟配置两个模型，密钥只保存在本机。</p>
          </div>

          <form v-else-if="step === 1" key="main" class="oobe-page" @submit.prevent="next">
            <h2>配置主模型</h2>
            <p class="oobe-tip">负责对话、调度与判分（DeepSeek）。</p>
            <div v-if="testResult && testResult.model === 'main'" class="alert" :class="{ success: testResult.ok }" role="status">
              <Icon :icon="testResult.ok ? 'mdi:check-circle-outline' : 'mdi:alert-circle-outline'" width="18" />
              <span v-if="testResult.ok">主模型连接成功（{{ testResult.latency }}ms）</span>
              <span v-else>主模型连接失败：{{ testResult.error }}</span>
            </div>
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
              <input
                v-model="form.main.api_key"
                type="password"
                autocomplete="off"
                :required="!form.main.key_set"
                :placeholder="form.main.key_set ? '已配置（留空表示不修改）' : '粘贴 DeepSeek API Key'"
              />
            </label>
          </form>

          <form v-else-if="step === 2" key="vision" class="oobe-page" @submit.prevent="next">
            <h2>配置视觉模型</h2>
            <p class="oobe-tip">负责识别作业图片与手写内容（SiliconFlow）。</p>
            <div v-if="testResult && testResult.model === 'vision'" class="alert" :class="{ success: testResult.ok }" role="status">
              <Icon :icon="testResult.ok ? 'mdi:check-circle-outline' : 'mdi:alert-circle-outline'" width="18" />
              <span v-if="testResult.ok">视觉模型连接成功（{{ testResult.latency }}ms）</span>
              <span v-else>视觉模型连接失败：{{ testResult.error }}</span>
            </div>
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
              <input
                v-model="form.vision.api_key"
                type="password"
                autocomplete="off"
                :required="!form.vision.key_set"
                :placeholder="form.vision.key_set ? '已配置（留空表示不修改）' : '粘贴 SiliconFlow API Key'"
              />
            </label>
          </form>

          <div v-else key="done" class="oobe-page">
            <h2>完成</h2>
            <p class="oobe-tip">确认配置后测试连接，然后进入应用。</p>
            <div class="oobe-summary">
              <div v-for="item in [summary.main, summary.vision]" :key="item.label" class="card oobe-summary-card">
                <div class="oobe-summary-head">
                  <span class="section-icon"><Icon :icon="item.label === '主模型' ? 'mdi:robot-outline' : 'mdi:image-search-outline'" width="18" /></span>
                  <strong>{{ item.label }}</strong>
                  <span class="badge" :class="{ success: item.keySet }">{{ item.keySet ? "密钥已配置" : "缺少密钥" }}</span>
                </div>
                <dl class="oobe-summary-grid">
                  <dt>地址</dt><dd>{{ item.apiUrl }}</dd>
                  <dt>模型</dt><dd>{{ item.model }}</dd>
                  <dt>接入</dt><dd>{{ item.transport }}</dd>
                </dl>
              </div>
            </div>
            <div v-if="testResult" class="alert" :class="{ success: testResult.ok }" role="status">
              <Icon :icon="testResult.ok ? 'mdi:check-circle-outline' : 'mdi:alert-circle-outline'" width="18" />
              <span v-if="testResult.ok">主模型连接成功（{{ testResult.latency }}ms）</span>
              <span v-else>主模型连接失败：{{ testResult.error }}</span>
            </div>
          </div>
        </Transition>

        <p v-if="error" class="alert" role="alert">
          <Icon icon="mdi:alert-circle-outline" width="18" />{{ error }}
        </p>
        <p v-if="notice" class="alert success" role="status">
          <Icon icon="mdi:check-circle-outline" width="18" />{{ notice }}
        </p>
      </div>

      <footer class="oobe-foot">
        <button v-if="step > 0 && step < 3" class="btn ghost" @click="step -= 1">
          <Icon icon="mdi:arrow-left" width="18" />上一步
        </button>
        <span class="oobe-foot-spacer"></span>
        <button v-if="step < 3" class="btn primary" @click="next">
          下一步<Icon icon="mdi:arrow-right" width="18" />
        </button>
        <template v-else>
          <button class="btn ghost" :disabled="testing || saving" @click="testConnection('main')">
            <Icon :icon="testing ? 'mdi:loading' : 'mdi:connection'" :class="{ spin: testing }" width="18" />
            {{ testing ? "测试中…" : "测试连接" }}
          </button>
          <button class="btn primary" :disabled="saving" @click="finish">
            <Icon icon="mdi:check" width="18" />{{ saving ? "保存中…" : "完成" }}
          </button>
        </template>
      </footer>
    </section>
  </div>
</template>
