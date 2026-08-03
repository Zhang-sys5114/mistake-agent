//! Tauri GUI 入口：拉起 sidecar kernel，经 stdio JSONL 通信（Channel 桥接）。

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Mutex;

use tauri::{Manager, State, ipc::Channel};

struct KernelProcess {
    #[allow(dead_code)]
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
}

/// 启动 sidecar：stdout 逐行经 Channel 推给前端，前端经 kernel_send 回写 stdin。
#[tauri::command]
fn start_kernel(app: tauri::AppHandle, on_frame: Channel<String>) -> Result<(), String> {
    let sidecar = sidecar_path();
    let mut child = Command::new(sidecar)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("拉起 sidecar 失败：{e}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "sidecar 无 stdout".to_string())?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "sidecar 无 stdin".to_string())?;

    if let Some(stderr) = child.stderr.take() {
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                eprintln!("[sidecar] {line}");
            }
        });
    }
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            let _ = on_frame.send(line);
        }
    });

    app.manage(KernelProcess {
        child: Mutex::new(child),
        stdin: Mutex::new(stdin),
    });
    Ok(())
}

/// 定位 sidecar 二进制：开发期与主程序同目录（target/debug/），打包期随 bundle 放置。
fn sidecar_path() -> std::path::PathBuf {
    let exe = std::env::current_exe().expect("无法定位当前可执行文件");
    let dir = exe.parent().expect("无法定位可执行文件目录");
    let name = if cfg!(windows) {
        "sidecar.exe"
    } else {
        "sidecar"
    };
    dir.join(name)
}

/// 向前端转发：写一行 JSONL 请求给 sidecar。
#[tauri::command]
fn kernel_send(state: State<'_, KernelProcess>, line: String) -> Result<(), String> {
    let mut stdin = state.stdin.lock().map_err(|e| e.to_string())?;
    writeln!(stdin, "{line}").map_err(|e| e.to_string())?;
    stdin.flush().map_err(|e| e.to_string())
}

/// 作业文件选择器：返回本地路径，前端拼成消息让模型调 grading::upload。
#[tauri::command]
fn pick_homework_file() -> Result<Option<String>, String> {
    let picked = rfd::FileDialog::new()
        .add_filter("作业文件", &["png", "jpg", "jpeg", "webp", "bmp", "pdf"])
        .pick_file();
    Ok(picked.map(|p| p.to_string_lossy().into_owned()))
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            start_kernel,
            kernel_send,
            pick_homework_file
        ])
        .run(tauri::generate_context!())
        .expect("Tauri 应用运行失败");
}
