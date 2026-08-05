// 工具目录（Tool catalog）：唯一事实源 = 后端 list_tools（user_visible 入口）。
// 前端不得硬编码工具名 → 标题/图标映射；缺失（不可见工具如 compute::verify）时回退 entry 名。
// 规则见 CONTEXT.md「Tool catalog」、PROJECT.md §5、docs/api.md、docs/plugin-dev/user.md。

let catalog = new Map(); // entry -> list_tools 原始条目
let loading = null; // 并发去重：多个页面同时拉取只发一次请求

export function toolList() {
  return [...catalog.values()];
}

export async function loadToolCatalog(kernel) {
  if (catalog.size > 0) return catalog;
  if (loading) return loading;
  loading = (async () => {
    try {
      const r = await kernel.listTools();
      catalog = new Map((r.tools || []).map((t) => [t.entry, t]));
    } catch {
      // 拉取失败：保持空目录，调用方按 entry 名回退。
    }
    return catalog;
  })();
  return loading;
}

export function toolTitle(entry) {
  const t = catalog.get(entry);
  return t?.title || entry;
}

export function toolIcon(entry) {
  return catalog.get(entry)?.icon || "mdi:toolbox-outline";
}
