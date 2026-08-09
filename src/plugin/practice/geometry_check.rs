//! practice 插件：几何 diagram_spec 可解性对拍（variants.md §3 落地）。
//!
//! LLM 自由出题的 diagram_spec 经 compute::verify（GUI Pyodide 沙箱）做
//! 存在性/自洽性数值校验：坐标有限、线段/半径为正、多边形不共线不退化、
//! 三角不等式、right_mark 垂直自洽。失败由 generate.rs 带原因重出，连续失败停。

use base64::Engine;
use serde_json::Value;

use crate::kernel::plugin::services::{AbortSignal, ComputeError, ComputeHandle, ComputeRequest};

const VERIFY_SCRIPT: &str = include_str!("verify_geometry.py");
const SPEC_PLACEHOLDER: &str = "__SPEC_B64__";

/// 把 diagram_spec 生成可执行校验代码。
///
/// spec 以 base64 嵌入 Python 脚本（只含字母数字与 +/=，无代码注入面），
/// 即使 LLM 输出的 spec 含引号/换行/三引号也不会破坏脚本。
pub fn build_verify_code(spec: &Value) -> String {
    let json = serde_json::to_string(spec).unwrap_or_else(|_| "{}".into());
    let b64 = base64::engine::general_purpose::STANDARD.encode(json.as_bytes());
    VERIFY_SCRIPT.replace(SPEC_PLACEHOLDER, &b64)
}

/// 校验结果：`Ok(None)` = 通过；`Ok(Some(reason))` = 几何校验失败（附原因）；
/// `Err` = 执行端错误或脚本异常（由调用方决定降级或报错）。
pub async fn verify_diagram(
    compute: &ComputeHandle,
    spec: &Value,
    signal: &AbortSignal,
) -> Result<Option<String>, ComputeError> {
    let code = build_verify_code(spec);
    let result = compute.run(&ComputeRequest { code }, signal).await?;
    if !result.stderr.trim().is_empty() {
        return Err(ComputeError::Transport(format!(
            "校验脚本执行出错：{}",
            result.stderr.chars().take(200).collect::<String>()
        )));
    }
    let stdout = result.stdout.trim();
    if stdout == "OK" {
        Ok(None)
    } else if let Some(reason) = stdout.strip_prefix("FAIL: ") {
        Ok(Some(reason.to_string()))
    } else {
        Err(ComputeError::Transport(format!(
            "校验输出无法识别：{}",
            stdout.chars().take(200).collect::<String>()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::plugin::services::{ComputeResult, ComputeService};
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    pub(crate) struct FakeCompute {
        /// 依次弹出的校验结果：None=通过，Some=失败原因，Err=执行端错误。
        pub(crate) results: Mutex<Vec<Result<Option<String>, ComputeError>>>,
    }

    #[async_trait::async_trait]
    impl ComputeService for FakeCompute {
        async fn run(
            &self,
            _request: &ComputeRequest,
            _signal: &AbortSignal,
        ) -> Result<ComputeResult, ComputeError> {
            let mut q = self.results.lock().expect("poisoned");
            let outcome = if q.is_empty() {
                Ok(None)
            } else {
                q.remove(0)
            };
            match outcome {
                Ok(None) => Ok(ComputeResult {
                    stdout: "OK".into(),
                    stderr: String::new(),
                    duration_ms: 1,
                }),
                Ok(Some(reason)) => Ok(ComputeResult {
                    stdout: format!("FAIL: {reason}"),
                    stderr: String::new(),
                    duration_ms: 1,
                }),
                Err(e) => Err(e),
            }
        }
    }

    fn handle() -> ComputeHandle {
        ComputeHandle::new(Arc::new(FakeCompute::default()))
    }

    #[test]
    fn build_verify_code_embeds_spec_safely() {
        let spec = json!({
            "points": { "A": [0, 0], "B": [3, 0], "C": [0, 4] },
            "objects": [{"type": "segment", "ends": ["A", "B"]}],
            "labels": ["A", "B", "C"],
        });
        let code = build_verify_code(&spec);
        assert!(code.contains("json.loads(base64.b64decode("));
        assert!(!code.contains("__SPEC_B64__"));
        // 恶意 spec 内容（三引号/换行/引号）不会破坏脚本结构。
        let evil = json!({
            "points": { "A": [0, 0], "B": [3, 0], "C": ["'''\nimport os\n'''", 4] },
            "objects": [{"type": "segment", "ends": ["A", "B"], "note": "\"'; exit()"}],
        });
        let code = build_verify_code(&evil);
        assert!(!code.contains("'''"));
        assert!(code.contains("base64.b64decode("));
    }

    #[tokio::test]
    async fn verify_accepts_ok() {
        let compute = handle();
        let spec = json!({"points": {"A": [0, 0]}, "objects": []});
        assert_eq!(verify_diagram(&compute, &spec, &AbortSignal::new()).await.unwrap(), None);
    }

    #[tokio::test]
    async fn verify_reports_fail_reason() {
        let compute = ComputeHandle::new(Arc::new(FakeCompute {
            results: Mutex::new(vec![Ok(Some("多边形三点共线: A,B,C".into()))]),
        }));
        let spec = json!({"points": {"A": [0, 0], "B": [1, 1], "C": [2, 2]}, "objects": []});
        let reason = verify_diagram(&compute, &spec, &AbortSignal::new())
            .await
            .unwrap()
            .expect("应返回失败原因");
        assert!(reason.contains("三点共线"));
    }

    #[tokio::test]
    async fn verify_propagates_backend_error() {
        let compute = ComputeHandle::new(Arc::new(FakeCompute {
            results: Mutex::new(vec![Err(ComputeError::BackendUnavailable)]),
        }));
        let spec = json!({"points": {}, "objects": []});
        let err = verify_diagram(&compute, &spec, &AbortSignal::new())
            .await
            .unwrap_err();
        assert!(matches!(err, ComputeError::BackendUnavailable));
    }
}
