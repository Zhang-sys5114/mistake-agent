<script setup>
import { computed, inject, ref } from "vue";
import { Icon } from "@iconify/vue";
import GeometryFigure from "./GeometryFigure.vue";

const props = defineProps({
  item: { type: Object, required: true },
  knowledgePoint: { type: String, default: "" },
});

const emit = defineEmits(["practice-again"]);

const kernel = inject("kernel");

const userAnswer = ref("");
const checking = ref(false);
const checkResult = ref(null);

const answerRevealed = computed(() => checkResult.value !== null);
const generatedDifficulty = computed(() => props.item.difficulty || "basic");

const DIFFICULTY = {
  basic:    { label: "基础补漏", color: "var(--color-success-ink)" },
  variant:  { label: "同类变式", color: "var(--color-accent-deep)" },
  advanced: { label: "综合拔高", color: "var(--color-warn-ink)" },
  exam:     { label: "高考真题", color: "var(--color-danger-ink)" },
};

function diffMeta(d) {
  return DIFFICULTY[d] || DIFFICULTY.basic;
}

async function submitAnswer() {
  const a = userAnswer.value.trim();
  if (!a || checking.value) return;
  checking.value = true;
  try {
    const result = await kernel.triggerCommand("practice::check", {
      question:         props.item.question_text,
      student_answer:   a,
      reference_answer: props.item.answer_spec,
      knowledge_point:  props.knowledgePoint,
      item_id:          props.item.template_id,
      difficulty:       generatedDifficulty.value,
    });
    checkResult.value = result;
  } catch (e) {
    checkResult.value = {
      correct: false,
      method:  "error",
      analysis: `批改失败：${e.message || e}`,
    };
  } finally {
    checking.value = false;
  }
}

function retrySame() {
  emit("practice-again", {
    samePoint: true,
    knowledge_point: props.knowledgePoint,
    difficulty: generatedDifficulty.value,
  });
}
function retryDiff() {
  emit("practice-again", {
    samePoint: false,
    knowledge_point: props.knowledgePoint,
  });
}
</script>

<template>
  <div class="practice-question">
    <!-- ── 题目头部 ── -->
    <div class="pq-header">
      <span
        class="badge"
        :style="{ borderColor: diffMeta(generatedDifficulty).color, color: diffMeta(generatedDifficulty).color }"
      >{{ diffMeta(generatedDifficulty).label }}</span>
      <span v-if="item.source" class="badge weak">{{ item.source }}</span>
      <span class="pq-id muted">#{{ item.template_id }}</span>
    </div>

    <!-- ── 题干 (KaTeX) ── -->
    <div class="pq-question md-body" v-html-smiles="item.question_text"></div>

    <!-- ── 几何图 ── -->
    <GeometryFigure v-if="item.diagram_spec" :spec="item.diagram_spec" />

    <!-- ── 作答区（批改前） ── -->
    <div v-if="!answerRevealed" class="pq-answer-area">
      <textarea
        v-model="userAnswer"
        class="pq-answer-input"
        rows="3"
        placeholder="输入你的答案…（支持 Markdown / KaTeX）"
        :disabled="checking"
      ></textarea>
      <button
        class="btn primary pq-submit"
        :disabled="!userAnswer.trim() || checking"
        @click="submitAnswer"
      >
        <Icon v-if="checking" icon="mdi:loading" width="16" class="spin" />
        <Icon v-else icon="mdi:check-circle-outline" width="16" />
        {{ checking ? "批改中…" : "提交批改" }}
      </button>
    </div>

    <!-- ── 批改结果 ── -->
    <div v-if="answerRevealed" class="pq-result">
      <div class="pq-verdict" :class="{ correct: checkResult.correct, incorrect: !checkResult.correct }">
        <Icon :icon="checkResult.correct ? 'mdi:check-circle' : 'mdi:close-circle'" width="20" />
        <span>{{ checkResult.correct ? "回答正确！" : "回答有误" }}</span>
        <span v-if="checkResult.score != null" class="pq-score">
          {{ checkResult.score }}/{{ checkResult.total }}
        </span>
      </div>

      <!-- 参考答案 -->
      <div v-if="item.answer_spec" class="pq-ref-answer">
        <div class="pq-ref-label">
          <Icon icon="mdi:check-decagram-outline" width="16" /> 参考答案
        </div>
        <div class="md-body" v-html-smiles="item.answer_spec"></div>
      </div>

      <!-- 解析 -->
      <div v-if="checkResult.analysis" class="pq-analysis">
        <div class="pq-analysis-label">
          <Icon icon="mdi:lightbulb-on-outline" width="16" /> 解析
        </div>
        <div class="md-body" v-html-smiles="checkResult.analysis"></div>
      </div>

      <!-- 归档提示 -->
      <div v-if="checkResult.archived_mistake" class="pq-archived">
        <Icon icon="mdi:notebook-outline" width="14" /> 已归档错题本
      </div>

      <!-- 操作 -->
      <div class="pq-actions">
        <button class="btn ghost" @click="retrySame">
          <Icon icon="mdi:refresh" width="16" />再练一题（同知识点）
        </button>
        <button class="btn ghost" @click="retryDiff">
          <Icon icon="mdi:shuffle-variant" width="16" />换知识点
        </button>
      </div>
    </div>
  </div>
</template>
