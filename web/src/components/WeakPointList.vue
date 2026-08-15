<script setup>
import { Icon } from "@iconify/vue";

const props = defineProps({
  points: { type: Array, required: true },
});

const emit = defineEmits(["generate"]);

const DIFF = {
  basic:    { label: "基础补漏", icon: "mdi:stairs-up",        color: "var(--color-success-ink)" },
  variant:  { label: "同类变式", icon: "mdi:swap-horizontal",   color: "var(--color-accent-deep)" },
  advanced: { label: "综合拔高", icon: "mdi:trending-up",       color: "var(--color-warn-ink)" },
  exam:     { label: "高考真题", icon: "mdi:school",            color: "var(--color-danger-ink)" },
};

function diffMeta(d) { return DIFF[d] || DIFF.basic; }

function doGenerate(point) {
  emit("generate", {
    knowledge_point: point.knowledge_point,
    difficulty: point.suggested_difficulty || "basic",
  });
}
</script>

<template>
  <div class="weak-point-list">
    <div class="wpl-header">
      <Icon icon="mdi:target" width="18" />
      <span>{{ points.length }} 个薄弱知识点</span>
      <span class="wpl-hint">点击卡片开始针对性练习</span>
    </div>
    <div class="wpl-grid">
      <button
        v-for="(p, i) in points"
        :key="i"
        class="wpl-card"
        @click="doGenerate(p)"
      >
        <div class="wpl-card-top">
          <Icon
            :icon="diffMeta(p.suggested_difficulty).icon"
            width="18"
            :style="{ color: diffMeta(p.suggested_difficulty).color }"
          />
          <span class="wpl-point-name">{{ p.knowledge_point }}</span>
        </div>
        <div class="wpl-card-bottom">
          <span class="badge danger">{{ p.error_count }} 次错误</span>
          <span
            class="badge"
            :style="{ borderColor: diffMeta(p.suggested_difficulty).color, color: diffMeta(p.suggested_difficulty).color }"
          >{{ diffMeta(p.suggested_difficulty).label }}</span>
        </div>
      </button>
    </div>
  </div>
</template>
