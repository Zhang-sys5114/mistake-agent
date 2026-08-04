// 会话消息树 → 前端气泡（聊天页与会话历史详情共用同一渲染）。

const TOOL_ICONS = {
  "demo::hello": "mdi:hand-wave",
  "grading::upload": "mdi:upload",
  "grading::list": "mdi:format-list-bulleted",
  "memory::save": "mdi:content-save",
  "memory::show": "mdi:book-open-variant",
  "memory::remove": "mdi:delete",
  "compute::verify": "mdi:calculator-variant",
  "practice::generate": "mdi:pen",
  "report::weekly": "mdi:chart-bar",
  "exam::compose": "mdi:file-document-edit-outline",
  "tracking::checkin": "mdi:clipboard-check-outline",
};

export function toolIcon(entry) {
  return TOOL_ICONS[entry] ?? "mdi:toolbox-outline";
}

const ATTACH_RE = /\n附件：(\S+)\|([^|\n]+)/;

/** 从消息文本解析持久化附件标记（kernel 落盘的「附件：路径|名称」）。 */
export function parseAttachment(text) {
  const m = String(text || "").match(ATTACH_RE);
  if (!m) return null;
  return { path: m[1], name: m[2] };
}

/** 会话消息树 → 前端气泡；同一父节点的兄弟互为分支。 */
export function renderPath(messages) {
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
        const text = kind.text || "";
        const attachment = parseAttachment(text);
        return {
          ...base,
          type: "user",
          text: attachment ? text.replace(ATTACH_RE, "").trim() : text,
          attachment,
        };
      }
      if (kind.kind === "assistant") {
        return { ...base, type: "assistant", text: kind.text || "" };
      }
      if (kind.kind === "system") {
        return { ...base, type: "system", text: kind.text || "" };
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
