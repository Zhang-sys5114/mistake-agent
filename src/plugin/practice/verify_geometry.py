# 几何 diagram_spec 可解性校验（存在性 + 自洽性）。
# 由内核 practice 插件经 compute::verify（GUI Pyodide 沙箱）调用；
# spec 以 base64 嵌入 __SPEC_B64__ 占位（无代码注入面）。
import base64
import json
import math

spec = json.loads(base64.b64decode("__SPEC_B64__").decode("utf-8"))
problems = []
pts = spec.get("points") or {}
objects = spec.get("objects") or []


def pt(name):
    c = pts.get(name)
    if not c or len(c) < 2:
        return None
    x, y = c[0], c[1]
    if not (
        isinstance(x, (int, float))
        and isinstance(y, (int, float))
        and math.isfinite(x)
        and math.isfinite(y)
    ):
        return None
    return (float(x), float(y))


for name in pts:
    if pt(name) is None:
        problems.append("点 %s 坐标非法" % name)

segs, polys, circs = [], [], []
for obj in objects:
    t = obj.get("type")
    if t == "segment":
        segs.append(obj.get("ends") or [])
    elif t == "polygon":
        polys.append(obj.get("points") or obj.get("ends") or [])
    elif t == "circle":
        circs.append(obj)

for ends in segs:
    if len(ends) != 2:
        problems.append("线段端点非法: %s" % ends)
        continue
    a, b = pt(ends[0]), pt(ends[1])
    if a is None or b is None:
        problems.append("线段端点未定义: %s" % ends)
        continue
    if math.dist(a, b) <= 1e-9:
        problems.append("线段 %s-%s 长度为零" % (ends[0], ends[1]))

for c in circs:
    center, r = c.get("center"), c.get("radius")
    if pt(center) is None:
        problems.append("圆心未定义: %s" % center)
    if not (isinstance(r, (int, float)) and r > 1e-9):
        problems.append("圆半径非法: %s" % r)


def area3(a, b, c):
    return abs((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])) / 2.0


for verts in polys:
    ps = [pt(v) for v in verts]
    if any(p is None for p in ps):
        problems.append("多边形顶点未定义: %s" % verts)
        continue
    n = len(ps)
    if n < 3:
        problems.append("多边形顶点不足: %s" % verts)
        continue
    for i in range(n):
        for j in range(i + 1, n):
            if math.dist(ps[i], ps[j]) <= 1e-9:
                problems.append("多边形顶点重合: %s,%s" % (verts[i], verts[j]))
    for i in range(n):
        for j in range(i + 1, n):
            for k in range(j + 1, n):
                if area3(ps[i], ps[j], ps[k]) <= 1e-9:
                    problems.append("多边形三点共线: %s,%s,%s" % (verts[i], verts[j], verts[k]))
                a = math.dist(ps[i], ps[j])
                b = math.dist(ps[j], ps[k])
                c = math.dist(ps[k], ps[i])
                if a + b <= c + 1e-9 or a + c <= b + 1e-9 or b + c <= a + 1e-9:
                    problems.append("三角形 %s,%s,%s 不满足三角不等式" % (verts[i], verts[j], verts[k]))

for obj in objects:
    if obj.get("type") == "right_mark":
        v, a, b = pt(obj.get("vertex")), pt(obj.get("arm1")), pt(obj.get("arm2"))
        if v is None or a is None or b is None:
            problems.append("直角标记点未定义: %s" % obj.get("vertex"))
            continue
        va = (a[0] - v[0], a[1] - v[1])
        vb = (b[0] - v[0], b[1] - v[1])
        if abs(va[0] * vb[0] + va[1] * vb[1]) > 1e-6:
            problems.append("直角标记 %s 处两臂不垂直" % obj.get("vertex"))

if problems:
    print("FAIL: " + " | ".join(problems))
else:
    print("OK")
