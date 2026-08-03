# 使用说明

## 1. 前置配置

数据根目录：`~/Documents/.mistake-agent/`（Windows：`%USERPROFILE%\Documents\.mistake-agent\`）。首次运行前创建 `settings.json`：

```json
{
  "log_level": "info",
  "main_model": {
    "api_url": "https://api.deepseek.com",
    "api_key": "你的 DeepSeek key",
    "model": "deepseek-v4-flash",
    "transport": "responses"
  },
  "vision_model": {
    "api_url": "https://api.siliconflow.cn/v1",
    "api_key": "你的 SiliconFlow key",
    "model": "Qwen/Qwen3-VL-32B-Instruct"
  }
}
```

## 2. 启动

```bash
# 改过前端后先构建（否则用旧的嵌入资源）
cd web && npm install && npm run build && cd ..

# 开发运行
cargo run --bin mistake-agent

# 只跑 kernel（CLI）
cargo run --bin sidecar
```

## 3. 界面操作

- **聊天**：底部输入框发消息，Enter 或「发送」。
- **批改作业**：点「作业」选择图片/PDF → 自动生成"请批改这份作业：<路径>"并发送 → Agent 调 `grading::upload`。
- **停止**：回答/批改过程中点「停止」立即中止（回合中发送按钮禁用，先停止再发新消息）。
- **思考过程**：模型推理增量默认折叠在"思考过程"卡片里，点击展开/折叠；不展示给学生也随时可查。
- **工具进度**：批改中底部显示"grading::upload：正在识别…"等进度。
- **错题本**：在聊天里问"错题本里有什么"，Agent 会调 `grading::list` 展示。

## 4. sidecar CLI 用法

```bash
printf '%s\n' '{"id":1,"method":"send_user_message","text":"你好"}' | cargo run --bin sidecar
```

stdout 输出 JSONL 帧（response + event），stderr 输出日志（WARN 及以上）。

## 5. 数据与产物

| 路径 | 内容 |
|---|---|
| settings.json | 模型配置与 key（用户独占写） |
| sessions/<key>.jsonl | 会话消息树（首行元数据） |
| mistakes/mistakes.json | 错题本 |
| audit/audit.jsonl | 审计（10MB 轮转） |
| logs/ | 分级诊断日志（10MB 轮转） |

## 6. 常见问题

- 发送后没反应：确认右上角状态为"就绪"（内核自检通过）；查看终端 stderr 是否有 sidecar 报错。
- 扫描版 PDF：提示不支持，请拍照成图片再上传。
- 模型报"余额不足/模型不可用"：检查对应 key 与官方模型状态。
