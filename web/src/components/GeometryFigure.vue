<script setup>
import { computed } from "vue";
import DOMPurify from "dompurify";
import { renderGeometry } from "../lib/geometry.js";

const props = defineProps({
  spec: { type: Object, required: true },
});

const svg = computed(() => {
  try {
    const { svg } = renderGeometry(props.spec);
    // 第二道防线：即使 renderGeometry 有漏，DOMPurify 也只放行 SVG 白名单。
    return DOMPurify.sanitize(svg, {
      USE_PROFILES: { svg: true, svgFilters: true },
      FORBID_TAGS: ["script", "foreignObject", "animate", "set"],
      FORBID_ATTR: [/^on/i],
    });
  } catch (e) {
    return DOMPurify.sanitize(
      `<text x="0" y="0" font-size="0.5" fill="#dc2626">图形规格错误</text>`,
      { USE_PROFILES: { svg: true } },
    );
  }
});
</script>

<template>
  <div class="geometry-figure" v-html="svg"></div>
</template>
