<script setup>
import { computed, inject, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { Icon } from "@iconify/vue";

const props = defineProps({ kernel: { type: Object, required: true } });
const navigateToChatWithMessage = inject("navigateToChatWithMessage", () => {});

const loading = ref(false);
const error = ref("");
const mistakes = ref([]);
const subjects = ref([]);
const subject = ref("");
const search = ref("");
const sortBy = ref("time_desc");

/* ──── 编辑模式 ──── */
const editMode = ref(false);
const selectedIds = ref(new Set());
const confirmDeleteCount = ref(0); // >0 时显示确认对话框

/* ──── 单题编辑弹窗 ──── */
const editingMistake = ref(null);
const editForm = ref({});

/* ──── 右键 / 长按菜单 ──── */
const contextMenu = ref({ visible: false, x: 0, y: 0, mistake: null });
let pointerTimer = null;
let pointerMoved = false;

const total = ref(0);
const wrong = ref(0);

/* -------- 抽屉 -------- */
const drawerIndex = ref(null); // null = 关闭；数字 = filtered 列表索引
const drawerItem = computed(() =>
  drawerIndex.value != null ? filtered.value[drawerIndex.value] : null,
);

function openDrawer(item) {
  const idx = filtered.value.indexOf(item);
  drawerIndex.value = idx >= 0 ? idx : 0;
}

/* -------- 备注（localStorage） -------- */
const LS_PREFIX = "mistake-note:";
const noteText = ref("");
const noteSaved = ref(false);  // 控制「✓ 已自动保存」淡入
let noteTimer = null;
let noteTa = null; // textarea DOM 引用

function noteKey(mistake) {
  const id = mistake?.id != null ? mistake.id : "";
  return LS_PREFIX + id;
}

function loadNote(mistake) {
  // 清掉上一个题目的待保存定时器，防止串到错误题号
  if (noteTimer) { clearTimeout(noteTimer); noteTimer = null; }
  if (!mistake) { noteText.value = ""; return; }
  noteText.value = localStorage.getItem(noteKey(mistake)) || "";
  noteSaved.value = false;
  nextTick(() => autoGrow());
}

// 翻题 / 新开抽屉时自动加载对应备注
watch(() => drawerItem.value, (item) => { loadNote(item); });

function setNoteTa(el) {
  noteTa = el;
  nextTick(() => autoGrow());
}

function autoGrow() {
  if (!noteTa) return;
  noteTa.style.height = "auto";
  noteTa.style.height = noteTa.scrollHeight + "px";
}

function onNoteInput() {
  // 保存到 localStorage（防抖 600ms）
  if (noteTimer) clearTimeout(noteTimer);
  noteTimer = setTimeout(() => {
    const m = drawerItem.value;
    if (!m) return;
    const key = noteKey(m);
    if (noteText.value.trim()) {
      localStorage.setItem(key, noteText.value);
    } else {
      localStorage.removeItem(key);
    }
    // 闪一下「已自动保存」
    noteSaved.value = true;
    setTimeout(() => { noteSaved.value = false; }, 1500);
  }, 600);
}

function closeDrawer() {
  drawerIndex.value = null;
}

function goPrev() {
  if (drawerIndex.value == null) return;
  const n = filtered.value.length;
  if (n <= 1) return;
  drawerIndex.value = (drawerIndex.value - 1 + n) % n;
}

function goNext() {
  if (drawerIndex.value == null) return;
  const n = filtered.value.length;
  if (n <= 1) return;
  drawerIndex.value = (drawerIndex.value + 1) % n;
}

function onKeydown(e) {
  if (drawerIndex.value == null) return;
  if (e.key === "Escape") {
    e.preventDefault();
    closeDrawer();
  } else if (e.key === "ArrowRight") {
    e.preventDefault();
    goNext();
  } else if (e.key === "ArrowLeft") {
    e.preventDefault();
    goPrev();
  }
}

watch(
  () => drawerIndex.value,
  (val) => {
    if (val != null) {
      document.addEventListener("keydown", onKeydown);
    } else {
      document.removeEventListener("keydown", onKeydown);
    }
  },
);

onBeforeUnmount(() => {
  document.removeEventListener("keydown", onKeydown);
  document.removeEventListener("keydown", onCtxKey);
  if (pointerTimer) clearTimeout(pointerTimer);
});

/* -------- 长文徽标 -------- */
function stripHtml(html) {
  if (!html) return "";
  const div = document.createElement("div");
  div.innerHTML = html;
  return div.textContent || "";
}

function wordCount(text) {
  if (!text) return 0;
  const plain = stripHtml(text);
  const chinese = (plain.match(/[一-鿿]/g) || []).length;
  const english = (plain.match(/[a-zA-Z]+/g) || []).length;
  return chinese + english;
}

function isLongText(text) {
  return wordCount(text) > 60;
}

/* -------- 搜索 / 排序 -------- */
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
    // 置顶优先
    if (a.pinned && !b.pinned) return -1;
    if (!a.pinned && b.pinned) return 1;
    if (sortBy.value === "subject") {
      return (a.subject || "").localeCompare(b.subject || "", "zh");
    }
    return new Date(b.created_at) - new Date(a.created_at);
  });
  return sorted;
});

