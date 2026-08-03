# 模型方案：主模型 + 视觉模型，双端点配置

v2 使用两个模型：主模型 deepseek-v4-flash 负责 agent loop 的调度与对话；视觉模型 qwen3-VL（硅基流动 SiliconFlow 提供）负责 OCR 与图片理解，由 grading 等插件经 ModelHandle 调用。两者均为 OpenAI 兼容端点，在 settings 中分别配置 API_URL 与 API_KEY；ModelHandle.complete 支持按用途选择模型（默认主模型）。
