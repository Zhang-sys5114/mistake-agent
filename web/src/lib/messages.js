// 会话消息树 → 前端气泡（聊天页与会话历史详情共用同一渲染）。

import { toolIcon, toolTitle } from "./tools";

// 附件名截到（ 或 ( 为止：历史消息里 kernel 曾在名字后接「（…）」注记，
// 不截断会把注记吞进附件名。
const ATTACH_RE = /\n附件：(\S+)\|([^|\n（(]+)/g;
// 系统临时暂存路径（mistake-agent- 前缀）：展示时一律隐藏，不把路径暴露给学生。
const TMP_PATH_RE = /\/tmp\/mistake-agent-[^\s|（(]+/g;
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
 * 会话消息树 → 前端气泡；同一父节点的兄弟互为分支。
 * opts.history=true（会话历史页）保留 system 消息完整原文；
 * 聊天合并流（默认）中，会话切换摘要只显示一次「会话已切换」：
 * 旧会话的交接摘要不渲染，新会话的上一会话梗概显示精简文案。
 */
export function renderPath(messages, opts = {}) {
  const byParent = new Map();
  for (const m of messages || []) {
    const key = m.parent_id ? String(m.parent_id) : "__root__";
    if (!byParent.has(key)) byParent.set(key, []);
    byParent.get(key).push(String(m.id));
  }
  return (messages || [])
    .map((m) => {
      const parentKey = m.parent_id ? String(m.parent_id) : "__root__";
      const siblingIds = (byParent.get(parentKey) || []).filter(
        (id) => id !== String(m.id),
      );
      const base = {
        messageId: String(m.id),
        parentId: m.parent_id ? String(m.parent_id) : null,
        siblingIds,
        createdAt: m.created_at,
      };
      const kind = m.kind || {};
      if (kind.kind === "user") {
        const raw = kind.text || "";
        const attachments = parseAttachments(raw);
        let shown = (kind.display_text || raw)
          .replace(ATTACH_RE, "")
          .replace(TMP_PATH_RE, "")
          .trim();
        if (!kind.display_text) {
          const forced = shown.match(FORCED_RE);
          if (forced) {
            const title = toolTitle(forced[1]);
            const rest = forced[2].replace(TMP_PATH_RE, "").trim();
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
        // 上一会话梗概：聊天流显示为「会话已切换」（新数据取 display_text，旧数据按前缀兜底）。
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