/** 全选状态 */
const allSelected = computed(() => {
  if (!filtered.value.length) return false;
  return filtered.value.every((m) => selectedIds.value.has(String(m.id)));
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

/* ================================================================
 *  编辑模式
 * ================================================================ */
function toggleEditMode() {
  editMode.value = !editMode.value;
  if (!editMode.value) selectedIds.value = new Set();
}

function toggleSelect(id) {
  const s = new Set(selectedIds.value);
  if (s.has(id)) s.delete(id); else s.add(id);
  selectedIds.value = s;
}

function selectAll() {
  if (allSelected.value) {
    selectedIds.value = new Set();
  } else {
    selectedIds.value = new Set(filtered.value.map((m) => String(m.id)));
  }
}

async function batchDelete() {
  const ids = [...selectedIds.value];
  if (!ids.length) return;
  if (confirmDeleteCount.value === 0) {
    confirmDeleteCount.value = ids.length;
    return;
  }
  // 二次确认后执行
  try {
    const r = await props.kernel.triggerCommand("grading::remove_many", { ids });
    confirmDeleteCount.value = 0;
    selectedIds.value = new Set();
    editMode.value = false;
    await load();
    alertMsg.value = `已删除 ${r.deleted ?? ids.length} 条`;
    setTimeout(() => { alertMsg.value = ""; }, 3000);
  } catch (e) {
    confirmDeleteCount.value = 0;
    alertMsg.value = `删除失败：${e.message || e}`;
    setTimeout(() => { alertMsg.value = ""; }, 4000);
  }
}

function cancelBatchDelete() {
  confirmDeleteCount.value = 0;
}

const alertMsg = ref("");
const longPressed = ref(false);  // 长按后阻止 click 打开抽屉

/* ================================================================
 *  右键 / 长按菜单
 * ================================================================ */
function menuStyle() {
  const { x, y } = contextMenu.value;
  // 防止溢出视口：默认右下展开，超出则翻折
  let left = x + "px";
  let top = y + "px";
  return { left, top };
}

function openContextMenu(e, mistake) {
  contextMenu.value = { visible: true, x: e.clientX, y: e.clientY, mistake };
  // 标记已触发右键/长按，阻止后续 click 事件打开抽屉
  longPressed.value = true;
  setTimeout(() => { longPressed.value = false; }, 100);
}
function closeContextMenu() {
  contextMenu.value = { visible: false, x: 0, y: 0, mistake: null };
}

watch(
  () => contextMenu.value.visible,
  (v) => {
    if (v) document.addEventListener("keydown", onCtxKey);
    else document.removeEventListener("keydown", onCtxKey);
  },
);
function onCtxKey(e) {
  if (e.key === "Escape") closeContextMenu();
}

/* 长按检测 */
function onPointerDown(e, mistake) {
  pointerMoved = false;
  pointerTimer = setTimeout(() => {
    if (!pointerMoved) {
      openContextMenu(e, mistake);
    }
  }, 500);
}
function onPointerMove()  { pointerMoved = true; }
function onPointerUp()    { clearTimeout(pointerTimer); }

/* 菜单操作 */
async function menuAskQuestion() {
  const m = contextMenu.value.mistake;
  if (!m) return;
  closeContextMenu();
  const q = (m.question || "").replace(/<[^>]+>/g, "").slice(0, 200);
  navigateToChatWithMessage({ action: "ask-question", text: `追问这道错题：${q}` });
}

async function menuTogglePin(pin) {
  const m = contextMenu.value.mistake;
  if (!m) return;
  closeContextMenu();
  try {
    const r = await props.kernel.triggerCommand("grading::update", { id: String(m.id), pinned: pin });
    Object.assign(m, r.mistake);
    await load();
  } catch (e) {
    alertMsg.value = `操作失败：${e.message || e}`;
    setTimeout(() => { alertMsg.value = ""; }, 3000);
  }
}

async function menuMarkMastered() {
  const m = contextMenu.value.mistake;
  if (!m) return;
  closeContextMenu();
  try {
    const r = await props.kernel.triggerCommand("grading::update", { id: String(m.id), is_correct: true });
    Object.assign(m, r.mistake);
    await load();
  } catch (e) {
    alertMsg.value = `操作失败：${e.message || e}`;
    setTimeout(() => { alertMsg.value = ""; }, 3000);
  }
}

async function menuUnmarkMastered() {
  const m = contextMenu.value.mistake;
  if (!m) return;
  closeContextMenu();
  try {
    const r = await props.kernel.triggerCommand("grading::update", { id: String(m.id), is_correct: false });
    Object.assign(m, r.mistake);
    await load();
    alertMsg.value = "已取消已掌握标记";
    setTimeout(() => { alertMsg.value = ""; }, 2000);
  } catch (e) {
    alertMsg.value = `操作失败：${e.message || e}`;
    setTimeout(() => { alertMsg.value = ""; }, 3000);
  }
}

async function menuDelete() {
  const m = contextMenu.value.mistake;
  if (!m) return;
  closeContextMenu();
  try {
    await props.kernel.triggerCommand("grading::remove", { id: String(m.id) });
    await load();
    alertMsg.value = "已删除";
    setTimeout(() => { alertMsg.value = ""; }, 2000);
  } catch (e) {
    alertMsg.value = `删除失败：${e.message || e}`;
    setTimeout(() => { alertMsg.value = ""; }, 3000);
  }
}

/* ================================================================
 *  单题编辑弹窗
 * ================================================================ */
function openEditDialog(mistake) {
  editForm.value = {
    id:               String(mistake.id),
    subject:          mistake.subject || "",
    knowledge_point:  mistake.knowledge_point || "",
    question:         mistake.question || "",
    student_answer:   mistake.student_answer || "",
    reference_answer: mistake.reference_answer || "",
    analysis:         mistake.analysis || "",
  };
  editingMistake.value = mistake;
}

async function saveEditDialog() {
  const m = editingMistake.value;
  if (!m) return;
  try {
    const r = await props.kernel.triggerCommand("grading::update", {
      id:               editForm.value.id,
      subject:          editForm.value.subject || undefined,
      knowledge_point:  editForm.value.knowledge_point || undefined,
      question:         editForm.value.question || undefined,
      student_answer:   editForm.value.student_answer || undefined,
      reference_answer: editForm.value.reference_answer || null,
      analysis:         editForm.value.analysis || undefined,
    });
    Object.assign(m, r.mistake);
    editingMistake.value = null;
    await load();
    alertMsg.value = "已保存";
    setTimeout(() => { alertMsg.value = ""; }, 2000);
  } catch (e) {
    alertMsg.value = `保存失败：${e.message || e}`;
    setTimeout(() => { alertMsg.value = ""; }, 4000);
  }
}

function closeEditDialog() {
  editingMistake.value = null;
}

/* ================================================================
 *  抽屉操作（标记已掌握 / 变式练习）
 * ================================================================ */
async function markDrawerMastered() {
  const m = drawerItem.value;
  if (!m) return;
  try {
    const r = await props.kernel.triggerCommand("grading::update", { id: String(m.id), is_correct: true });
    Object.assign(m, r.mistake);
    await load();
    alertMsg.value = "已标记为掌握";
    setTimeout(() => { alertMsg.value = ""; }, 2000);
  } catch (e) {
    alertMsg.value = `操作失败：${e.message || e}`;
    setTimeout(() => { alertMsg.value = ""; }, 3000);
  }
}

function doVariantPractice() {
  const m = drawerItem.value;
  if (!m) return;
  const kp = m.knowledge_point || "";
  navigateToChatWithMessage({
    action: "variant-practice",
    knowledge_point: kp,
    difficulty: "variant",
  });
}

onMounted(load);
</script>

<template>
  <div class="page">
    <!-- ======== 头部 ======== -->
    <div class="page-head">
      <h2>错题本</h2>
      <div class="page-head-actions">
        <button class="btn ghost" :class="{ active: editMode }" @click="toggleEditMode">
          <Icon icon="mdi:pencil-box-multiple" width="18" />
          {{ editMode ? "退出编辑" : "编辑" }}
        </button>
        <button class="btn ghost" :disabled="loading" @click="load">
          <Icon icon="mdi:refresh" width="18" />刷新
        </button>
      </div>
    </div>

    <!-- 操作提示 -->
    <p v-if="alertMsg" class="alert success" role="status">
      <Icon icon="mdi:check-circle-outline" width="18" />{{ alertMsg }}
    </p>

    <!-- ======== 统计卡片 ======== -->
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

    <!-- ======== 筛选 chips ======== -->
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

    <!-- ======== 搜索 + 排序 ======== -->
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

    <!-- ======== 状态提示 ======== -->
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

    <!-- ======== 编辑工具栏 ======== -->
    <div v-if="editMode && filtered.length" class="edit-toolbar">
      <label class="edit-check-all">
        <input type="checkbox" :checked="allSelected" @change="selectAll" />
        全选（{{ selectedIds.size }}/{{ filtered.length }}）
      </label>
      <button
        class="btn danger"
        :disabled="selectedIds.size === 0"
        @click="batchDelete"
      >
        <Icon icon="mdi:delete-outline" width="16" />批量删除（{{ selectedIds.size }}）
      </button>
    </div>

    <!-- ======== 卡片网格（扫读层） ======== -->
    <div class="mistake-grid">
      <article
        v-for="m in filtered"
        :key="String(m.id)"
        class="card mistake-card"
        :class="{ 'edit-mode': editMode }"
        tabindex="0"
        role="button"
        :aria-label="'打开错题详情：' + stripHtml(m.question).slice(0, 40)"
        @click="longPressed ? (longPressed = false) : (editMode ? toggleSelect(String(m.id)) : openDrawer(m))"
        @keydown.enter="longPressed ? (longPressed = false) : (editMode ? toggleSelect(String(m.id)) : openDrawer(m))"
        @keydown.space.prevent="longPressed ? (longPressed = false) : (editMode ? toggleSelect(String(m.id)) : openDrawer(m))"
        @contextmenu.prevent.stop="openContextMenu($event, m)"
        @pointerdown="onPointerDown($event, m)"
        @pointermove="onPointerMove"
        @pointerup="onPointerUp"
      >
        <!-- 编辑模式复选框 -->
        <div v-if="editMode" class="mistake-card-check" @click.stop>
          <input
            type="checkbox"
            :checked="selectedIds.has(String(m.id))"
            @change="toggleSelect(String(m.id))"
          />
        </div>

        <div class="card-head">
          <span v-if="m.pinned" class="badge pinned-badge">
            <Icon icon="mdi:pin" width="11" />置顶
          </span>
          <span v-if="m.is_correct" class="badge success">
            <Icon icon="mdi:check-circle" width="11" />已掌握
          </span>
          <span class="badge">{{ m.subject || "未分类" }}</span>
          <span class="badge weak">{{ m.knowledge_point || "未标注知识点" }}</span>
          <span v-if="isLongText(m.question)" class="badge long-text-badge">
            <Icon icon="mdi:file-document-outline" width="12" />长文 · 约 {{ wordCount(m.question) }} 词
          </span>
          <time class="muted">{{ formatTime(m.created_at) }}</time>
        </div>

        <!-- 题干 2 行截断 -->
        <div class="mistake-question-clamp md-body" v-html-smiles="m.question"></div>

        <!-- 作答对比压缩为一行 -->
        <div class="answer-strip">
          <span v-if="m.student_answer" class="answer-strip-label student-label">你的作答</span>
          <span v-if="m.student_answer" class="answer-strip-text">{{ m.student_answer }}</span>
          <span v-if="m.student_answer && m.reference_answer" class="answer-strip-sep">|</span>
          <span v-if="m.reference_answer" class="answer-strip-label ref-label">参考答案</span>
          <span v-if="m.reference_answer" class="answer-strip-text">{{ m.reference_answer }}</span>
        </div>
      </article>
    </div>

    <!-- ======== 右键 / 长按菜单 ======== -->
    <Teleport to="body">
      <div
        v-if="contextMenu.visible"
        class="ctx-overlay"
        @click="closeContextMenu"
        @keydown.esc="closeContextMenu"
      >
        <div class="ctx-menu card" :style="menuStyle()" @click.stop>
          <button class="ctx-item" @click="menuAskQuestion()">
            <Icon icon="mdi:chat-question-outline" width="18" />追问
          </button>
          <button
            v-if="contextMenu.mistake?.pinned"
            class="ctx-item"
            @click="menuTogglePin(false)"
          >
            <Icon icon="mdi:pin-off-outline" width="18" />取消置顶
          </button>
          <button v-else class="ctx-item" @click="menuTogglePin(true)">
            <Icon icon="mdi:pin-outline" width="18" />置顶
          </button>
          <button
            v-if="!contextMenu.mistake?.is_correct"
            class="ctx-item"
            @click="menuMarkMastered()"
          >
            <Icon icon="mdi:check-circle-outline" width="18" />标记已掌握
          </button>
          <button
            v-else
            class="ctx-item"
            @click="menuUnmarkMastered()"
          >
            <Icon icon="mdi:close-circle-outline" width="18" />取消已掌握
          </button>
          <button class="ctx-item" @click="openEditDialog(contextMenu.mistake)">
            <Icon icon="mdi:pencil-outline" width="18" />编辑
          </button>
          <button class="ctx-item danger" @click="menuDelete()">
            <Icon icon="mdi:delete-outline" width="18" />删除
          </button>
        </div>
      </div>
    </Teleport>

    <!-- ======== 批量删除确认 ======== -->
    <Teleport to="body">
      <div v-if="confirmDeleteCount > 0" class="confirm-overlay" @click.self="cancelBatchDelete">
        <div class="confirm-dialog card">
          <p>
            <Icon icon="mdi:alert-circle-outline" width="20" />
            确定要删除选中的 <strong>{{ confirmDeleteCount }}</strong> 道错题吗？
          </p>
          <p class="muted">删除后可在后端数据中保留（软删除），列表不再展示。</p>
          <div class="confirm-actions">
            <button class="btn ghost" @click="cancelBatchDelete">取消</button>
            <button class="btn danger" @click="batchDelete">确认删除</button>
          </div>
        </div>
      </div>
    </Teleport>

    <!-- ======== 单题编辑弹窗 ======== -->
    <Teleport to="body">
      <div v-if="editingMistake" class="mistake-edit-overlay" @click.self="closeEditDialog">
        <div class="mistake-edit-dialog card" @keydown.esc="closeEditDialog">
          <div class="edit-dialog-head">
            <h3><Icon icon="mdi:pencil-outline" width="20" />编辑错题</h3>
            <button class="icon-btn" @click="closeEditDialog" aria-label="关闭">
              <Icon icon="mdi:close" width="20" />
            </button>
          </div>
          <div class="edit-dialog-body">
            <div class="field">
              <span>学科</span>
              <input v-model="editForm.subject" class="input" placeholder="如：数学" />
            </div>
            <div class="field">
              <span>知识点</span>
              <input v-model="editForm.knowledge_point" class="input" placeholder="如：三角函数" />
            </div>
            <div class="field">
              <span>题目</span>
              <textarea v-model="editForm.question" rows="4" class="input" placeholder="题目内容（支持 Markdown）"></textarea>
            </div>
            <div class="field">
              <span>你的作答</span>
              <textarea v-model="editForm.student_answer" rows="2" class="input" placeholder="学生作答"></textarea>
            </div>
            <div class="field">
              <span>参考答案</span>
              <textarea v-model="editForm.reference_answer" rows="2" class="input" placeholder="参考答案"></textarea>
            </div>
            <div class="field">
              <span>错因分析</span>
              <textarea v-model="editForm.analysis" rows="3" class="input" placeholder="错因分析（支持 Markdown）"></textarea>
            </div>
          </div>
          <div class="edit-dialog-foot">
            <button class="btn ghost" @click="closeEditDialog">取消</button>
            <button class="btn primary" @click="saveEditDialog">保存</button>
          </div>
        </div>
      </div>
    </Teleport>

    <!-- ======== 抽屉遮罩 + 面板（精读层） ======== -->
    <Transition name="drawer">
      <div
        v-if="drawerItem"
        class="mistake-drawer-overlay"
        @click.self="closeDrawer"
        aria-label="关闭详情"
      >
        <aside class="mistake-drawer" role="dialog" aria-label="错题详情">
          <!-- 头部 -->
          <div class="drawer-header">
            <div class="drawer-header-tags">
              <span v-if="drawerItem.pinned" class="badge pinned-badge">
                <Icon icon="mdi:pin" width="11" />置顶
              </span>
              <span v-if="drawerItem.is_correct" class="badge success">
                <Icon icon="mdi:check-circle" width="11" />已掌握
              </span>
              <span class="badge">{{ drawerItem.subject || "未分类" }}</span>
              <span class="badge weak">{{ drawerItem.knowledge_point || "未标注知识点" }}</span>
              <span v-if="isLongText(drawerItem.question)" class="badge long-text-badge">
                <Icon icon="mdi:file-document-outline" width="12" />长文 · 约 {{ wordCount(drawerItem.question) }} 词
              </span>
            </div>
            <div class="drawer-header-right">
              <time class="muted">{{ formatTime(drawerItem.created_at) }}</time>
              <span class="drawer-counter muted">{{ drawerIndex + 1 }} / {{ filtered.length }}</span>
              <button class="icon-btn" @click="closeDrawer" aria-label="关闭详情">
                <Icon icon="mdi:close" width="20" />
              </button>
            </div>
          </div>

          <!-- 滚动内容区 -->
          <div class="drawer-body">
            <!-- 完整题干 -->
            <section class="drawer-section">
              <h3 class="drawer-section-title">
                <Icon icon="mdi:help-circle-outline" width="18" />题目
              </h3>
              <div class="drawer-question md-body" v-html-smiles="drawerItem.question"></div>
            </section>

            <!-- 你的作答 / 参考答案 红绿对照 -->
            <section v-if="drawerItem.student_answer || drawerItem.reference_answer" class="drawer-section">
              <h3 class="drawer-section-title">
                <Icon icon="mdi:compare-horizontal" width="18" />作答对比
              </h3>
              <div class="answer-blocks">
                <div v-if="drawerItem.student_answer" class="answer-block student">
                  <div class="answer-block-label">
                    <Icon icon="mdi:pencil-outline" width="16" />你的作答
                  </div>
                  <div class="answer-block-text md-body" v-html-smiles="drawerItem.student_answer"></div>
                </div>
                <div v-if="drawerItem.reference_answer" class="answer-block reference">
                  <div class="answer-block-label">
                    <Icon icon="mdi:check-decagram-outline" width="16" />参考答案
                  </div>
                  <div class="answer-block-text md-body" v-html-smiles="drawerItem.reference_answer"></div>
                </div>
              </div>
            </section>

            <!-- 完整错因分析 -->
            <section v-if="drawerItem.analysis" class="drawer-section">
              <h3 class="drawer-section-title">
                <Icon icon="mdi:lightbulb-on-outline" width="18" />错因分析
              </h3>
              <div class="drawer-analysis md-body" v-html-smiles="drawerItem.analysis"></div>
            </section>

            <!-- 我的备注 -->
            <section class="drawer-section">
              <div class="dw-note">
                <div class="dw-note-head">
                  <span class="dw-note-title">
                    <Icon icon="mdi:pencil-outline" width="16" />我的备注
                  </span>
                  <Transition name="fade">
                    <span v-if="noteSaved" class="dw-note-saved">
                      <Icon icon="mdi:check" width="13" />已自动保存
                    </span>
                  </Transition>
                </div>
                <textarea
                  :ref="setNoteTa"
                  v-model="noteText"
                  class="dw-note-ta"
                  placeholder="记点心得：当时怎么想的、下次怎么避坑…"
                  @input="onNoteInput"
                ></textarea>
              </div>
            </section>
          </div>

          <!-- 固定底部操作栏 -->
          <div class="drawer-foot">
            <button class="btn ghost drawer-nav-btn" @click="goPrev" :disabled="filtered.length <= 1">
              <Icon icon="mdi:chevron-left" width="22" />上一题
            </button>
            <div class="drawer-foot-actions">
              <button class="btn primary drawer-action-btn" @click="doVariantPractice">
                <Icon icon="mdi:sparkles" width="16" />变式练习
              </button>
              <button
                class="btn drawer-action-btn"
                :class="drawerItem?.is_correct ? 'success-ghost' : 'ghost'"
                :disabled="drawerItem?.is_correct"
                @click="markDrawerMastered"
              >
                <Icon icon="mdi:check-circle-outline" width="16" />
                {{ drawerItem?.is_correct ? "已掌握" : "标记已掌握" }}
              </button>
              <button class="btn ghost drawer-action-btn" @click="openEditDialog(drawerItem)">
                <Icon icon="mdi:pencil-outline" width="16" />编辑
              </button>
            </div>
            <button class="btn ghost drawer-nav-btn" @click="goNext" :disabled="filtered.length <= 1">
              下一题<Icon icon="mdi:chevron-right" width="22" />
            </button>
          </div>
        </aside>
      </div>
    </Transition>
  </div>
</template>
