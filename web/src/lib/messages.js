// 会话消息树 → 前端气泡（聊天页与会话历史详情共用同一渲染）。

import { toolIcon, toolTitle } from "./tools";

// 附件名截到（ 或 ( 为止：历史消息里 kernel 曾在名字后接「（…）」注记，
// 不截断会把注记吞进附件名。
const ATTACH_RE = /\n附件：(\S+)\|([^|\n（(]+)/g;
// 系统临时暂存路径（Unix + Windows）：展示时一律隐藏，不把路径暴露给学生。
const TMP_PATH_RE = /\/tmp\/mistake-agent-[^\s|（(]+/g;
const WIN_PATH_RE = /\b[A-Z]:\\[^\s|（(]*?mistake-agent[^\s|（(]*/gi;
// 老消息（无 display_text）：从「请调用工具 X 处理：Y」还原为「标题：Y」。
const FORCED_RE = /^请调用工具 (\S+) 处理[:：]?\s*(.*)$/s;

/** 从消息文本解析全部持久化附件标记（kernel 落盘的「附件：路径|名称」，可能多条）。 */
export function parseAttachments(text) {
  const out = [];
  const re = ATTACH_RE;
  re.lastIndex = 0;
  let m;
  while ((m = re.exec(String(text || "")))) {
    out.push({ path: m[1], name: m[2] });
  }
  return out;
}

/**
 * 构建会话视图：消息树 + 逐节点版本指针（参考 DeepSeek 网页版：
 * childIds / currentChildIndex / rootBranchIds / rootBranchIndex）。
 * activePath 来自服务端 meta.active_path，用于把指针初始化到服务端活跃链。
 */
export function buildSessionView(messages, activePath) {
  const list = messages || [];
  const nodes = new Map();
  for (const m of list) {
    nodes.set(String(m.id), { message: m, childIds: [], currentChildIndex: 0 });
  }
  const byTime = (ids) =>
    ids.slice().sort((a, b) => {
      const ta = new Date(nodes.get(a)?.message.created_at || 0);
      const tb = new Date(nodes.get(b)?.message.created_at || 0);
      return ta - tb;
    });
  const roots = [];
  for (const m of list) {
    const id = String(m.id);
    const p = m.parent_id ? String(m.parent_id) : null;
    if (p && nodes.has(p)) nodes.get(p).childIds.push(id);
    else roots.push(id);
  }
  let rootBranchIds = byTime(roots);
  for (const n of nodes.values()) n.childIds = byTime(n.childIds);

  // 沿服务端活跃路径回溯，把每层指针指向活跃子节点
  let end = activePath ? String(activePath) : null;
  if (!end && list.length) end = String(list[list.length - 1].id);
  const path = [];
  const seen = new Set();
  let cur = end;
  while (cur && nodes.has(cur) && !seen.has(cur)) {
    seen.add(cur);
    path.push(cur);
    cur = nodes.get(cur).message.parent_id
      ? String(nodes.get(cur).message.parent_id)
      : null;
  }
  path.reverse();
  let rootBranchIndex = 0;
  if (path.length) {
    const ri = rootBranchIds.indexOf(path[0]);
    if (ri >= 0) rootBranchIndex = ri;
    for (let i = 1; i < path.length; i += 1) {
      const parent = nodes.get(path[i - 1]);
      const ci = parent.childIds.indexOf(path[i]);
      if (ci >= 0) parent.currentChildIndex = ci;
    }
  }
  return { nodes, rootBranchIds, rootBranchIndex };
}

/** 活跃链：根版本 → 逐层 currentChildIndex 子节点 → 叶子。 */
export function getActiveChain(view) {
  const chain = [];
  let cur = view.rootBranchIds[view.rootBranchIndex];
  const seen = new Set();
  while (cur && view.nodes.has(cur) && !seen.has(cur)) {
    seen.add(cur);
    const node = view.nodes.get(cur);
    chain.push(node.message);
    cur = node.childIds[node.currentChildIndex] ?? null;
  }
  return chain;
}

/**
 * < / > 切换版本：对本消息所在版本列表取模循环。
 * 根层切 rootBranchIndex；非根层把父节点的 currentChildIndex 前/后移一位。
 */
export function navigateBranch(view, messageId, dir = 1) {
  const m = view.nodes.get(String(messageId));
  if (!m) return;
  const parentId = m.message.parent_id ? String(m.message.parent_id) : null;
  if (!parentId || !view.nodes.has(parentId)) {
    const n = view.rootBranchIds.length;
    if (n < 2) return;
    view.rootBranchIndex = (((view.rootBranchIndex + dir) % n) + n) % n;
  } else {
    const parent = view.nodes.get(parentId);
    const n = parent.childIds.length;
    if (n < 2) return;
    parent.currentChildIndex = (((parent.currentChildIndex + dir) % n) + n) % n;
  }
}

/**
 * 会话视图 → 前端气泡：只渲染活跃链（一次一个版本，DeepSeek 式）。
 * opts.history=true（会话历史页）保留 system 消息完整原文；聊天流（默认）中
 * 会话切换摘要只显示一次「会话已切换」。每个气泡带版本元数据供 < / > 使用。
 */
export function renderPath(view, opts = {}) {
  return getActiveChain(view)
    .map((m) => {
      const parentKey = m.parent_id ? String(m.parent_id) : "__root__";
      const parentNode =
        parentKey === "__root__" ? null : view.nodes.get(parentKey) || null;
      const group = parentNode ? parentNode.childIds : view.rootBranchIds;
      const versionIndex = parentNode
        ? parentNode.currentChildIndex
        : view.rootBranchIndex;
      const base = {
        messageId: String(m.id),
        parentId: m.parent_id ? String(m.parent_id) : null,
        siblingIds: group.filter((g) => g !== String(m.id)),
        versions: group.map((gid) => ({
          id: gid,
          createdAt: view.nodes.get(gid)?.message.created_at || null,
        })),
        versionIndex,
        versionCount: group.length,
        createdAt: m.created_at,
        sessionKey: opts.sessionKey ?? null,
      };
      const kind = m.kind || {};
      if (kind.kind === "user") {
        const raw = kind.text || "";
        const attachments = parseAttachments(raw);
        let shown = (kind.display_text || raw)
          .replace(ATTACH_RE, "")
          .replace(TMP_PATH_RE, "")
          .replace(WIN_PATH_RE, "")
          .trim();
        if (!kind.display_text) {
          const forced = shown.match(FORCED_RE);
          if (forced) {
            const title = toolTitle(forced[1]);
            const rest = forced[2].replace(TMP_PATH_RE, "").replace(WIN_PATH_RE, "").trim();
            shown = rest ? `${title}：${rest}` : title;
          }
        }
        const text = attachments.length
          ? shown.replace(ATTACH_RE, "").trim()
          : shown;
        return {
          ...base,
          type: "user",
          text,
          attachments,
        };
      }
      if (kind.kind === "assistant") {
        return { ...base, type: "assistant", text: kind.text || "" };
      }
      if (kind.kind === "system") {
        const raw = kind.text || "";
        if (opts.history) {
          // 历史页保留完整交接记录，不隐藏。
          return { ...base, type: "system", text: raw };
        }
        // 聊天合并流：旧会话交接摘要不渲染（新数据 display_text 为空，旧数据按前缀识别）。
        if (kind.display_text === "" || /^交接摘要[:：]/.test(raw)) return null;
        // 上一会话梗概（新会话子树起点）：聊天流显示为「会话已切换」
        // （新数据取 display_text，旧数据按前缀兜底）。
        if (kind.display_text || /^上一会话梗概[:：]/.test(raw)) {
          return { ...base, type: "system", text: kind.display_text || "会话已切换" };
        }
        return {
          ...base,
          type: "system",
          text: raw,
        };
      }
      if (kind.kind === "reasoning") {
        return { ...base, type: "reasoning", text: kind.text || "" };
      }
      if (kind.kind === "tool_call") {
        const ok = Boolean(kind.result?.Ok);
        return {
          ...base,
          type: "tool",
          entry: kind.entry || "",
          toolOk: ok,
          toolIcon: toolIcon(kind.entry || ""),
          params: kind.params || {},
          result: ok ? kind.result?.Ok : kind.result?.Err || null,
        };
      }
      return null;
    })
    .filter(Boolean);
}
