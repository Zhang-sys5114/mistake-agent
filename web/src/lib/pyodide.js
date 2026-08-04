// Pyodide 验算执行端：kernel 发 ComputeRequest 事件，本模块在 WebView 内
// 跑 Python（WASM 沙箱，无文件/网络），结果经 compute_result RPC 回执。

let pyodidePromise = null;

async function loadPyodideOnce() {
  if (!pyodidePromise) {
    pyodidePromise = import("pyodide").then(async (mod) => {
      const py = await mod.loadPyodide({ indexURL: "/pyodide/" });
      return py;
    });
  }
  return pyodidePromise;
}

/**
 * 运行 Python 代码，返回 { stdout, stderr, durationMs }。
 * 超时由 Promise.race 兜底（Pyodide 单线程无法真正中断长循环，
 * 内核侧 200s 超时会另行取消工具调用）。
 */
export async function runPython(code, timeoutMs = 150000) {
  const py = await loadPyodideOnce();
  let stdout = "";
  let stderr = "";
  py.setStdout({ batched: (s) => (stdout += `${s}\n`) });
  py.setStderr({ batched: (s) => (stderr += `${s}\n`) });

  const started = performance.now();
  let timer = null;
  const timeout = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error("Python 执行超过前端时限")), timeoutMs);
  });
  try {
    await Promise.race([py.runPythonAsync(code), timeout]);
  } finally {
    clearTimeout(timer);
  }
  const durationMs = Math.round(performance.now() - started);
  return {
    stdout: stdout.trimEnd(),
    stderr: stderr.trimEnd(),
    durationMs,
  };
}

export async function pyodideStatus() {
  try {
    const py = await loadPyodideOnce();
    return { available: true, version: py.version };
  } catch (e) {
    return { available: false, error: String(e?.message ?? e) };
  }
}
