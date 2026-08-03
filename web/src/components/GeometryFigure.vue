<script setup>
import { computed } from "vue";
import { renderGeometry } from "../lib/geometry.js";

const props = defineProps({
  spec: { type: Object, required: true },
});

const svg = computed(() => {
  try {
    return renderGeometry(props.spec).svg;
  } catch (e) {
    return `<text x="0" y="0" font-size="0.5" fill="#e5484d">图形规格错误：${e.message}</text>`;
  }
});
</script>

<template>
  <div class="geometry-figure" v-html="svg"></div>
</template>
