<!-- 小问分段文本：错题详情页的「参考答案 / 错因分析」用。
     模型写进错题本的这两段文字常常没有任何换行，整段堆在一起观感差；
     这里按小问标记切成独立段落，用左竖线缩进块分行渲染（见 lib/segments.js）。
     识别不出小问时退回原来的整段渲染，行为与改动前一致。 -->
<script setup>
import { computed } from "vue";
import { splitSubQuestions } from "../lib/segments";

const props = defineProps({
  text: { type: String, default: "" },
});

const segments = computed(() => splitSubQuestions(props.text));
</script>

<template>
  <div v-if="!segments" class="md-body" v-html-smiles="text"></div>
  <div v-else class="sq-list md-body">
    <div v-for="(seg, i) in segments" :key="i" class="sq-item" v-html-smiles="seg"></div>
  </div>
</template>
