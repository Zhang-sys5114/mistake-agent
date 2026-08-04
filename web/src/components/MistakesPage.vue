<script setup>
import { computed, onMounted, ref } from "vue";
import { Icon } from "@iconify/vue";
import { renderMarkdown } from "../lib/markdown";

const props = defineProps({ kernel: { type: Object, required: true } });

const loading = ref(false);
const error = ref("");
const mistakes = ref([]);
const subjects = ref([]);
const subject = ref("");
const search = ref("");
const sortBy = ref("time_desc");

const total = ref(0);
const wrong = ref(0);

/** 搜索：题目/知识点/错因/学生作答 文本模糊过滤。 */
const filtered = computed(() => {
  const q = search.value.trim().toLowerCase();
  let list = mistakes.value;
  if (q) {
    list = list.filter((m) =>
      [m.question, m.knowledge_point, m.analysis, m.student_answer, m.reference_answer]
        .filter(Boolean)
        .some((t) => String(t).toLowerCase().includes(q)),
    );
  }
  const sorted = [...list].sort((a, b) => {
    if (sortBy.value === "subject") {
      return (a.subject || "").localeCompare(b.subject || "", "zh");
    }
    return new Date(b.created_at) - new Date(a.created_at);
  });
  return sorted;
});

async function load() {
  loading.value = true;
  error.value = "";
  try {
    const r = await props.kernel.triggerCommand(
      "grading::list",
      subject.value ? { subject: subject.value } : {},
    );
    mistakes.value = r.mistakes || [];
    total.value = r.count || mistakes.value.length;
    wrong.value = mistakes.value.filter((m) => !m.is_correct).length;
    subjects.value = [...new Set(mistakes.value.map((m) => m.subject).filter(Boolean))].sort();
  } catch (e) {
    error.value =
      e.code === "not_implemented"
        ? "错题本命令尚未接通，请稍后再试"
        : `加载失败：${e.message}`;
    mistakes.value = [];
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

onMounted(load);
</script>

<template>
  <div class="page">
    <div class="page-head">
      <h2>错题本</h2>
      <button class="btn ghost" :disabled="loading" @click="load">
        <Icon icon="mdi:refresh" width="18" />刷新
      </button>
    </div>

    <div v-if="!loading && mistakes.length" class="stat-strip">
      <div class="card stat-card">
        <span class="stat-icon"><Icon icon="mdi:notebook-outline" width="20" /></span>
        <div>
          <div class="stat-value">{{ total }}</div>
          <div class="stat-label">错题总数</div>
        </div>
      </div>
      <div class="card stat-card">
        <span class="stat-icon"><Icon icon="mdi:alert-circle-outline" width="20" /></span>
        <div>
          <div class="stat-value">{{ wrong }}</div>
          <div class="stat-label">未掌握</div>
        </div>
      </div>
      <div class="card stat-card">
        <span class="stat-icon"><Icon icon="mdi:book-open-variant" width="20" /></span>
        <div>
          <div class="stat-value">{{ subjects.length }}</div>
          <div class="stat-label">涉及学科</div>
        </div>
      </div>
    </div>

    <div v-if="subjects.length" class="chips" aria-label="按学科筛选">
      <button class="chip" :class="{ active: !subject }" @click="subject = ''; load()">全部</button>
      <button
        v-for="s in subjects"
        :key="s"
        class="chip"
        :class="{ active: subject === s }"
        @click="subject = s; load()"
      >
        {{ s }}
      </button>
    </div>

    <div class="mistake-tools">
      <input
        v-model="search"
        class="mistake-search"
        type="search"
        placeholder="搜索题目、知识点或错因…"
        aria-label="搜索错题"
      />
      <select v-model="sortBy" class="mistake-sort" aria-label="排序方式">
        <option value="time_desc">最新在前</option>
        <option value="subject">按学科</option>
      </select>
    </div>

    <p v-if="error" class="alert" role="alert">
      <Icon icon="mdi:alert-circle-outline" width="18" />{{ error }}
    </p>

    <div v-if="loading" class="empty">
      <Icon icon="mdi:loading" width="28" class="spin" />
      <p>正在读取错题本…</p>
    </div>

    <div v-else-if="!mistakes.length" class="empty">
      <Icon icon="mdi:notebook-outline" width="36" />
      <p>还没有错题。上传一份作业让 Agent 批改，错题会自动归档到这里。</p>
    </div>

    <div v-else-if="!filtered.length" class="empty">
      <Icon icon="mdi:file-search-outline" width="36" />
      <p>没有匹配的错题，换个关键词试试。</p>
    </div>

    <div v-else class="mistake-grid">
      <article v-for="m in filtered" :key="String(m.id)" class="card mistake-card">
        <div class="card-head">
          <span class="badge">{{ m.subject || "未分类" }}</span>
          <span class="badge weak">{{ m.knowledge_point || "未标注知识点" }}</span>
          <time class="muted">{{ formatTime(m.created_at) }}</time>
        </div>
        <div class="md-body mistake-question" v-html="renderMarkdown(m.question)"></div>
        <dl class="answer-grid">
          <div v-if="m.student_answer">
            <dt><Icon icon="mdi:pencil-outline" width="13" /> 学生作答</dt>
            <dd>{{ m.student_answer }}</dd>
          </div>
          <div v-if="m.reference_answer">
            <dt><Icon icon="mdi:check-decagram-outline" width="13" /> 参考答案</dt>
            <dd class="md-body" v-html="renderMarkdown(m.reference_answer)"></dd>
          </div>
          <div v-if="m.analysis">
            <dt><Icon icon="mdi:lightbulb-on-outline" width="13" /> 错因分析</dt>
            <dd class="md-body" v-html="renderMarkdown(m.analysis)"></dd>
          </div>
        </dl>
      </article>
    </div>
  </div>
</template>
