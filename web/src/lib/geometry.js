// 几何图渲染器（场景二预研）：diagram_spec → SVG。
// 纯确定性数学渲染：模型只出规格，渲染器画图；图与答案同源，不会自相矛盾。

function num(v) {
  const n = Number(v);
  if (!Number.isFinite(n)) throw new Error(`非法坐标：${v}`);
  return n;
}

export function renderGeometry(spec) {
  const points = {};
  for (const [name, xy] of Object.entries(spec.points || {})) {
    points[name] = [num(xy[0]), num(xy[1])];
  }

  // 视口自适应：所有点外扩 margin。
  const xs = Object.values(points).map(([x]) => x);
  const ys = Object.values(points).map(([, y]) => y);
  const margin = 1.2;
  const minX = Math.min(...xs) - margin;
  const maxX = Math.max(...xs) + margin;
  const minY = Math.min(...ys) - margin;
  const maxY = Math.max(...ys) + margin;
  const viewBox = [minX, minY, maxX - minX, maxY - minY].join(" ");

  const parts = [];
  const push = (s) => parts.push(s);

  const P = (name) => {
    const p = points[name];
    if (!p) throw new Error(`未知点：${name}`);
    return p;
  };

  for (const obj of spec.objects || []) {
    const common = `stroke="#1f2937" stroke-width="0.08" fill="none"`;
    switch (obj.type) {
      case "segment": {
        const [ax, ay] = P(obj.ends[0]);
        const [bx, by] = P(obj.ends[1]);
        const dash = obj.dashed ? ' stroke-dasharray="0.35 0.25"' : "";
        const color = obj.color ? ` stroke="${obj.color}"` : "";
        push(`<line x1="${ax}" y1="${ay}" x2="${bx}" y2="${by}" ${common}${dash}${color}/>`);
        break;
      }
      case "polygon": {
        const pts = obj.vertices.map(P).map(([x, y]) => `${x},${y}`).join(" ");
        const fill = obj.fill ? ` fill="${obj.fill}" fill-opacity="0.15"` : "";
        const dash = obj.dashed ? ' stroke-dasharray="0.35 0.25"' : "";
        push(`<polygon points="${pts}" ${common}${fill}${dash}/>`);
        break;
      }
      case "circle": {
        const [cx, cy] = P(obj.center);
        const r = num(obj.radius);
        const dash = obj.dashed ? ' stroke-dasharray="0.35 0.25"' : "";
        push(`<circle cx="${cx}" cy="${cy}" r="${r}" ${common}${dash}/>`);
        break;
      }
      case "right_mark": {
        // 直角标记：顶点 C，沿 CA / CB 各取 0.5 单位做小方块。
        const [vx, vy] = P(obj.vertex);
        const [ax, ay] = P(obj.arm1);
        const [bx, by] = P(obj.arm2);
        const u1 = norm([ax - vx, ay - vy]);
        const u2 = norm([bx - vx, by - vy]);
        const s = 0.45;
        const p1 = [vx + u1[0] * s, vy + u1[1] * s];
        const p2 = [vx + u1[0] * s + u2[0] * s, vy + u1[1] * s + u2[1] * s];
        const p3 = [vx + u2[0] * s, vy + u2[1] * s];
        push(
          `<polyline points="${p1[0]},${p1[1]} ${p2[0]},${p2[1]} ${p3[0]},${p3[1]}" ${common}/>`,
        );
        break;
      }
      case "equal_ticks": {
        // 等长标记：在两端点中点画两条垂直小短线。
        const [ax, ay] = P(obj.ends[0]);
        const [bx, by] = P(obj.ends[1]);
        const u = norm([bx - ax, by - ay]);
        const nv = [-u[1], u[0]];
        const len = Math.hypot(bx - ax, by - ay);
        const n = obj.count || 2;
        for (let i = 1; i <= n; i++) {
          const t = (i / (n + 1)) * len;
          const mx = ax + u[0] * t;
          const my = ay + u[1] * t;
          const h = 0.18;
          push(
            `<line x1="${mx - nv[0] * h}" y1="${my - nv[1] * h}" x2="${mx + nv[0] * h}" y2="${my + nv[1] * h}" ${common}/>`,
          );
        }
        break;
      }
      case "angle_arc": {
        const [vx, vy] = P(obj.vertex);
        const [ax, ay] = P(obj.arm1);
        const [bx, by] = P(obj.arm2);
        const r = num(obj.radius ?? 0.8);
        const a1 = Math.atan2(ay - vy, ax - vx);
        const a2 = Math.atan2(by - vy, bx - vx);
        const large = Math.abs(a2 - a1) > Math.PI ? 1 : 0;
        const sweep = a2 > a1 ? 1 : 0;
        const p1 = [vx + Math.cos(a1) * r, vy + Math.sin(a1) * r];
        const p2 = [vx + Math.cos(a2) * r, vy + Math.sin(a2) * r];
        push(
          `<path d="M ${p1[0]} ${p1[1]} A ${r} ${r} 0 ${large} ${sweep} ${p2[0]} ${p2[1]}" ${common}/>`,
        );
        break;
      }
      case "label": {
        const [x, y] = P(obj.point);
        const dx = num(obj.dx ?? 0.25);
        const dy = num(obj.dy ?? -0.35);
        const text = escapeXml(String(obj.text ?? obj.point));
        push(
          `<text x="${x + dx}" y="${y + dy}" font-size="0.5" fill="#1f2937" font-family="sans-serif">${text}</text>`,
        );
        break;
      }
      default:
        throw new Error(`未知图形对象：${obj.type}`);
    }
  }

  return {
    svg: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${viewBox}" width="100%" height="100%" preserveAspectRatio="xMidYMid meet">${parts.join("")}</svg>`,
    viewBox,
  };
}

function norm([x, y]) {
  const len = Math.hypot(x, y);
  return len === 0 ? [0, 0] : [x / len, y / len];
}

function escapeXml(s) {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

export const SAMPLE_TRIANGLE = {
  points: {
    A: [0, 0],
    B: [6, 0],
    C: [6, 4],
    O: [3, 2],
  },
  objects: [
    { type: "segment", ends: ["A", "B"] },
    { type: "segment", ends: ["B", "C"] },
    { type: "segment", ends: ["C", "A"] },
    { type: "right_mark", vertex: "C", arm1: "B", arm2: "A" },
    { type: "equal_ticks", ends: ["A", "B"] },
    { type: "circle", center: "O", radius: 1.8, dashed: true },
    { type: "label", point: "A", text: "A", dx: -0.35, dy: 0.45 },
    { type: "label", point: "B", text: "B", dx: 0.15, dy: 0.45 },
    { type: "label", point: "C", text: "C", dx: 0.25, dy: -0.4 },
    { type: "label", point: "O", text: "O", dx: -0.35, dy: 0.45 },
  ],
};
