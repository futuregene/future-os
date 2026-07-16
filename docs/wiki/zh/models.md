# 内置模型目录

> 自动生成于 2026-07-16T10:42:08 — 915 个模型，覆盖 30 个 Provider。
> 运行 `make generate-models` 更新。

## Provider 概览

| Provider | 模型数 | 示例模型 |
|---|---|---|
| OpenRouter | 272 | auto, grok-4-fast, grok-4.1-fast |
| vercel-ai-gateway | 162 | grok-4-fast-non-reasoning, grok-4-fast-reasoning, grok-4.1-fast-non-reasoning |
| Amazon Bedrock | 89 | meta.llama4-scout-17b-instruct-v1:0, us.meta.llama4-scout-17b-instruct-v1:0, writer.palmyra-x5-v1:0 |
| OpenAI | 44 | gpt-5.4-pro, gpt-5.5-pro, gpt-5.6-terra |
| google-vertex | 43 | gemini-2.0-flash, gemini-2.0-flash-lite, gemini-2.5-flash |
| azure-openai-responses | 41 | gpt-5.4, gpt-5.4-pro, gpt-5.5 |
| Google | 28 | gemini-2.0-flash, gemini-2.0-flash-lite, gemini-2.5-flash |
| Mistral | 27 | devstral-2512, devstral-medium-latest, mistral-large-2512 |
| github-copilot | 26 | gpt-5.1-codex, gpt-5.1-codex-max, gpt-5.1-codex-mini |
| xai | 25 | grok-4-1-fast, grok-4-1-fast-non-reasoning, grok-4-fast |
| Anthropic | 24 | claude-opus-4-6, claude-opus-4-7, claude-opus-4-8 |
| huggingface | 22 | DeepSeek-V4-Pro, Qwen3-235B-A22B-Thinking-2507, Qwen3-Coder-480B-A35B-Instruct |
| groq | 18 | kimi-k2-instruct-0905, deepseek-r1-distill-llama-70b, compound |
| zai | 13 | glm-4.6, glm-4.7, glm-5 |
| zhipuai | 12 | glm-4.6, glm-4.7, glm-5 |
| cloudflare-workers-ai | 8 | gemma-4-26b-a4b-it, kimi-k2.5, kimi-k2.6 |
| moonshotai | 7 | kimi-k2-0905-preview, kimi-k2-thinking, kimi-k2-thinking-turbo |
| moonshotai-cn | 7 | kimi-k2-0905-preview, kimi-k2-thinking, kimi-k2-thinking-turbo |
| minimax | 6 | MiniMax-M2.1, MiniMax-M2.5, MiniMax-M2.5-highspeed |
| minimax-cn | 6 | MiniMax-M2.1, MiniMax-M2.5, MiniMax-M2.5-highspeed |
| xiaomi | 5 | mimo-v2-pro, mimo-v2.5, mimo-v2.5-pro |
| xiaomi-token-plan-ams | 5 | mimo-v2-pro, mimo-v2.5, mimo-v2.5-pro |
| xiaomi-token-plan-cn | 5 | mimo-v2-pro, mimo-v2.5, mimo-v2.5-pro |
| xiaomi-token-plan-sgp | 5 | mimo-v2-pro, mimo-v2.5, mimo-v2.5-pro |
| cerebras | 4 | gpt-oss-120b, zai-glm-4.7, qwen-3-235b-a22b-instruct-2507 |
| DeepSeek | 4 | deepseek-chat, deepseek-reasoner, deepseek-v4-flash |
| kimi-coding | 2 | kimi-for-coding, kimi-k2-thinking |
| opencode | 2 | deepseek-v4-pro, kimi-k2.6 |
| opencode-go | 2 | deepseek-v4-pro, kimi-k2.6 |
| openai-codex | 1 | gpt-5.5 |

---

## 各 Provider 详情

### Amazon Bedrock

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 | 输入 ($/1M) | 输出 ($/1M) |
|---|---|---|---|---|---|---|---|
| `meta.llama4-scout-17b-instruct-v1:0` | Llama 4 Scout 17B Instruct | 4M | 16K | ✅ | — | $0.17 | $0.66 |
| `us.meta.llama4-scout-17b-instruct-v1:0` | Llama 4 Scout 17B Instruct (US) | 4M | 16K | ✅ | — | $0.17 | $0.66 |
| `writer.palmyra-x5-v1:0` | Palmyra X5 | 1M | 8K | — | ✅ | $0.60 | $6.00 |
| `anthropic.claude-opus-4-6-v1` | Claude Opus 4.6 | 1M | 128K | ✅ | ✅ | $5.00 | $25.00 |
| `anthropic.claude-opus-4-7` | Claude Opus 4.7 | 1M | 128K | ✅ | ✅ | $5.00 | $25.00 |
| `anthropic.claude-opus-4-8` | Claude Opus 4.8 | 1M | 128K | ✅ | ✅ | $5.00 | $25.00 |
| `anthropic.claude-sonnet-4-6` | Claude Sonnet 4.6 | 1M | 64K | ✅ | ✅ | $3.00 | $15.00 |
| `au.anthropic.claude-opus-4-6-v1` | AU Anthropic Claude Opus 4.6 | 1M | 128K | ✅ | ✅ | $16.50 | $82.50 |
| `au.anthropic.claude-sonnet-4-6` | AU Anthropic Claude Sonnet 4.6 | 1M | 128K | ✅ | ✅ | $3.30 | $16.50 |
| `eu.anthropic.claude-opus-4-6-v1` | Claude Opus 4.6 (EU) | 1M | 128K | ✅ | ✅ | $5.00 | $25.00 |
| `eu.anthropic.claude-opus-4-7` | Claude Opus 4.7 (EU) | 1M | 128K | ✅ | ✅ | $5.00 | $25.00 |
| `eu.anthropic.claude-opus-4-8` | Claude Opus 4.8 (EU) | 1M | 128K | ✅ | ✅ | $5.00 | $25.00 |
| `eu.anthropic.claude-sonnet-4-6` | Claude Sonnet 4.6 (EU) | 1M | 64K | ✅ | ✅ | $3.00 | $15.00 |
| `global.anthropic.claude-opus-4-6-v1` | Claude Opus 4.6 (Global) | 1M | 128K | ✅ | ✅ | $5.00 | $25.00 |
| `global.anthropic.claude-opus-4-7` | Claude Opus 4.7 (Global) | 1M | 128K | ✅ | ✅ | $5.00 | $25.00 |
| `global.anthropic.claude-opus-4-8` | Claude Opus 4.8 (Global) | 1M | 128K | ✅ | ✅ | $5.00 | $25.00 |
| `global.anthropic.claude-sonnet-4-6` | Claude Sonnet 4.6 (Global) | 1M | 64K | ✅ | ✅ | $3.00 | $15.00 |
| `jp.anthropic.claude-opus-4-7` | Claude Opus 4.7 (JP) | 1M | 128K | ✅ | ✅ | $5.00 | $25.00 |
| `jp.anthropic.claude-opus-4-8` | Claude Opus 4.8 (JP) | 1M | 128K | ✅ | ✅ | $5.00 | $25.00 |
| `jp.anthropic.claude-sonnet-4-6` | Claude Sonnet 4.6 (JP) | 1M | 64K | ✅ | ✅ | $3.00 | $15.00 |
| `meta.llama4-maverick-17b-instruct-v1:0` | Llama 4 Maverick 17B Instruct | 1M | 16K | ✅ | — | $0.24 | $0.97 |
| `us.anthropic.claude-opus-4-6-v1` | Claude Opus 4.6 (US) | 1M | 128K | ✅ | ✅ | $5.00 | $25.00 |
| `us.anthropic.claude-opus-4-7` | Claude Opus 4.7 (US) | 1M | 128K | ✅ | ✅ | $5.00 | $25.00 |
| `us.anthropic.claude-opus-4-8` | Claude Opus 4.8 (US) | 1M | 128K | ✅ | ✅ | $5.00 | $25.00 |
| `us.anthropic.claude-sonnet-4-6` | Claude Sonnet 4.6 (US) | 1M | 64K | ✅ | ✅ | $3.00 | $15.00 |
| `us.meta.llama4-maverick-17b-instruct-v1:0` | Llama 4 Maverick 17B Instruct (US) | 1M | 16K | ✅ | — | $0.24 | $0.97 |
| `amazon.nova-lite-v1:0` | Nova Lite | 300K | 8K | ✅ | — | $0.06 | $0.24 |
| `amazon.nova-pro-v1:0` | Nova Pro | 300K | 8K | ✅ | — | $0.80 | $3.20 |
| `nvidia.nemotron-super-3-120b` | NVIDIA Nemotron 3 Super 120B A12B | 262K | 131K | — | ✅ | $0.15 | $0.65 |
| `qwen.qwen3-235b-a22b-2507-v1:0` | Qwen3 235B A22B 2507 | 262K | 131K | — | — | $0.22 | $0.88 |
| `qwen.qwen3-coder-30b-a3b-v1:0` | Qwen3 Coder 30B A3B Instruct | 262K | 131K | — | — | $0.15 | $0.60 |
| `qwen.qwen3-next-80b-a3b` | Qwen/Qwen3-Next-80B-A3B-Instruct | 262K | 262K | — | — | $0.14 | $1.40 |
| `qwen.qwen3-vl-235b-a22b` | Qwen/Qwen3-VL-235B-A22B-Instruct | 262K | 262K | ✅ | — | $0.30 | $1.50 |
| `mistral.devstral-2-123b` | Devstral 2 123B | 256K | 8K | — | — | $0.40 | $2.00 |
| `mistral.ministral-3-3b-instruct` | Ministral 3 3B | 256K | 8K | ✅ | — | $0.10 | $0.10 |
| `mistral.mistral-large-3-675b-instruct` | Mistral Large 3 | 256K | 8K | ✅ | — | $0.50 | $1.50 |
| `moonshot.kimi-k2-thinking` | Kimi K2 Thinking | 256K | 256K | — | ✅ | $0.60 | $2.50 |
| `moonshotai.kimi-k2.5` | Kimi K2.5 | 256K | 256K | ✅ | ✅ | $0.60 | $3.00 |
| `minimax.minimax-m2.1` | MiniMax M2.1 | 205K | 131K | — | ✅ | $0.30 | $1.20 |
| `zai.glm-4.7` | GLM-4.7 | 205K | 131K | — | ✅ | $0.60 | $2.20 |
| `minimax.minimax-m2` | MiniMax M2 | 205K | 128K | — | ✅ | $0.30 | $1.20 |
| `google.gemma-3-27b-it` | Google Gemma 3 27B Instruct | 203K | 8K | ✅ | — | $0.12 | $0.20 |
| `zai.glm-5` | GLM-5 | 203K | 101K | — | ✅ | $1.00 | $3.20 |
| `anthropic.claude-haiku-4-5-20251001-v1:0` | Claude Haiku 4.5 | 200K | 64K | ✅ | ✅ | $1.00 | $5.00 |
| `anthropic.claude-opus-4-1-20250805-v1:0` | Claude Opus 4.1 | 200K | 32K | ✅ | ✅ | $15.00 | $75.00 |
| `anthropic.claude-opus-4-5-20251101-v1:0` | Claude Opus 4.5 | 200K | 64K | ✅ | ✅ | $5.00 | $25.00 |
| `anthropic.claude-sonnet-4-5-20250929-v1:0` | Claude Sonnet 4.5 | 200K | 64K | ✅ | ✅ | $3.00 | $15.00 |
| `au.anthropic.claude-haiku-4-5-20251001-v1:0` | Claude Haiku 4.5 (AU) | 200K | 64K | ✅ | ✅ | $1.00 | $5.00 |
| `au.anthropic.claude-sonnet-4-5-20250929-v1:0` | Claude Sonnet 4.5 (AU) | 200K | 64K | ✅ | ✅ | $3.00 | $15.00 |
| `eu.anthropic.claude-haiku-4-5-20251001-v1:0` | Claude Haiku 4.5 (EU) | 200K | 64K | ✅ | ✅ | $1.00 | $5.00 |
| `eu.anthropic.claude-opus-4-5-20251101-v1:0` | Claude Opus 4.5 (EU) | 200K | 64K | ✅ | ✅ | $5.00 | $25.00 |
| `eu.anthropic.claude-sonnet-4-5-20250929-v1:0` | Claude Sonnet 4.5 (EU) | 200K | 64K | ✅ | ✅ | $3.00 | $15.00 |
| `global.anthropic.claude-haiku-4-5-20251001-v1:0` | Claude Haiku 4.5 (Global) | 200K | 64K | ✅ | ✅ | $1.00 | $5.00 |
| `global.anthropic.claude-opus-4-5-20251101-v1:0` | Claude Opus 4.5 (Global) | 200K | 64K | ✅ | ✅ | $5.00 | $25.00 |
| `global.anthropic.claude-sonnet-4-5-20250929-v1:0` | Claude Sonnet 4.5 (Global) | 200K | 64K | ✅ | ✅ | $3.00 | $15.00 |
| `jp.anthropic.claude-sonnet-4-5-20250929-v1:0` | Claude Sonnet 4.5 (JP) | 200K | 64K | ✅ | ✅ | $3.00 | $15.00 |
| `us.anthropic.claude-haiku-4-5-20251001-v1:0` | Claude Haiku 4.5 (US) | 200K | 64K | ✅ | ✅ | $1.00 | $5.00 |
| `us.anthropic.claude-opus-4-1-20250805-v1:0` | Claude Opus 4.1 (US) | 200K | 32K | ✅ | ✅ | $15.00 | $75.00 |
| `us.anthropic.claude-opus-4-5-20251101-v1:0` | Claude Opus 4.5 (US) | 200K | 64K | ✅ | ✅ | $5.00 | $25.00 |
| `us.anthropic.claude-sonnet-4-5-20250929-v1:0` | Claude Sonnet 4.5 (US) | 200K | 64K | ✅ | ✅ | $3.00 | $15.00 |
| `zai.glm-4.7-flash` | GLM-4.7-Flash | 200K | 131K | — | ✅ | $0.07 | $0.40 |
| `minimax.minimax-m2.5` | MiniMax M2.5 | 197K | 98K | — | ✅ | $0.30 | $1.20 |
| `deepseek.v3-v1:0` | DeepSeek-V3.1 | 164K | 82K | — | ✅ | $0.58 | $1.68 |
| `deepseek.v3.2` | DeepSeek-V3.2 | 164K | 82K | — | ✅ | $0.62 | $1.85 |
| `qwen.qwen3-coder-480b-a35b-v1:0` | Qwen3 Coder 480B A35B Instruct | 131K | 66K | — | — | $0.22 | $1.80 |
| `qwen.qwen3-coder-next` | Qwen3 Coder Next | 131K | 66K | — | ✅ | $0.22 | $1.80 |
| `amazon.nova-2-lite-v1:0` | Nova 2 Lite | 128K | 4K | ✅ | — | $0.33 | $2.75 |
| `amazon.nova-micro-v1:0` | Nova Micro | 128K | 8K | — | — | $0.04 | $0.14 |
| `deepseek.r1-v1:0` | DeepSeek-R1 | 128K | 33K | — | ✅ | $1.35 | $5.40 |
| `google.gemma-3-4b-it` | Gemma 3 4B IT | 128K | 4K | ✅ | — | $0.04 | $0.08 |
| `meta.llama3-1-70b-instruct-v1:0` | Llama 3.1 70B Instruct | 128K | 4K | — | — | $0.72 | $0.72 |
| `meta.llama3-1-8b-instruct-v1:0` | Llama 3.1 8B Instruct | 128K | 4K | — | — | $0.22 | $0.22 |
| `meta.llama3-3-70b-instruct-v1:0` | Llama 3.3 70B Instruct | 128K | 4K | — | — | $0.72 | $0.72 |
| `mistral.magistral-small-2509` | Magistral Small 1.2 | 128K | 40K | ✅ | ✅ | $0.50 | $1.50 |
| `mistral.ministral-3-14b-instruct` | Ministral 14B 3.0 | 128K | 4K | — | — | $0.20 | $0.20 |
| `mistral.ministral-3-8b-instruct` | Ministral 3 8B | 128K | 4K | — | — | $0.15 | $0.15 |
| `mistral.pixtral-large-2502-v1:0` | Pixtral Large (25.02) | 128K | 8K | ✅ | — | $2.00 | $6.00 |
| `mistral.voxtral-mini-3b-2507` | Voxtral Mini 3B 2507 | 128K | 4K | — | — | $0.04 | $0.04 |
| `nvidia.nemotron-nano-12b-v2` | NVIDIA Nemotron Nano 12B v2 VL BF16 | 128K | 4K | ✅ | — | $0.20 | $0.60 |
| `nvidia.nemotron-nano-3-30b` | NVIDIA Nemotron Nano 3 30B | 128K | 4K | — | ✅ | $0.06 | $0.24 |
| `nvidia.nemotron-nano-9b-v2` | NVIDIA Nemotron Nano 9B v2 | 128K | 4K | — | — | $0.06 | $0.23 |
| `openai.gpt-oss-120b-1:0` | gpt-oss-120b | 128K | 4K | — | — | $0.15 | $0.60 |
| `openai.gpt-oss-20b-1:0` | gpt-oss-20b | 128K | 4K | — | — | $0.07 | $0.30 |
| `openai.gpt-oss-safeguard-120b` | GPT OSS Safeguard 120B | 128K | 4K | — | — | $0.15 | $0.60 |
| `openai.gpt-oss-safeguard-20b` | GPT OSS Safeguard 20B | 128K | 4K | — | — | $0.07 | $0.20 |
| `us.deepseek.r1-v1:0` | DeepSeek-R1 (US) | 128K | 33K | — | ✅ | $1.35 | $5.40 |
| `writer.palmyra-x4-v1:0` | Palmyra X4 | 123K | 8K | — | ✅ | $2.50 | $10.00 |
| `mistral.voxtral-small-24b-2507` | Voxtral Small 24B 2507 | 32K | 8K | — | — | $0.15 | $0.35 |
| `qwen.qwen3-32b-v1:0` | Qwen3 32B (dense) | 16K | 16K | — | ✅ | $0.15 | $0.60 |

### Anthropic

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 | 输入 ($/1M) | 输出 ($/1M) |
|---|---|---|---|---|---|---|---|
| `claude-opus-4-6` | Claude Opus 4.6 | 1M | 128K | ✅ | ✅ | $5.00 | $25.00 |
| `claude-opus-4-7` | Claude Opus 4.7 | 1M | 128K | ✅ | ✅ | $5.00 | $25.00 |
| `claude-opus-4-8` | Claude Opus 4.8 | 1M | 128K | ✅ | ✅ | $5.00 | $25.00 |
| `claude-sonnet-4-6` | Claude Sonnet 4.6 | 1M | 64K | ✅ | ✅ | $3.00 | $15.00 |
| `claude-3-5-haiku-20241022` | Claude Haiku 3.5 | 200K | 8K | ✅ | — | $0.80 | $4.00 |
| `claude-3-5-haiku-latest` | Claude Haiku 3.5 (latest) | 200K | 8K | ✅ | — | $0.80 | $4.00 |
| `claude-3-5-sonnet-20240620` | Claude Sonnet 3.5 | 200K | 8K | ✅ | — | $3.00 | $15.00 |
| `claude-3-5-sonnet-20241022` | Claude Sonnet 3.5 v2 | 200K | 8K | ✅ | — | $3.00 | $15.00 |
| `claude-3-7-sonnet-20250219` | Claude Sonnet 3.7 | 200K | 64K | ✅ | ✅ | $3.00 | $15.00 |
| `claude-3-haiku-20240307` | Claude Haiku 3 | 200K | 4K | ✅ | — | $0.25 | $1.25 |
| `claude-3-opus-20240229` | Claude Opus 3 | 200K | 4K | ✅ | — | $15.00 | $75.00 |
| `claude-3-sonnet-20240229` | Claude Sonnet 3 | 200K | 4K | ✅ | — | $3.00 | $15.00 |
| `claude-haiku-4-5` | Claude Haiku 4.5 (latest) | 200K | 64K | ✅ | ✅ | $1.00 | $5.00 |
| `claude-haiku-4-5-20251001` | Claude Haiku 4.5 | 200K | 64K | ✅ | ✅ | $1.00 | $5.00 |
| `claude-opus-4-0` | Claude Opus 4 (latest) | 200K | 32K | ✅ | ✅ | $15.00 | $75.00 |
| `claude-opus-4-1` | Claude Opus 4.1 (latest) | 200K | 32K | ✅ | ✅ | $15.00 | $75.00 |
| `claude-opus-4-1-20250805` | Claude Opus 4.1 | 200K | 32K | ✅ | ✅ | $15.00 | $75.00 |
| `claude-opus-4-20250514` | Claude Opus 4 | 200K | 32K | ✅ | ✅ | $15.00 | $75.00 |
| `claude-opus-4-5` | Claude Opus 4.5 (latest) | 200K | 64K | ✅ | ✅ | $5.00 | $25.00 |
| `claude-opus-4-5-20251101` | Claude Opus 4.5 | 200K | 64K | ✅ | ✅ | $5.00 | $25.00 |
| `claude-sonnet-4-0` | Claude Sonnet 4 (latest) | 200K | 64K | ✅ | ✅ | $3.00 | $15.00 |
| `claude-sonnet-4-20250514` | Claude Sonnet 4 | 200K | 64K | ✅ | ✅ | $3.00 | $15.00 |
| `claude-sonnet-4-5` | Claude Sonnet 4.5 (latest) | 200K | 64K | ✅ | ✅ | $3.00 | $15.00 |
| `claude-sonnet-4-5-20250929` | Claude Sonnet 4.5 | 200K | 64K | ✅ | ✅ | $3.00 | $15.00 |

### azure-openai-responses

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 | 输入 ($/1M) | 输出 ($/1M) |
|---|---|---|---|---|---|---|---|
| `gpt-5.4` | GPT-5.4 | 1M | 128K | ✅ | ✅ | $2.50 | $15.00 |
| `gpt-5.4-pro` | GPT-5.4 Pro | 1M | 128K | ✅ | ✅ | $30.00 | $180.00 |
| `gpt-5.5` | GPT-5.5 | 1M | 128K | ✅ | ✅ | $5.00 | $30.00 |
| `gpt-5.5-pro` | GPT-5.5 Pro | 1M | 128K | ✅ | ✅ | $30.00 | $180.00 |
| `gpt-4.1` | GPT-4.1 | 1M | 33K | ✅ | — | $2.00 | $8.00 |
| `gpt-4.1-mini` | GPT-4.1 mini | 1M | 33K | ✅ | — | $0.40 | $1.60 |
| `gpt-4.1-nano` | GPT-4.1 nano | 1M | 33K | ✅ | — | $0.10 | $0.40 |
| `gpt-5` | GPT-5 | 400K | 128K | ✅ | ✅ | $1.25 | $10.00 |
| `gpt-5-codex` | GPT-5-Codex | 400K | 128K | ✅ | ✅ | $1.25 | $10.00 |
| `gpt-5-mini` | GPT-5 Mini | 400K | 128K | ✅ | ✅ | $0.25 | $2.00 |
| `gpt-5-nano` | GPT-5 Nano | 400K | 128K | ✅ | ✅ | $0.05 | $0.40 |
| `gpt-5-pro` | GPT-5 Pro | 400K | 272K | ✅ | ✅ | $15.00 | $120.00 |
| `gpt-5.1` | GPT-5.1 | 400K | 128K | ✅ | ✅ | $1.25 | $10.00 |
| `gpt-5.1-codex` | GPT-5.1 Codex | 400K | 128K | ✅ | ✅ | $1.25 | $10.00 |
| `gpt-5.1-codex-max` | GPT-5.1 Codex Max | 400K | 128K | ✅ | ✅ | $1.25 | $10.00 |
| `gpt-5.1-codex-mini` | GPT-5.1 Codex mini | 400K | 128K | ✅ | ✅ | $0.25 | $2.00 |
| `gpt-5.2` | GPT-5.2 | 400K | 128K | ✅ | ✅ | $1.75 | $14.00 |
| `gpt-5.2-codex` | GPT-5.2 Codex | 400K | 128K | ✅ | ✅ | $1.75 | $14.00 |
| `gpt-5.2-pro` | GPT-5.2 Pro | 400K | 128K | ✅ | ✅ | $21.00 | $168.00 |
| `gpt-5.3-codex` | GPT-5.3 Codex | 400K | 128K | ✅ | ✅ | $1.75 | $14.00 |
| `gpt-5.4-mini` | GPT-5.4 mini | 400K | 128K | ✅ | ✅ | $0.75 | $4.50 |
| `gpt-5.4-nano` | GPT-5.4 nano | 400K | 128K | ✅ | ✅ | $0.20 | $1.25 |
| `o1` | o1 | 200K | 100K | ✅ | ✅ | $15.00 | $60.00 |
| `o1-pro` | o1-pro | 200K | 100K | ✅ | ✅ | $150.00 | $600.00 |
| `o3` | o3 | 200K | 100K | ✅ | ✅ | $2.00 | $8.00 |
| `o3-deep-research` | o3-deep-research | 200K | 100K | ✅ | ✅ | $10.00 | $40.00 |
| `o3-mini` | o3-mini | 200K | 100K | — | ✅ | $1.10 | $4.40 |
| `o3-pro` | o3-pro | 200K | 100K | ✅ | ✅ | $20.00 | $80.00 |
| `o4-mini` | o4-mini | 200K | 100K | ✅ | ✅ | $1.10 | $4.40 |
| `o4-mini-deep-research` | o4-mini-deep-research | 200K | 100K | ✅ | ✅ | $2.00 | $8.00 |
| `gpt-4-turbo` | GPT-4 Turbo | 128K | 4K | ✅ | — | $10.00 | $30.00 |
| `gpt-4o` | GPT-4o | 128K | 16K | ✅ | — | $2.50 | $10.00 |
| `gpt-4o-2024-05-13` | GPT-4o (2024-05-13) | 128K | 4K | ✅ | — | $5.00 | $15.00 |
| `gpt-4o-2024-08-06` | GPT-4o (2024-08-06) | 128K | 16K | ✅ | — | $2.50 | $10.00 |
| `gpt-4o-2024-11-20` | GPT-4o (2024-11-20) | 128K | 16K | ✅ | — | $2.50 | $10.00 |
| `gpt-4o-mini` | GPT-4o mini | 128K | 16K | ✅ | — | $0.15 | $0.60 |
| `gpt-5.1-chat-latest` | GPT-5.1 Chat | 128K | 16K | ✅ | ✅ | $1.25 | $10.00 |
| `gpt-5.2-chat-latest` | GPT-5.2 Chat | 128K | 16K | ✅ | ✅ | $1.75 | $14.00 |
| `gpt-5.3-chat-latest` | GPT-5.3 Chat (latest) | 128K | 16K | ✅ | — | $1.75 | $14.00 |
| `gpt-5.3-codex-spark` | GPT-5.3 Codex Spark | 128K | 32K | ✅ | ✅ | $1.75 | $14.00 |
| `gpt-4` | GPT-4 | 8K | 8K | — | — | $30.00 | $60.00 |

### cerebras

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 | 输入 ($/1M) | 输出 ($/1M) |
|---|---|---|---|---|---|---|---|
| `gpt-oss-120b` | GPT OSS 120B | 131K | 33K | — | ✅ | $0.25 | $0.69 |
| `zai-glm-4.7` | Z.AI GLM-4.7 | 131K | 40K | — | — | $2.25 | $2.75 |
| `qwen-3-235b-a22b-instruct-2507` | Qwen 3 235B Instruct | 131K | 32K | — | — | $0.60 | $1.20 |
| `llama3.1-8b` | Llama 3.1 8B | 32K | 8K | — | — | $0.10 | $0.10 |

### cloudflare-workers-ai

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 | 输入 ($/1M) | 输出 ($/1M) |
|---|---|---|---|---|---|---|---|
| `gemma-4-26b-a4b-it` | Gemma 4 26B A4B IT | 256K | 16K | ✅ | ✅ | $0.10 | $0.30 |
| `kimi-k2.5` | Kimi K2.5 | 256K | 256K | ✅ | ✅ | $0.60 | $3.00 |
| `kimi-k2.6` | Kimi K2.6 | 256K | 256K | ✅ | ✅ | $0.95 | $4.00 |
| `nemotron-3-120b-a12b` | Nemotron 3 Super 120B | 256K | 256K | — | ✅ | $0.50 | $1.50 |
| `glm-4.7-flash` | GLM-4.7-Flash | 131K | 131K | — | ✅ | $0.06 | $0.40 |
| `llama-4-scout-17b-16e-instruct` | Llama 4 Scout 17B 16E Instruct | 128K | 16K | ✅ | — | $0.27 | $0.85 |
| `gpt-oss-120b` | GPT OSS 120B | 128K | 16K | — | ✅ | $0.35 | $0.75 |
| `gpt-oss-20b` | GPT OSS 20B | 128K | 16K | — | ✅ | $0.20 | $0.30 |

### DeepSeek

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 | 输入 ($/1M) | 输出 ($/1M) |
|---|---|---|---|---|---|---|---|
| `deepseek-chat` | DeepSeek Chat | 1M | 384K | — | — | $0.14 | $0.28 |
| `deepseek-reasoner` | DeepSeek Reasoner | 1M | 384K | — | ✅ | $0.14 | $0.28 |
| `deepseek-v4-flash` | DeepSeek V4 Flash | 1M | 384K | — | ✅ | $0.14 | $0.28 |
| `deepseek-v4-pro` | DeepSeek V4 Pro | 1M | 384K | — | ✅ | $1.74 | $3.48 |

### github-copilot

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 | 输入 ($/1M) | 输出 ($/1M) |
|---|---|---|---|---|---|---|---|
| `gpt-5.1-codex` | GPT-5.1-Codex | 400K | 128K | ✅ | ✅ | - | - |
| `gpt-5.1-codex-max` | GPT-5.1-Codex-max | 400K | 128K | ✅ | ✅ | - | - |
| `gpt-5.1-codex-mini` | GPT-5.1-Codex-mini | 400K | 128K | ✅ | ✅ | - | - |
| `gpt-5.2-codex` | GPT-5.2-Codex | 400K | 128K | ✅ | ✅ | - | - |
| `gpt-5.3-codex` | GPT-5.3-Codex | 400K | 128K | ✅ | ✅ | - | - |
| `gpt-5.4` | GPT-5.4 | 400K | 128K | ✅ | ✅ | - | - |
| `gpt-5.4-mini` | GPT-5.4 Mini | 400K | 128K | ✅ | ✅ | - | - |
| `gpt-5.5` | GPT-5.5 | 400K | 128K | ✅ | ✅ | - | - |
| `gpt-5-mini` | GPT-5-mini | 264K | 64K | ✅ | ✅ | - | - |
| `gpt-5.1` | GPT-5.1 | 264K | 64K | ✅ | ✅ | - | - |
| `gpt-5.2` | GPT-5.2 | 264K | 64K | ✅ | ✅ | - | - |
| `claude-sonnet-4` | Claude Sonnet 4 | 216K | 16K | ✅ | ✅ | - | - |
| `claude-sonnet-4.6` | Claude Sonnet 4.6 | 200K | 32K | ✅ | ✅ | - | - |
| `claude-opus-4.5` | Claude Opus 4.5 | 160K | 32K | ✅ | ✅ | - | - |
| `claude-haiku-4.5` | Claude Haiku 4.5 | 144K | 32K | ✅ | ✅ | - | - |
| `claude-opus-4.6` | Claude Opus 4.6 | 144K | 64K | ✅ | ✅ | - | - |
| `claude-opus-4.7` | Claude Opus 4.7 | 144K | 64K | ✅ | ✅ | - | - |
| `claude-sonnet-4.5` | Claude Sonnet 4.5 | 144K | 32K | ✅ | ✅ | - | - |
| `gemini-2.5-pro` | Gemini 2.5 Pro | 128K | 64K | ✅ | — | - | - |
| `gemini-3-flash-preview` | Gemini 3 Flash | 128K | 64K | ✅ | ✅ | - | - |
| `gemini-3-pro-preview` | Gemini 3 Pro Preview | 128K | 64K | ✅ | ✅ | - | - |
| `gemini-3.1-pro-preview` | Gemini 3.1 Pro Preview | 128K | 64K | ✅ | ✅ | - | - |
| `gpt-4.1` | GPT-4.1 | 128K | 16K | ✅ | — | - | - |
| `gpt-4o` | GPT-4o | 128K | 4K | ✅ | — | - | - |
| `gpt-5` | GPT-5 | 128K | 128K | ✅ | ✅ | - | - |
| `grok-code-fast-1` | Grok Code Fast 1 | 128K | 64K | — | ✅ | - | - |

### Google

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 | 输入 ($/1M) | 输出 ($/1M) |
|---|---|---|---|---|---|---|---|
| `gemini-2.0-flash` | Gemini 2.0 Flash | 1M | 8K | ✅ | — | $0.10 | $0.40 |
| `gemini-2.0-flash-lite` | Gemini 2.0 Flash Lite | 1M | 8K | ✅ | — | $0.07 | $0.30 |
| `gemini-2.5-flash` | Gemini 2.5 Flash | 1M | 66K | ✅ | ✅ | $0.30 | $2.50 |
| `gemini-2.5-flash-lite` | Gemini 2.5 Flash Lite | 1M | 66K | ✅ | ✅ | $0.10 | $0.40 |
| `gemini-2.5-flash-lite-preview-06-17` | Gemini 2.5 Flash Lite Preview 06-17 | 1M | 66K | ✅ | ✅ | $0.10 | $0.40 |
| `gemini-2.5-flash-lite-preview-09-2025` | Gemini 2.5 Flash Lite Preview 09-25 | 1M | 66K | ✅ | ✅ | $0.10 | $0.40 |
| `gemini-2.5-flash-preview-04-17` | Gemini 2.5 Flash Preview 04-17 | 1M | 66K | ✅ | ✅ | $0.15 | $0.60 |
| `gemini-2.5-flash-preview-05-20` | Gemini 2.5 Flash Preview 05-20 | 1M | 66K | ✅ | ✅ | $0.15 | $0.60 |
| `gemini-2.5-flash-preview-09-2025` | Gemini 2.5 Flash Preview 09-25 | 1M | 66K | ✅ | ✅ | $0.30 | $2.50 |
| `gemini-2.5-pro` | Gemini 2.5 Pro | 1M | 66K | ✅ | ✅ | $1.25 | $10.00 |
| `gemini-2.5-pro-preview-05-06` | Gemini 2.5 Pro Preview 05-06 | 1M | 66K | ✅ | ✅ | $1.25 | $10.00 |
| `gemini-2.5-pro-preview-06-05` | Gemini 2.5 Pro Preview 06-05 | 1M | 66K | ✅ | ✅ | $1.25 | $10.00 |
| `gemini-3-flash-preview` | Gemini 3 Flash Preview | 1M | 66K | ✅ | ✅ | $0.50 | $3.00 |
| `gemini-3.1-flash-lite` | Gemini 3.1 Flash Lite | 1M | 66K | ✅ | ✅ | $0.25 | $1.50 |
| `gemini-3.1-flash-lite-preview` | Gemini 3.1 Flash Lite Preview | 1M | 66K | ✅ | ✅ | $0.25 | $1.50 |
| `gemini-3.1-pro-preview` | Gemini 3.1 Pro Preview | 1M | 66K | ✅ | ✅ | $2.00 | $12.00 |
| `gemini-3.1-pro-preview-customtools` | Gemini 3.1 Pro Preview Custom Tools | 1M | 66K | ✅ | ✅ | $2.00 | $12.00 |
| `gemini-flash-latest` | Gemini Flash Latest | 1M | 66K | ✅ | ✅ | $0.30 | $2.50 |
| `gemini-flash-lite-latest` | Gemini Flash-Lite Latest | 1M | 66K | ✅ | ✅ | $0.10 | $0.40 |
| `gemini-1.5-flash` | Gemini 1.5 Flash | 1M | 8K | ✅ | — | $0.07 | $0.30 |
| `gemini-1.5-flash-8b` | Gemini 1.5 Flash-8B | 1M | 8K | ✅ | — | $0.04 | $0.15 |
| `gemini-1.5-pro` | Gemini 1.5 Pro | 1M | 8K | ✅ | — | $1.25 | $5.00 |
| `gemini-3-pro-preview` | Gemini 3 Pro Preview | 1M | 64K | ✅ | ✅ | $2.00 | $12.00 |
| `gemma-4-26b-a4b-it` | Gemma 4 26B | 256K | 8K | ✅ | ✅ | - | - |
| `gemma-4-31b-it` | Gemma 4 31B | 256K | 8K | ✅ | ✅ | - | - |
| `gemini-live-2.5-flash-preview-native-audio` | Gemini Live 2.5 Flash Preview Native Audio | 131K | 66K | — | ✅ | $0.50 | $2.00 |
| `gemma-3-27b-it` | Gemma 3 27B | 131K | 8K | ✅ | — | - | - |
| `gemini-live-2.5-flash` | Gemini Live 2.5 Flash | 128K | 8K | ✅ | ✅ | $0.50 | $2.00 |

### google-vertex

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 | 输入 ($/1M) | 输出 ($/1M) |
|---|---|---|---|---|---|---|---|
| `gemini-2.0-flash` | Gemini 2.0 Flash | 1M | 8K | ✅ | — | $0.15 | $0.60 |
| `gemini-2.0-flash-lite` | Gemini 2.0 Flash Lite | 1M | 8K | ✅ | — | $0.07 | $0.30 |
| `gemini-2.5-flash` | Gemini 2.5 Flash | 1M | 66K | ✅ | ✅ | $0.30 | $2.50 |
| `gemini-2.5-flash-lite` | Gemini 2.5 Flash Lite | 1M | 66K | ✅ | ✅ | $0.10 | $0.40 |
| `gemini-2.5-flash-lite-preview-09-2025` | Gemini 2.5 Flash Lite Preview 09-25 | 1M | 66K | ✅ | ✅ | $0.10 | $0.40 |
| `gemini-2.5-flash-preview-04-17` | Gemini 2.5 Flash Preview 04-17 | 1M | 66K | ✅ | ✅ | $0.15 | $0.60 |
| `gemini-2.5-flash-preview-05-20` | Gemini 2.5 Flash Preview 05-20 | 1M | 66K | ✅ | ✅ | $0.15 | $0.60 |
| `gemini-2.5-flash-preview-09-2025` | Gemini 2.5 Flash Preview 09-25 | 1M | 66K | ✅ | ✅ | $0.30 | $2.50 |
| `gemini-2.5-pro` | Gemini 2.5 Pro | 1M | 66K | ✅ | ✅ | $1.25 | $10.00 |
| `gemini-2.5-pro-preview-05-06` | Gemini 2.5 Pro Preview 05-06 | 1M | 66K | ✅ | ✅ | $1.25 | $10.00 |
| `gemini-2.5-pro-preview-06-05` | Gemini 2.5 Pro Preview 06-05 | 1M | 66K | ✅ | ✅ | $1.25 | $10.00 |
| `gemini-3-flash-preview` | Gemini 3 Flash Preview | 1M | 66K | ✅ | ✅ | $0.50 | $3.00 |
| `gemini-3-pro-preview` | Gemini 3 Pro Preview | 1M | 66K | ✅ | ✅ | $2.00 | $12.00 |
| `gemini-3.1-flash-lite` | Gemini 3.1 Flash Lite | 1M | 66K | ✅ | ✅ | $0.25 | $1.50 |
| `gemini-3.1-flash-lite-preview` | Gemini 3.1 Flash Lite Preview | 1M | 66K | ✅ | ✅ | $0.25 | $1.50 |
| `gemini-3.1-pro-preview` | Gemini 3.1 Pro Preview | 1M | 66K | ✅ | ✅ | $2.00 | $12.00 |
| `gemini-3.1-pro-preview-customtools` | Gemini 3.1 Pro Preview Custom Tools | 1M | 66K | ✅ | ✅ | $2.00 | $12.00 |
| `gemini-flash-latest` | Gemini Flash Latest | 1M | 66K | ✅ | ✅ | $0.30 | $2.50 |
| `gemini-flash-lite-latest` | Gemini Flash-Lite Latest | 1M | 66K | ✅ | ✅ | $0.10 | $0.40 |
| `claude-opus-4-6@default` | Claude Opus 4.6 | 1M | 128K | ✅ | ✅ | $5.00 | $25.00 |
| `claude-opus-4-7@default` | Claude Opus 4.7 | 1M | 128K | ✅ | ✅ | $5.00 | $25.00 |
| `claude-opus-4-8@default` | Claude Opus 4.8 | 1M | 128K | ✅ | ✅ | $5.00 | $25.00 |
| `llama-4-maverick-17b-128e-instruct-maas` | Llama 4 Maverick 17B 128E Instruct | 524K | 8K | ✅ | — | $0.35 | $1.15 |
| `kimi-k2-thinking-maas` | Kimi K2 Thinking | 262K | 262K | — | ✅ | $0.60 | $2.50 |
| `qwen3-235b-a22b-instruct-2507-maas` | Qwen3 235B A22B Instruct | 262K | 16K | — | ✅ | $0.22 | $0.88 |
| `glm-5-maas` | GLM-5 | 203K | 131K | — | ✅ | $1.00 | $3.20 |
| `claude-3-5-haiku@20241022` | Claude Haiku 3.5 | 200K | 8K | ✅ | — | $0.80 | $4.00 |
| `claude-3-5-sonnet@20241022` | Claude Sonnet 3.5 v2 | 200K | 8K | ✅ | — | $3.00 | $15.00 |
| `claude-3-7-sonnet@20250219` | Claude Sonnet 3.7 | 200K | 64K | ✅ | ✅ | $3.00 | $15.00 |
| `claude-haiku-4-5@20251001` | Claude Haiku 4.5 | 200K | 64K | ✅ | ✅ | $1.00 | $5.00 |
| `claude-opus-4-1@20250805` | Claude Opus 4.1 | 200K | 32K | ✅ | ✅ | $15.00 | $75.00 |
| `claude-opus-4-5@20251101` | Claude Opus 4.5 | 200K | 64K | ✅ | ✅ | $5.00 | $25.00 |
| `claude-opus-4@20250514` | Claude Opus 4 | 200K | 32K | ✅ | ✅ | $15.00 | $75.00 |
| `claude-sonnet-4-5@20250929` | Claude Sonnet 4.5 | 200K | 64K | ✅ | ✅ | $3.00 | $15.00 |
| `claude-sonnet-4-6@default` | Claude Sonnet 4.6 | 200K | 64K | ✅ | ✅ | $3.00 | $15.00 |
| `claude-sonnet-4@20250514` | Claude Sonnet 4 | 200K | 64K | ✅ | ✅ | $3.00 | $15.00 |
| `glm-4.7-maas` | GLM-4.7 | 200K | 128K | — | ✅ | $0.60 | $2.20 |
| `deepseek-v3.1-maas` | DeepSeek V3.1 | 164K | 33K | — | ✅ | $0.60 | $1.70 |
| `deepseek-v3.2-maas` | DeepSeek V3.2 | 164K | 66K | — | ✅ | $0.56 | $1.68 |
| `gpt-oss-120b-maas` | GPT OSS 120B | 131K | 33K | — | ✅ | $0.09 | $0.36 |
| `gpt-oss-20b-maas` | GPT OSS 20B | 131K | 33K | — | ✅ | $0.07 | $0.25 |
| `llama-3.3-70b-instruct-maas` | Llama 3.3 70B Instruct | 128K | 8K | — | — | $0.72 | $0.72 |
| `gemini-2.5-flash-lite-preview-06-17` | Gemini 2.5 Flash Lite Preview 06-17 | 66K | 66K | ✅ | ✅ | $0.10 | $0.40 |

### groq

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 | 输入 ($/1M) | 输出 ($/1M) |
|---|---|---|---|---|---|---|---|
| `kimi-k2-instruct-0905` | Kimi K2 Instruct 0905 | 262K | 16K | — | — | $1.00 | $3.00 |
| `deepseek-r1-distill-llama-70b` | DeepSeek R1 Distill Llama 70B | 131K | 8K | — | ✅ | $0.75 | $0.99 |
| `compound` | Compound | 131K | 8K | — | ✅ | - | - |
| `compound-mini` | Compound Mini | 131K | 8K | — | ✅ | - | - |
| `llama-3.1-8b-instant` | Llama 3.1 8B Instant | 131K | 131K | — | — | $0.05 | $0.08 |
| `llama-3.3-70b-versatile` | Llama 3.3 70B Versatile | 131K | 33K | — | — | $0.59 | $0.79 |
| `llama-4-maverick-17b-128e-instruct` | Llama 4 Maverick 17B | 131K | 8K | ✅ | — | $0.20 | $0.60 |
| `llama-4-scout-17b-16e-instruct` | Llama 4 Scout 17B | 131K | 8K | ✅ | — | $0.11 | $0.34 |
| `kimi-k2-instruct` | Kimi K2 Instruct | 131K | 16K | — | — | $1.00 | $3.00 |
| `gpt-oss-120b` | GPT OSS 120B | 131K | 66K | — | ✅ | $0.15 | $0.60 |
| `gpt-oss-20b` | GPT OSS 20B | 131K | 66K | — | ✅ | $0.07 | $0.30 |
| `gpt-oss-safeguard-20b` | Safety GPT OSS 20B | 131K | 66K | — | ✅ | $0.07 | $0.30 |
| `qwen-qwq-32b` | Qwen QwQ 32B | 131K | 16K | — | ✅ | $0.29 | $0.39 |
| `qwen3-32b` | Qwen3 32B | 131K | 41K | — | ✅ | $0.29 | $0.59 |
| `mistral-saba-24b` | Mistral Saba 24B | 33K | 33K | — | — | $0.79 | $0.79 |
| `gemma2-9b-it` | Gemma 2 9B | 8K | 8K | — | — | $0.20 | $0.20 |
| `llama3-70b-8192` | Llama 3 70B | 8K | 8K | — | — | $0.59 | $0.79 |
| `llama3-8b-8192` | Llama 3 8B | 8K | 8K | — | — | $0.05 | $0.08 |

### huggingface

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 | 输入 ($/1M) | 输出 ($/1M) |
|---|---|---|---|---|---|---|---|
| `DeepSeek-V4-Pro` | DeepSeek V4 Pro | 1M | 393K | — | ✅ | $1.74 | $3.48 |
| `Qwen3-235B-A22B-Thinking-2507` | Qwen3-235B-A22B-Thinking-2507 | 262K | 131K | — | ✅ | $0.30 | $3.00 |
| `Qwen3-Coder-480B-A35B-Instruct` | Qwen3-Coder-480B-A35B-Instruct | 262K | 67K | — | — | $2.00 | $2.00 |
| `Qwen3-Coder-Next` | Qwen3-Coder-Next | 262K | 66K | — | — | $0.20 | $1.50 |
| `Qwen3-Next-80B-A3B-Instruct` | Qwen3-Next-80B-A3B-Instruct | 262K | 67K | — | — | $0.25 | $1.00 |
| `Qwen3-Next-80B-A3B-Thinking` | Qwen3-Next-80B-A3B-Thinking | 262K | 131K | — | — | $0.30 | $2.00 |
| `Qwen3.5-397B-A17B` | Qwen3.5-397B-A17B | 262K | 33K | ✅ | ✅ | $0.60 | $3.60 |
| `MiMo-V2-Flash` | MiMo-V2-Flash | 262K | 4K | — | ✅ | $0.10 | $0.30 |
| `Kimi-K2-Instruct-0905` | Kimi-K2-Instruct-0905 | 262K | 16K | — | — | $1.00 | $3.00 |
| `Kimi-K2-Thinking` | Kimi-K2-Thinking | 262K | 262K | — | ✅ | $0.60 | $2.50 |
| `Kimi-K2.5` | Kimi-K2.5 | 262K | 262K | ✅ | ✅ | $0.60 | $3.00 |
| `Kimi-K2.6` | Kimi-K2.6 | 262K | 262K | ✅ | ✅ | $0.95 | $4.00 |
| `MiniMax-M2.1` | MiniMax-M2.1 | 205K | 131K | — | ✅ | $0.30 | $1.20 |
| `MiniMax-M2.5` | MiniMax-M2.5 | 205K | 131K | — | ✅ | $0.30 | $1.20 |
| `MiniMax-M2.7` | MiniMax-M2.7 | 205K | 131K | — | ✅ | $0.30 | $1.20 |
| `GLM-4.7` | GLM-4.7 | 205K | 131K | — | ✅ | $0.60 | $2.20 |
| `GLM-5` | GLM-5 | 203K | 131K | — | ✅ | $1.00 | $3.20 |
| `GLM-5.1` | GLM-5.1 | 203K | 131K | — | ✅ | $1.00 | $3.20 |
| `GLM-4.7-Flash` | GLM-4.7-Flash | 200K | 128K | — | ✅ | - | - |
| `DeepSeek-R1-0528` | DeepSeek-R1-0528 | 164K | 164K | — | ✅ | $3.00 | $5.00 |
| `DeepSeek-V3.2` | DeepSeek-V3.2 | 164K | 66K | — | ✅ | $0.28 | $0.40 |
| `Kimi-K2-Instruct` | Kimi-K2-Instruct | 131K | 16K | — | — | $1.00 | $3.00 |

### kimi-coding

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 | 输入 ($/1M) | 输出 ($/1M) |
|---|---|---|---|---|---|---|---|
| `kimi-for-coding` | Kimi For Coding | 262K | 33K | — | ✅ | - | - |
| `kimi-k2-thinking` | Kimi K2 Thinking | 262K | 33K | — | ✅ | - | - |

### minimax

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 | 输入 ($/1M) | 输出 ($/1M) |
|---|---|---|---|---|---|---|---|
| `MiniMax-M2.1` | MiniMax-M2.1 | 205K | 131K | — | ✅ | $0.30 | $1.20 |
| `MiniMax-M2.5` | MiniMax-M2.5 | 205K | 131K | — | ✅ | $0.30 | $1.20 |
| `MiniMax-M2.5-highspeed` | MiniMax-M2.5-highspeed | 205K | 131K | — | ✅ | $0.60 | $2.40 |
| `MiniMax-M2.7` | MiniMax-M2.7 | 205K | 131K | — | ✅ | $0.30 | $1.20 |
| `MiniMax-M2.7-highspeed` | MiniMax-M2.7-highspeed | 205K | 131K | — | ✅ | $0.60 | $2.40 |
| `MiniMax-M2` | MiniMax-M2 | 197K | 128K | — | ✅ | $0.30 | $1.20 |

### minimax-cn

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 | 输入 ($/1M) | 输出 ($/1M) |
|---|---|---|---|---|---|---|---|
| `MiniMax-M2.1` | MiniMax-M2.1 | 205K | 131K | — | ✅ | $0.30 | $1.20 |
| `MiniMax-M2.5` | MiniMax-M2.5 | 205K | 131K | — | ✅ | $0.30 | $1.20 |
| `MiniMax-M2.5-highspeed` | MiniMax-M2.5-highspeed | 205K | 131K | — | ✅ | $0.60 | $2.40 |
| `MiniMax-M2.7` | MiniMax-M2.7 | 205K | 131K | — | ✅ | $0.30 | $1.20 |
| `MiniMax-M2.7-highspeed` | MiniMax-M2.7-highspeed | 205K | 131K | — | ✅ | $0.60 | $2.40 |
| `MiniMax-M2` | MiniMax-M2 | 197K | 128K | — | ✅ | $0.30 | $1.20 |

### Mistral

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 | 输入 ($/1M) | 输出 ($/1M) |
|---|---|---|---|---|---|---|---|
| `devstral-2512` | Devstral 2 | 262K | 262K | — | — | $0.40 | $2.00 |
| `devstral-medium-latest` | Devstral 2 (latest) | 262K | 262K | — | — | $0.40 | $2.00 |
| `mistral-large-2512` | Mistral Large 3 | 262K | 262K | ✅ | — | $0.50 | $1.50 |
| `mistral-large-latest` | Mistral Large (latest) | 262K | 262K | ✅ | — | $0.50 | $1.50 |
| `mistral-medium-2508` | Mistral Medium 3.1 | 262K | 262K | ✅ | — | $0.40 | $2.00 |
| `mistral-medium-2604` | Mistral Medium 3.5 | 262K | 262K | ✅ | ✅ | $1.50 | $7.50 |
| `mistral-medium-latest` | Mistral Medium (latest) | 262K | 262K | ✅ | ✅ | $1.50 | $7.50 |
| `codestral-latest` | Codestral (latest) | 256K | 4K | — | — | $0.30 | $0.90 |
| `labs-devstral-small-2512` | Devstral Small 2 | 256K | 256K | ✅ | — | - | - |
| `mistral-small-2603` | Mistral Small 4 | 256K | 256K | ✅ | ✅ | $0.15 | $0.60 |
| `mistral-small-latest` | Mistral Small (latest) | 256K | 256K | ✅ | ✅ | $0.15 | $0.60 |
| `mistral-large-2411` | Mistral Large 2.1 | 131K | 16K | — | — | $2.00 | $6.00 |
| `mistral-medium-2505` | Mistral Medium 3 | 131K | 131K | ✅ | — | $0.40 | $2.00 |
| `devstral-medium-2507` | Devstral Medium | 128K | 128K | — | — | $0.40 | $2.00 |
| `devstral-small-2505` | Devstral Small 2505 | 128K | 128K | — | — | $0.10 | $0.30 |
| `devstral-small-2507` | Devstral Small | 128K | 128K | — | — | $0.10 | $0.30 |
| `magistral-medium-latest` | Magistral Medium (latest) | 128K | 16K | — | ✅ | $2.00 | $5.00 |
| `magistral-small` | Magistral Small | 128K | 128K | — | ✅ | $0.50 | $1.50 |
| `ministral-3b-latest` | Ministral 3B (latest) | 128K | 128K | — | — | $0.04 | $0.04 |
| `ministral-8b-latest` | Ministral 8B (latest) | 128K | 128K | — | — | $0.10 | $0.10 |
| `mistral-nemo` | Mistral Nemo | 128K | 128K | — | — | $0.15 | $0.15 |
| `mistral-small-2506` | Mistral Small 3.2 | 128K | 16K | ✅ | — | $0.10 | $0.30 |
| `pixtral-12b` | Pixtral 12B | 128K | 128K | ✅ | — | $0.15 | $0.15 |
| `pixtral-large-latest` | Pixtral Large (latest) | 128K | 128K | ✅ | — | $2.00 | $6.00 |
| `open-mixtral-8x22b` | Mixtral 8x22B | 64K | 64K | — | — | $2.00 | $6.00 |
| `open-mixtral-8x7b` | Mixtral 8x7B | 32K | 32K | — | — | $0.70 | $0.70 |
| `open-mistral-7b` | Mistral 7B | 8K | 8K | — | — | $0.25 | $0.25 |

### moonshotai

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 | 输入 ($/1M) | 输出 ($/1M) |
|---|---|---|---|---|---|---|---|
| `kimi-k2-0905-preview` | Kimi K2 0905 | 262K | 262K | — | — | $0.60 | $2.50 |
| `kimi-k2-thinking` | Kimi K2 Thinking | 262K | 262K | — | ✅ | $0.60 | $2.50 |
| `kimi-k2-thinking-turbo` | Kimi K2 Thinking Turbo | 262K | 262K | — | ✅ | $1.15 | $8.00 |
| `kimi-k2-turbo-preview` | Kimi K2 Turbo | 262K | 262K | — | — | $2.40 | $10.00 |
| `kimi-k2.5` | Kimi K2.5 | 262K | 262K | ✅ | ✅ | $0.60 | $3.00 |
| `kimi-k2.6` | Kimi K2.6 | 262K | 262K | ✅ | ✅ | $0.95 | $4.00 |
| `kimi-k2-0711-preview` | Kimi K2 0711 | 131K | 16K | — | — | $0.60 | $2.50 |

### moonshotai-cn

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 | 输入 ($/1M) | 输出 ($/1M) |
|---|---|---|---|---|---|---|---|
| `kimi-k2-0905-preview` | Kimi K2 0905 | 262K | 262K | — | — | $0.60 | $2.50 |
| `kimi-k2-thinking` | Kimi K2 Thinking | 262K | 262K | — | ✅ | $0.60 | $2.50 |
| `kimi-k2-thinking-turbo` | Kimi K2 Thinking Turbo | 262K | 262K | — | ✅ | $1.15 | $8.00 |
| `kimi-k2-turbo-preview` | Kimi K2 Turbo | 262K | 262K | — | — | $2.40 | $10.00 |
| `kimi-k2.5` | Kimi K2.5 | 262K | 262K | ✅ | ✅ | $0.60 | $3.00 |
| `kimi-k2.6` | Kimi K2.6 | 262K | 262K | ✅ | ✅ | $0.95 | $4.00 |
| `kimi-k2-0711-preview` | Kimi K2 0711 | 131K | 16K | — | — | $0.60 | $2.50 |

### OpenAI

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 | 输入 ($/1M) | 输出 ($/1M) |
|---|---|---|---|---|---|---|---|
| `gpt-5.4-pro` | GPT-5.4 Pro | 1M | 128K | ✅ | ✅ | $30.00 | $180.00 |
| `gpt-5.5-pro` | GPT-5.5 Pro | 1M | 128K | ✅ | ✅ | $30.00 | $180.00 |
| `gpt-5.6-terra` | GPT-5.6 Terra | 1M | 128K | ✅ | ✅ | $30.00 | $180.00 |
| `gpt-4.1` | GPT-4.1 | 1M | 33K | ✅ | — | $2.00 | $8.00 |
| `gpt-4.1-mini` | GPT-4.1 mini | 1M | 33K | ✅ | — | $0.40 | $1.60 |
| `gpt-4.1-nano` | GPT-4.1 nano | 1M | 33K | ✅ | — | $0.10 | $0.40 |
| `gpt-5` | GPT-5 | 400K | 128K | ✅ | ✅ | $1.25 | $10.00 |
| `gpt-5-codex` | GPT-5-Codex | 400K | 128K | ✅ | ✅ | $1.25 | $10.00 |
| `gpt-5-mini` | GPT-5 Mini | 400K | 128K | ✅ | ✅ | $0.25 | $2.00 |
| `gpt-5-nano` | GPT-5 Nano | 400K | 128K | ✅ | ✅ | $0.05 | $0.40 |
| `gpt-5-pro` | GPT-5 Pro | 400K | 272K | ✅ | ✅ | $15.00 | $120.00 |
| `gpt-5.1` | GPT-5.1 | 400K | 128K | ✅ | ✅ | $1.25 | $10.00 |
| `gpt-5.1-codex` | GPT-5.1 Codex | 400K | 128K | ✅ | ✅ | $1.25 | $10.00 |
| `gpt-5.1-codex-max` | GPT-5.1 Codex Max | 400K | 128K | ✅ | ✅ | $1.25 | $10.00 |
| `gpt-5.1-codex-mini` | GPT-5.1 Codex mini | 400K | 128K | ✅ | ✅ | $0.25 | $2.00 |
| `gpt-5.2` | GPT-5.2 | 400K | 128K | ✅ | ✅ | $1.75 | $14.00 |
| `gpt-5.2-codex` | GPT-5.2 Codex | 400K | 128K | ✅ | ✅ | $1.75 | $14.00 |
| `gpt-5.2-pro` | GPT-5.2 Pro | 400K | 128K | ✅ | ✅ | $21.00 | $168.00 |
| `gpt-5.3-codex` | GPT-5.3 Codex | 400K | 128K | ✅ | ✅ | $1.75 | $14.00 |
| `gpt-5.4-mini` | GPT-5.4 mini | 400K | 128K | ✅ | ✅ | $0.75 | $4.50 |
| `gpt-5.4-nano` | GPT-5.4 nano | 400K | 128K | ✅ | ✅ | $0.20 | $1.25 |
| `gpt-5.4` | GPT-5.4 | 272K | 128K | ✅ | ✅ | $2.50 | $15.00 |
| `gpt-5.5` | GPT-5.5 | 272K | 128K | ✅ | ✅ | $5.00 | $30.00 |
| `gpt-5.6-luna` | GPT-5.6 Luna | 272K | 128K | ✅ | ✅ | $3.00 | $18.00 |
| `gpt-5.6-sol` | GPT-5.6 Sol | 272K | 128K | ✅ | ✅ | $5.00 | $30.00 |
| `o1` | o1 | 200K | 100K | ✅ | ✅ | $15.00 | $60.00 |
| `o1-pro` | o1-pro | 200K | 100K | ✅ | ✅ | $150.00 | $600.00 |
| `o3` | o3 | 200K | 100K | ✅ | ✅ | $2.00 | $8.00 |
| `o3-deep-research` | o3-deep-research | 200K | 100K | ✅ | ✅ | $10.00 | $40.00 |
| `o3-mini` | o3-mini | 200K | 100K | — | ✅ | $1.10 | $4.40 |
| `o3-pro` | o3-pro | 200K | 100K | ✅ | ✅ | $20.00 | $80.00 |
| `o4-mini` | o4-mini | 200K | 100K | ✅ | ✅ | $1.10 | $4.40 |
| `o4-mini-deep-research` | o4-mini-deep-research | 200K | 100K | ✅ | ✅ | $2.00 | $8.00 |
| `gpt-4-turbo` | GPT-4 Turbo | 128K | 4K | ✅ | — | $10.00 | $30.00 |
| `gpt-4o` | GPT-4o | 128K | 16K | ✅ | — | $2.50 | $10.00 |
| `gpt-4o-2024-05-13` | GPT-4o (2024-05-13) | 128K | 4K | ✅ | — | $5.00 | $15.00 |
| `gpt-4o-2024-08-06` | GPT-4o (2024-08-06) | 128K | 16K | ✅ | — | $2.50 | $10.00 |
| `gpt-4o-2024-11-20` | GPT-4o (2024-11-20) | 128K | 16K | ✅ | — | $2.50 | $10.00 |
| `gpt-4o-mini` | GPT-4o mini | 128K | 16K | ✅ | — | $0.15 | $0.60 |
| `gpt-5.1-chat-latest` | GPT-5.1 Chat | 128K | 16K | ✅ | ✅ | $1.25 | $10.00 |
| `gpt-5.2-chat-latest` | GPT-5.2 Chat | 128K | 16K | ✅ | ✅ | $1.75 | $14.00 |
| `gpt-5.3-chat-latest` | GPT-5.3 Chat (latest) | 128K | 16K | ✅ | — | $1.75 | $14.00 |
| `gpt-5.3-codex-spark` | GPT-5.3 Codex Spark | 128K | 32K | ✅ | ✅ | $1.75 | $14.00 |
| `gpt-4` | GPT-4 | 8K | 8K | — | — | $30.00 | $60.00 |

### openai-codex

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 | 输入 ($/1M) | 输出 ($/1M) |
|---|---|---|---|---|---|---|---|
| `gpt-5.5` | .into() | 272K | 128K | ✅ | ✅ | - | - |

### opencode

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 | 输入 ($/1M) | 输出 ($/1M) |
|---|---|---|---|---|---|---|---|
| `deepseek-v4-pro` | .into() | 1M | 66K | ✅ | ✅ | - | - |
| `kimi-k2.6` | .into() | 128K | 66K | — | — | - | - |

### opencode-go

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 | 输入 ($/1M) | 输出 ($/1M) |
|---|---|---|---|---|---|---|---|
| `deepseek-v4-pro` | .into() | 1M | 66K | ✅ | ✅ | - | - |
| `kimi-k2.6` | .into() | 128K | 66K | — | — | - | - |

### OpenRouter

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 | 输入 ($/1M) | 输出 ($/1M) |
|---|---|---|---|---|---|---|---|
| `auto` | Auto Router | 2M | 4K | ✅ | ✅ | $-1000000.00 | $-1000000.00 |
| `grok-4-fast` | xAI: Grok 4 Fast | 2M | 30K | ✅ | ✅ | $0.20 | $0.50 |
| `grok-4.1-fast` | xAI: Grok 4.1 Fast | 2M | 30K | ✅ | ✅ | $0.20 | $0.50 |
| `grok-4.20` | xAI: Grok 4.20 | 2M | 4K | ✅ | ✅ | $1.25 | $2.50 |
| `gpt-5.4` | OpenAI: GPT-5.4 | 1M | 128K | ✅ | ✅ | $2.50 | $15.00 |
| `gpt-5.4-pro` | OpenAI: GPT-5.4 Pro | 1M | 128K | ✅ | ✅ | $30.00 | $180.00 |
| `gpt-5.5` | OpenAI: GPT-5.5 | 1M | 128K | ✅ | ✅ | $5.00 | $30.00 |
| `gpt-5.5-pro` | OpenAI: GPT-5.5 Pro | 1M | 128K | ✅ | ✅ | $30.00 | $180.00 |
| `gpt-latest` | OpenAI GPT Latest | 1M | 128K | ✅ | ✅ | $5.00 | $30.00 |
| `owl-alpha` | Owl Alpha | 1M | 262K | — | — | - | - |
| `deepseek-v4-flash` | DeepSeek: DeepSeek V4 Flash | 1M | 384K | — | ✅ | $0.14 | $0.28 |
| `deepseek-v4-pro` | DeepSeek: DeepSeek V4 Pro | 1M | 384K | — | ✅ | $0.43 | $0.87 |
| `gemini-2.0-flash-001` | Google: Gemini 2.0 Flash | 1M | 8K | ✅ | — | $0.10 | $0.40 |
| `gemini-2.0-flash-lite-001` | Google: Gemini 2.0 Flash Lite | 1M | 8K | ✅ | — | $0.07 | $0.30 |
| `gemini-2.5-flash` | Google: Gemini 2.5 Flash | 1M | 66K | ✅ | ✅ | $0.30 | $2.50 |
| `gemini-2.5-flash-lite` | Google: Gemini 2.5 Flash Lite | 1M | 66K | ✅ | ✅ | $0.10 | $0.40 |
| `gemini-2.5-flash-lite-preview-09-2025` | Google: Gemini 2.5 Flash Lite Preview 09-2025 | 1M | 66K | ✅ | ✅ | $0.10 | $0.40 |
| `gemini-2.5-pro` | Google: Gemini 2.5 Pro | 1M | 66K | ✅ | ✅ | $1.25 | $10.00 |
| `gemini-2.5-pro-preview` | Google: Gemini 2.5 Pro Preview 06-05 | 1M | 66K | ✅ | ✅ | $1.25 | $10.00 |
| `gemini-2.5-pro-preview-05-06` | Google: Gemini 2.5 Pro Preview 05-06 | 1M | 66K | ✅ | ✅ | $1.25 | $10.00 |
| `gemini-3-flash-preview` | Google: Gemini 3 Flash Preview | 1M | 66K | ✅ | ✅ | $0.50 | $3.00 |
| `gemini-3.1-flash-lite` | Google: Gemini 3.1 Flash Lite | 1M | 66K | ✅ | ✅ | $0.25 | $1.50 |
| `gemini-3.1-flash-lite-preview` | Google: Gemini 3.1 Flash Lite Preview | 1M | 66K | ✅ | ✅ | $0.25 | $1.50 |
| `gemini-3.1-pro-preview` | Google: Gemini 3.1 Pro Preview | 1M | 66K | ✅ | ✅ | $2.00 | $12.00 |
| `gemini-3.1-pro-preview-customtools` | Google: Gemini 3.1 Pro Preview Custom Tools | 1M | 66K | ✅ | ✅ | $2.00 | $12.00 |
| `mimo-v2-pro` | Xiaomi: MiMo-V2-Pro | 1M | 131K | — | ✅ | $1.00 | $3.00 |
| `mimo-v2.5` | Xiaomi: MiMo-V2.5 | 1M | 131K | ✅ | ✅ | $0.40 | $2.00 |
| `mimo-v2.5-pro` | Xiaomi: MiMo-V2.5-Pro | 1M | 16K | — | ✅ | $1.00 | $3.00 |
| `gemini-flash-latest` | Google Gemini Flash Latest | 1M | 66K | ✅ | ✅ | $0.50 | $3.00 |
| `gemini-pro-latest` | Google Gemini Pro Latest | 1M | 66K | ✅ | ✅ | $2.00 | $12.00 |
| `gpt-4.1` | OpenAI: GPT-4.1 | 1M | 4K | ✅ | — | $2.00 | $8.00 |
| `gpt-4.1-mini` | OpenAI: GPT-4.1 Mini | 1M | 33K | ✅ | — | $0.40 | $1.60 |
| `gpt-4.1-nano` | OpenAI: GPT-4.1 Nano | 1M | 33K | ✅ | — | $0.10 | $0.40 |
| `nova-2-lite-v1` | Amazon: Nova 2 Lite | 1M | 66K | ✅ | ✅ | $0.30 | $2.50 |
| `nova-premier-v1` | Amazon: Nova Premier 1.0 | 1M | 32K | ✅ | — | $2.50 | $12.50 |
| `claude-opus-4.6` | Anthropic: Claude Opus 4.6 | 1M | 128K | ✅ | ✅ | $5.00 | $25.00 |
| `claude-opus-4.6-fast` | Anthropic: Claude Opus 4.6 (Fast) | 1M | 128K | ✅ | ✅ | $30.00 | $150.00 |
| `claude-opus-4.7` | Anthropic: Claude Opus 4.7 | 1M | 128K | ✅ | ✅ | $5.00 | $25.00 |
| `claude-sonnet-4` | Anthropic: Claude Sonnet 4 | 1M | 64K | ✅ | ✅ | $3.00 | $15.00 |
| `claude-sonnet-4.5` | Anthropic: Claude Sonnet 4.5 | 1M | 64K | ✅ | ✅ | $3.00 | $15.00 |
| `claude-sonnet-4.6` | Anthropic: Claude Sonnet 4.6 | 1M | 128K | ✅ | ✅ | $3.00 | $15.00 |
| `minimax-m1` | MiniMax: MiniMax M1 | 1M | 40K | — | ✅ | $0.40 | $2.20 |
| `qwen-plus` | Qwen: Qwen-Plus | 1M | 33K | — | — | $0.26 | $0.78 |
| `qwen-plus-2025-07-28` | Qwen: Qwen Plus 0728 | 1M | 33K | — | — | $0.26 | $0.78 |
| `qwen-plus-2025-07-28:thinking` | Qwen: Qwen Plus 0728 (thinking) | 1M | 33K | — | ✅ | $0.26 | $0.78 |
| `qwen3-coder-flash` | Qwen: Qwen3 Coder Flash | 1M | 66K | — | — | $0.20 | $0.97 |
| `qwen3-coder-plus` | Qwen: Qwen3 Coder Plus | 1M | 66K | — | — | $0.65 | $3.25 |
| `qwen3.5-flash-02-23` | Qwen: Qwen3.5-Flash | 1M | 66K | ✅ | ✅ | $0.07 | $0.26 |
| `qwen3.5-plus-02-15` | Qwen: Qwen3.5 Plus 2026-02-15 | 1M | 66K | ✅ | ✅ | $0.26 | $1.56 |
| `qwen3.5-plus-20260420` | Qwen: Qwen3.5 Plus 2026-04-20 | 1M | 66K | ✅ | ✅ | $0.40 | $2.40 |
| `qwen3.6-flash` | Qwen: Qwen3.6 Flash | 1M | 66K | ✅ | ✅ | $0.25 | $1.50 |
| `qwen3.6-plus` | Qwen: Qwen3.6 Plus | 1M | 66K | ✅ | ✅ | $0.33 | $1.95 |
| `grok-4.3` | xAI: Grok 4.3 | 1M | 4K | ✅ | ✅ | $1.25 | $2.50 |
| `claude-opus-latest` | Anthropic: Claude Opus Latest | 1M | 128K | ✅ | ✅ | $5.00 | $25.00 |
| `claude-sonnet-latest` | Anthropic Claude Sonnet Latest | 1M | 128K | ✅ | ✅ | $3.00 | $15.00 |
| `gpt-5` | OpenAI: GPT-5 | 400K | 128K | ✅ | ✅ | $1.25 | $10.00 |
| `gpt-5-codex` | OpenAI: GPT-5 Codex | 400K | 128K | ✅ | ✅ | $1.25 | $10.00 |
| `gpt-5-mini` | OpenAI: GPT-5 Mini | 400K | 128K | ✅ | ✅ | $0.25 | $2.00 |
| `gpt-5-nano` | OpenAI: GPT-5 Nano | 400K | 4K | ✅ | ✅ | $0.05 | $0.40 |
| `gpt-5-pro` | OpenAI: GPT-5 Pro | 400K | 128K | ✅ | ✅ | $15.00 | $120.00 |
| `gpt-5.1` | OpenAI: GPT-5.1 | 400K | 128K | ✅ | ✅ | $1.25 | $10.00 |
| `gpt-5.1-codex` | OpenAI: GPT-5.1-Codex | 400K | 128K | ✅ | ✅ | $1.25 | $10.00 |
| `gpt-5.1-codex-max` | OpenAI: GPT-5.1-Codex-Max | 400K | 128K | ✅ | ✅ | $1.25 | $10.00 |
| `gpt-5.1-codex-mini` | OpenAI: GPT-5.1-Codex-Mini | 400K | 128K | ✅ | ✅ | $0.25 | $2.00 |
| `gpt-5.2` | OpenAI: GPT-5.2 | 400K | 128K | ✅ | ✅ | $1.75 | $14.00 |
| `gpt-5.2-codex` | OpenAI: GPT-5.2-Codex | 400K | 128K | ✅ | ✅ | $1.75 | $14.00 |
| `gpt-5.2-pro` | OpenAI: GPT-5.2 Pro | 400K | 128K | ✅ | ✅ | $21.00 | $168.00 |
| `gpt-5.3-codex` | OpenAI: GPT-5.3-Codex | 400K | 128K | ✅ | ✅ | $1.75 | $14.00 |
| `gpt-5.4-mini` | OpenAI: GPT-5.4 Mini | 400K | 128K | ✅ | ✅ | $0.75 | $4.50 |
| `gpt-5.4-nano` | OpenAI: GPT-5.4 Nano | 400K | 128K | ✅ | ✅ | $0.20 | $1.25 |
| `gpt-chat-latest` | OpenAI: GPT Chat Latest | 400K | 128K | ✅ | — | $5.00 | $30.00 |
| `gpt-mini-latest` | OpenAI GPT Mini Latest | 400K | 128K | ✅ | ✅ | $0.75 | $4.50 |
| `llama-4-scout` | Meta: Llama 4 Scout | 328K | 16K | ✅ | — | $0.08 | $0.30 |
| `nova-lite-v1` | Amazon: Nova Lite 1.0 | 300K | 5K | ✅ | — | $0.06 | $0.24 |
| `nova-pro-v1` | Amazon: Nova Pro 1.0 | 300K | 5K | ✅ | — | $0.80 | $3.20 |
| `trinity-large-thinking` | Arcee AI: Trinity Large Thinking | 262K | 262K | — | ✅ | $0.22 | $0.85 |
| `trinity-large-thinking:free` | Arcee AI: Trinity Large Thinking (free) | 262K | 80K | — | ✅ | - | - |
| `seed-1.6` | ByteDance Seed: Seed 1.6 | 262K | 33K | ✅ | ✅ | $0.25 | $2.00 |
| `seed-1.6-flash` | ByteDance Seed: Seed 1.6 Flash | 262K | 33K | ✅ | ✅ | $0.07 | $0.30 |
| `seed-2.0-lite` | ByteDance Seed: Seed-2.0-Lite | 262K | 131K | ✅ | ✅ | $0.25 | $2.00 |
| `seed-2.0-mini` | ByteDance Seed: Seed-2.0-Mini | 262K | 131K | ✅ | ✅ | $0.10 | $0.40 |
| `gemma-4-26b-a4b-it` | Google: Gemma 4 26B A4B  | 262K | 4K | ✅ | ✅ | $0.06 | $0.33 |
| `gemma-4-26b-a4b-it:free` | Google: Gemma 4 26B A4B  (free) | 262K | 33K | ✅ | ✅ | - | - |
| `gemma-4-31b-it` | Google: Gemma 4 31B | 262K | 16K | ✅ | ✅ | $0.12 | $0.37 |
| `gemma-4-31b-it:free` | Google: Gemma 4 31B (free) | 262K | 33K | ✅ | ✅ | - | - |
| `ling-2.6-1t` | inclusionAI: Ling-2.6-1T | 262K | 33K | — | — | $0.30 | $2.50 |
| `ling-2.6-flash` | inclusionAI: Ling-2.6-flash | 262K | 33K | — | — | $0.08 | $0.24 |
| `ring-2.6-1t:free` | inclusionAI: Ring-2.6-1T (free) | 262K | 66K | — | ✅ | - | - |
| `devstral-2512` | Mistral: Devstral 2 2512 | 262K | 4K | — | — | $0.40 | $2.00 |
| `ministral-14b-2512` | Mistral: Ministral 3 14B 2512 | 262K | 4K | ✅ | — | $0.20 | $0.20 |
| `ministral-8b-2512` | Mistral: Ministral 3 8B 2512 | 262K | 4K | ✅ | — | $0.15 | $0.15 |
| `mistral-large-2512` | Mistral: Mistral Large 3 2512 | 262K | 4K | ✅ | — | $0.50 | $1.50 |
| `mistral-medium-3-5` | Mistral: Mistral Medium 3.5 | 262K | 4K | ✅ | ✅ | $1.50 | $7.50 |
| `mistral-small-2603` | Mistral: Mistral Small 4 | 262K | 4K | ✅ | ✅ | $0.15 | $0.60 |
| `kimi-k2-0905` | MoonshotAI: Kimi K2 0905 | 262K | 262K | — | — | $0.40 | $2.00 |
| `kimi-k2-thinking` | MoonshotAI: Kimi K2 Thinking | 262K | 262K | — | ✅ | $0.60 | $2.50 |
| `kimi-k2.5` | MoonshotAI: Kimi K2.5 | 262K | 4K | ✅ | ✅ | $0.41 | $2.06 |
| `kimi-k2.6` | MoonshotAI: Kimi K2.6 | 262K | 66K | ✅ | ✅ | $0.75 | $3.50 |
| `nemotron-3-nano-30b-a3b` | NVIDIA: Nemotron 3 Nano 30B A3B | 262K | 228K | — | ✅ | $0.05 | $0.20 |
| `nemotron-3-super-120b-a12b` | NVIDIA: Nemotron 3 Super | 262K | 4K | — | ✅ | $0.09 | $0.45 |
| `nemotron-3-super-120b-a12b:free` | NVIDIA: Nemotron 3 Super (free) | 262K | 262K | — | ✅ | - | - |
| `qwen3-235b-a22b-2507` | Qwen: Qwen3 235B A22B Instruct 2507 | 262K | 16K | — | — | $0.07 | $0.10 |
| `qwen3-30b-a3b-instruct-2507` | Qwen: Qwen3 30B A3B Instruct 2507 | 262K | 262K | — | — | $0.09 | $0.30 |
| `qwen3-coder` | Qwen: Qwen3 Coder 480B A35B | 262K | 66K | — | — | $0.22 | $1.80 |
| `qwen3-coder-next` | Qwen: Qwen3 Coder Next | 262K | 262K | — | — | $0.11 | $0.80 |
| `qwen3-max` | Qwen: Qwen3 Max | 262K | 33K | — | — | $0.78 | $3.90 |
| `qwen3-max-thinking` | Qwen: Qwen3 Max Thinking | 262K | 33K | — | ✅ | $0.78 | $3.90 |
| `qwen3-next-80b-a3b-instruct` | Qwen: Qwen3 Next 80B A3B Instruct | 262K | 16K | — | — | $0.09 | $1.10 |
| `qwen3-next-80b-a3b-instruct:free` | Qwen: Qwen3 Next 80B A3B Instruct (free) | 262K | 4K | — | — | - | - |
| `qwen3-vl-235b-a22b-instruct` | Qwen: Qwen3 VL 235B A22B Instruct | 262K | 16K | ✅ | — | $0.20 | $0.88 |
| `qwen3.5-122b-a10b` | Qwen: Qwen3.5-122B-A10B | 262K | 66K | ✅ | ✅ | $0.26 | $2.08 |
| `qwen3.5-27b` | Qwen: Qwen3.5-27B | 262K | 66K | ✅ | ✅ | $0.20 | $1.56 |
| `qwen3.5-35b-a3b` | Qwen: Qwen3.5-35B-A3B | 262K | 82K | ✅ | ✅ | $0.14 | $1.00 |
| `qwen3.5-397b-a17b` | Qwen: Qwen3.5 397B A17B | 262K | 66K | ✅ | ✅ | $0.39 | $2.34 |
| `qwen3.5-9b` | Qwen: Qwen3.5-9B | 262K | 82K | ✅ | ✅ | $0.04 | $0.15 |
| `qwen3.6-27b` | Qwen: Qwen3.6 27B | 262K | 82K | ✅ | ✅ | $0.32 | $3.20 |
| `qwen3.6-35b-a3b` | Qwen: Qwen3.6 35B A3B | 262K | 262K | ✅ | ✅ | $0.15 | $1.00 |
| `qwen3.6-max-preview` | Qwen: Qwen3.6 Max Preview | 262K | 66K | — | ✅ | $1.04 | $6.24 |
| `step-3.5-flash` | StepFun: Step 3.5 Flash | 262K | 66K | — | ✅ | $0.10 | $0.30 |
| `hy3-preview` | Tencent: Hy3 preview | 262K | 262K | — | ✅ | $0.07 | $0.26 |
| `mimo-v2-flash` | Xiaomi: MiMo-V2-Flash | 262K | 66K | — | ✅ | $0.10 | $0.30 |
| `mimo-v2-omni` | Xiaomi: MiMo-V2-Omni | 262K | 66K | ✅ | ✅ | $0.40 | $2.00 |
| `kimi-latest` | MoonshotAI Kimi Latest | 262K | 66K | ✅ | ✅ | $0.75 | $3.50 |
| `qwen3-coder:free` | Qwen: Qwen3 Coder 480B A35B (free) | 262K | 262K | — | — | - | - |
| `jamba-large-1.7` | AI21: Jamba Large 1.7 | 256K | 4K | — | — | $2.00 | $8.00 |
| `kat-coder-pro-v2` | Kwaipilot: KAT-Coder-Pro V2 | 256K | 80K | — | — | $0.30 | $1.20 |
| `codestral-2508` | Mistral: Codestral 2508 | 256K | 4K | — | — | $0.30 | $0.90 |
| `nemotron-3-nano-30b-a3b:free` | NVIDIA: Nemotron 3 Nano 30B A3B (free) | 256K | 4K | — | ✅ | - | - |
| `nemotron-3-nano-omni-30b-a3b-reasoning:free` | NVIDIA: Nemotron 3 Nano Omni (free) | 256K | 66K | ✅ | — | - | - |
| `relace-search` | Relace: Relace Search | 256K | 128K | — | — | $1.00 | $3.00 |
| `grok-4` | xAI: Grok 4 | 256K | 4K | ✅ | ✅ | $3.00 | $15.00 |
| `grok-code-fast-1` | xAI: Grok Code Fast 1 | 256K | 10K | — | ✅ | $0.20 | $1.50 |
| `glm-4.6` | Z.ai: GLM 4.6 | 205K | 205K | — | ✅ | $0.39 | $1.90 |
| `glm-4.7` | Z.ai: GLM 4.7 | 203K | 131K | — | ✅ | $0.40 | $1.75 |
| `glm-4.7-flash` | Z.ai: GLM 4.7 Flash | 203K | 16K | — | ✅ | $0.06 | $0.40 |
| `glm-5` | Z.ai: GLM 5 | 203K | 4K | — | ✅ | $0.60 | $1.90 |
| `glm-5-turbo` | Z.ai: GLM 5 Turbo | 203K | 131K | — | ✅ | $1.20 | $4.00 |
| `glm-5.1` | Z.ai: GLM 5.1 | 203K | 4K | — | ✅ | $0.98 | $3.08 |
| `glm-5v-turbo` | Z.ai: GLM 5V Turbo | 203K | 131K | ✅ | ✅ | $1.20 | $4.00 |
| `claude-3-haiku` | Anthropic: Claude 3 Haiku | 200K | 4K | ✅ | — | $0.25 | $1.25 |
| `claude-3.5-haiku` | Anthropic: Claude 3.5 Haiku | 200K | 8K | ✅ | — | $0.80 | $4.00 |
| `claude-haiku-4.5` | Anthropic: Claude Haiku 4.5 | 200K | 64K | ✅ | ✅ | $1.00 | $5.00 |
| `claude-opus-4` | Anthropic: Claude Opus 4 | 200K | 32K | ✅ | ✅ | $15.00 | $75.00 |
| `claude-opus-4.1` | Anthropic: Claude Opus 4.1 | 200K | 32K | ✅ | ✅ | $15.00 | $75.00 |
| `claude-opus-4.5` | Anthropic: Claude Opus 4.5 | 200K | 64K | ✅ | ✅ | $5.00 | $25.00 |
| `o1` | OpenAI: o1 | 200K | 100K | ✅ | ✅ | $15.00 | $60.00 |
| `o3` | OpenAI: o3 | 200K | 100K | ✅ | ✅ | $2.00 | $8.00 |
| `o3-deep-research` | OpenAI: o3 Deep Research | 200K | 100K | ✅ | ✅ | $10.00 | $40.00 |
| `o3-mini` | OpenAI: o3 Mini | 200K | 100K | — | ✅ | $1.10 | $4.40 |
| `o3-mini-high` | OpenAI: o3 Mini High | 200K | 100K | — | ✅ | $1.10 | $4.40 |
| `o3-pro` | OpenAI: o3 Pro | 200K | 100K | ✅ | ✅ | $20.00 | $80.00 |
| `o4-mini` | OpenAI: o4 Mini | 200K | 100K | ✅ | ✅ | $1.10 | $4.40 |
| `o4-mini-deep-research` | OpenAI: o4 Mini Deep Research | 200K | 100K | ✅ | ✅ | $2.00 | $8.00 |
| `o4-mini-high` | OpenAI: o4 Mini High | 200K | 100K | ✅ | ✅ | $1.10 | $4.40 |
| `free` | Free Models Router | 200K | 4K | ✅ | ✅ | - | - |
| `claude-haiku-latest` | Anthropic Claude Haiku Latest | 200K | 64K | ✅ | ✅ | $1.00 | $5.00 |
| `minimax-m2` | MiniMax: MiniMax M2 | 197K | 197K | — | ✅ | $0.26 | $1.00 |
| `minimax-m2.1` | MiniMax: MiniMax M2.1 | 197K | 197K | — | ✅ | $0.29 | $0.95 |
| `minimax-m2.5` | MiniMax: MiniMax M2.5 | 197K | 197K | — | ✅ | $0.15 | $1.15 |
| `minimax-m2.5:free` | MiniMax: MiniMax M2.5 (free) | 197K | 8K | — | ✅ | - | - |
| `minimax-m2.7` | MiniMax: MiniMax M2.7 | 197K | 4K | — | ✅ | $0.28 | $1.20 |
| `deepseek-chat` | DeepSeek: DeepSeek V3 | 164K | 16K | — | — | $0.32 | $0.89 |
| `deepseek-chat-v3-0324` | DeepSeek: DeepSeek V3 0324 | 164K | 16K | — | — | $0.20 | $0.77 |
| `deepseek-chat-v3.1` | DeepSeek: DeepSeek V3.1 | 164K | 33K | — | ✅ | $0.21 | $0.79 |
| `deepseek-r1-0528` | DeepSeek: R1 0528 | 164K | 33K | — | ✅ | $0.50 | $2.15 |
| `deepseek-v3.1-terminus` | DeepSeek: DeepSeek V3.1 Terminus | 164K | 33K | — | ✅ | $0.27 | $0.95 |
| `deepseek-v3.2-exp` | DeepSeek: DeepSeek V3.2 Exp | 164K | 66K | — | ✅ | $0.27 | $0.41 |
| `qwen3-coder-30b-a3b-instruct` | Qwen: Qwen3 Coder 30B A3B Instruct | 160K | 33K | — | — | $0.07 | $0.27 |
| `tongyi-deepresearch-30b-a3b` | Tongyi DeepResearch 30B A3B | 131K | 131K | — | ✅ | $0.09 | $0.45 |
| `trinity-mini` | Arcee AI: Trinity Mini | 131K | 131K | — | ✅ | $0.04 | $0.15 |
| `virtuoso-large` | Arcee AI: Virtuoso Large | 131K | 64K | — | — | $0.75 | $1.20 |
| `cobuddy:free` | Baidu Qianfan: CoBuddy (free) | 131K | 66K | — | ✅ | - | - |
| `deepseek-v3.2` | DeepSeek: DeepSeek V3.2 | 131K | 66K | — | ✅ | $0.25 | $0.38 |
| `gemma-3-12b-it` | Google: Gemma 3 12B | 131K | 16K | ✅ | — | $0.04 | $0.13 |
| `gemma-3-27b-it` | Google: Gemma 3 27B | 131K | 16K | ✅ | — | $0.08 | $0.16 |
| `granite-4.1-8b` | IBM: Granite 4.1 8B | 131K | 131K | — | — | $0.05 | $0.10 |
| `llama-3.1-70b-instruct` | Meta: Llama 3.1 70B Instruct | 131K | 16K | — | — | $0.40 | $0.40 |
| `llama-3.3-70b-instruct` | Meta: Llama 3.3 70B Instruct | 131K | 16K | — | — | $0.10 | $0.32 |
| `devstral-medium` | Mistral: Devstral Medium | 131K | 4K | — | — | $0.40 | $2.00 |
| `devstral-small` | Mistral: Devstral Small 1.1 | 131K | 4K | — | — | $0.10 | $0.30 |
| `ministral-3b-2512` | Mistral: Ministral 3 3B 2512 | 131K | 4K | ✅ | — | $0.10 | $0.10 |
| `mistral-large-2407` | Mistral Large 2407 | 131K | 4K | — | — | $2.00 | $6.00 |
| `mistral-large-2411` | Mistral Large 2411 | 131K | 4K | — | — | $2.00 | $6.00 |
| `mistral-medium-3` | Mistral: Mistral Medium 3 | 131K | 4K | ✅ | — | $0.40 | $2.00 |
| `mistral-medium-3.1` | Mistral: Mistral Medium 3.1 | 131K | 4K | ✅ | — | $0.40 | $2.00 |
| `mistral-nemo` | Mistral: Mistral Nemo | 131K | 4K | — | — | $0.02 | $0.03 |
| `pixtral-large-2411` | Mistral: Pixtral Large 2411 | 131K | 4K | ✅ | — | $2.00 | $6.00 |
| `kimi-k2` | MoonshotAI: Kimi K2 0711 | 131K | 33K | — | — | $0.57 | $2.30 |
| `deepseek-v3.1-nex-n1` | Nex AGI: DeepSeek V3.1 Nex N1 | 131K | 164K | — | — | $0.14 | $0.50 |
| `llama-3.3-nemotron-super-49b-v1.5` | NVIDIA: Llama 3.3 Nemotron Super 49B V1.5 | 131K | 16K | — | ✅ | $0.10 | $0.40 |
| `nemotron-nano-9b-v2` | NVIDIA: Nemotron Nano 9B V2 | 131K | 16K | — | ✅ | $0.04 | $0.16 |
| `gpt-oss-120b` | OpenAI: gpt-oss-120b | 131K | 4K | — | ✅ | $0.04 | $0.18 |
| `gpt-oss-120b:free` | OpenAI: gpt-oss-120b (free) | 131K | 131K | — | ✅ | - | - |
| `gpt-oss-20b` | OpenAI: gpt-oss-20b | 131K | 131K | — | ✅ | $0.03 | $0.14 |
| `gpt-oss-20b:free` | OpenAI: gpt-oss-20b (free) | 131K | 8K | — | ✅ | - | - |
| `gpt-oss-safeguard-20b` | OpenAI: gpt-oss-safeguard-20b | 131K | 66K | — | ✅ | $0.07 | $0.30 |
| `laguna-m.1:free` | Poolside: Laguna M.1 (free) | 131K | 8K | — | ✅ | - | - |
| `laguna-xs.2:free` | Poolside: Laguna XS.2 (free) | 131K | 8K | — | ✅ | - | - |
| `intellect-3` | Prime Intellect: INTELLECT-3 | 131K | 131K | — | ✅ | $0.20 | $1.10 |
| `qwen-turbo` | Qwen: Qwen-Turbo | 131K | 8K | — | — | $0.03 | $0.13 |
| `qwen-vl-max` | Qwen: Qwen VL Max | 131K | 33K | ✅ | — | $0.52 | $2.08 |
| `qwen3-235b-a22b` | Qwen: Qwen3 235B A22B | 131K | 8K | — | ✅ | $0.45 | $1.82 |
| `qwen3-235b-a22b-thinking-2507` | Qwen: Qwen3 235B A22B Thinking 2507 | 131K | 4K | — | ✅ | $0.15 | $1.50 |
| `qwen3-30b-a3b-thinking-2507` | Qwen: Qwen3 30B A3B Thinking 2507 | 131K | 131K | — | ✅ | $0.08 | $0.40 |
| `qwen3-next-80b-a3b-thinking` | Qwen: Qwen3 Next 80B A3B Thinking | 131K | 33K | — | ✅ | $0.10 | $0.78 |
| `qwen3-vl-235b-a22b-thinking` | Qwen: Qwen3 VL 235B A22B Thinking | 131K | 33K | ✅ | ✅ | $0.26 | $2.60 |
| `qwen3-vl-30b-a3b-instruct` | Qwen: Qwen3 VL 30B A3B Instruct | 131K | 33K | ✅ | — | $0.13 | $0.52 |
| `qwen3-vl-30b-a3b-thinking` | Qwen: Qwen3 VL 30B A3B Thinking | 131K | 33K | ✅ | ✅ | $0.13 | $1.56 |
| `qwen3-vl-32b-instruct` | Qwen: Qwen3 VL 32B Instruct | 131K | 33K | ✅ | — | $0.10 | $0.42 |
| `qwen3-vl-8b-instruct` | Qwen: Qwen3 VL 8B Instruct | 131K | 33K | ✅ | — | $0.08 | $0.50 |
| `qwen3-vl-8b-thinking` | Qwen: Qwen3 VL 8B Thinking | 131K | 33K | ✅ | ✅ | $0.12 | $1.36 |
| `l3.1-euryale-70b` | Sao10K: Llama 3.1 Euryale 70B v2.2 | 131K | 16K | — | — | $0.85 | $0.85 |
| `grok-3` | xAI: Grok 3 | 131K | 4K | — | — | $3.00 | $15.00 |
| `grok-3-beta` | xAI: Grok 3 Beta | 131K | 4K | — | — | $3.00 | $15.00 |
| `grok-3-mini` | xAI: Grok 3 Mini | 131K | 4K | — | ✅ | $0.30 | $0.50 |
| `grok-3-mini-beta` | xAI: Grok 3 Mini Beta | 131K | 4K | — | ✅ | $0.30 | $0.50 |
| `glm-4.5` | Z.ai: GLM 4.5 | 131K | 98K | — | ✅ | $0.60 | $2.20 |
| `glm-4.5-air` | Z.ai: GLM 4.5 Air | 131K | 98K | — | ✅ | $0.13 | $0.85 |
| `glm-4.5-air:free` | Z.ai: GLM 4.5 Air (free) | 131K | 96K | — | ✅ | - | - |
| `glm-4.6v` | Z.ai: GLM 4.6V | 131K | 24K | ✅ | ✅ | $0.30 | $0.90 |
| `trinity-large-preview` | Arcee AI: Trinity Large Preview | 131K | 4K | — | — | $0.15 | $0.45 |
| `nova-micro-v1` | Amazon: Nova Micro 1.0 | 128K | 5K | — | — | $0.04 | $0.14 |
| `command-r-08-2024` | Cohere: Command R (08-2024) | 128K | 4K | — | — | $0.15 | $0.60 |
| `command-r-plus-08-2024` | Cohere: Command R+ (08-2024) | 128K | 4K | — | — | $2.50 | $10.00 |
| `mercury-2` | Inception: Mercury 2 | 128K | 50K | — | ✅ | $0.25 | $0.75 |
| `mistral-large` | Mistral Large | 128K | 4K | — | — | $2.00 | $6.00 |
| `mistral-small-3.2-24b-instruct` | Mistral: Mistral Small 3.2 24B | 128K | 16K | ✅ | — | $0.07 | $0.20 |
| `nemotron-nano-12b-v2-vl:free` | NVIDIA: Nemotron Nano 12B 2 VL (free) | 128K | 128K | ✅ | ✅ | - | - |
| `nemotron-nano-9b-v2:free` | NVIDIA: Nemotron Nano 9B V2 (free) | 128K | 4K | — | ✅ | - | - |
| `gpt-4-1106-preview` | OpenAI: GPT-4 Turbo (older v1106) | 128K | 4K | — | — | $10.00 | $30.00 |
| `gpt-4-turbo` | OpenAI: GPT-4 Turbo | 128K | 4K | ✅ | — | $10.00 | $30.00 |
| `gpt-4-turbo-preview` | OpenAI: GPT-4 Turbo Preview | 128K | 4K | — | — | $10.00 | $30.00 |
| `gpt-4o` | OpenAI: GPT-4o | 128K | 16K | ✅ | — | $2.50 | $10.00 |
| `gpt-4o-2024-05-13` | OpenAI: GPT-4o (2024-05-13) | 128K | 4K | ✅ | — | $5.00 | $15.00 |
| `gpt-4o-2024-08-06` | OpenAI: GPT-4o (2024-08-06) | 128K | 16K | ✅ | — | $2.50 | $10.00 |
| `gpt-4o-2024-11-20` | OpenAI: GPT-4o (2024-11-20) | 128K | 16K | ✅ | — | $2.50 | $10.00 |
| `gpt-4o-audio-preview` | OpenAI: GPT-4o Audio | 128K | 16K | — | — | $2.50 | $10.00 |
| `gpt-4o-mini` | OpenAI: GPT-4o-mini | 128K | 16K | ✅ | — | $0.15 | $0.60 |
| `gpt-4o-mini-2024-07-18` | OpenAI: GPT-4o-mini (2024-07-18) | 128K | 16K | ✅ | — | $0.15 | $0.60 |
| `gpt-5.1-chat` | OpenAI: GPT-5.1 Chat | 128K | 16K | ✅ | — | $1.25 | $10.00 |
| `gpt-5.2-chat` | OpenAI: GPT-5.2 Chat | 128K | 32K | ✅ | — | $1.75 | $14.00 |
| `gpt-5.3-chat` | OpenAI: GPT-5.3 Chat | 128K | 16K | ✅ | — | $1.75 | $14.00 |
| `gpt-audio` | OpenAI: GPT Audio | 128K | 16K | — | — | $2.50 | $10.00 |
| `gpt-audio-mini` | OpenAI: GPT Audio Mini | 128K | 16K | — | — | $0.60 | $2.40 |
| `solar-pro-3` | Upstage: Solar Pro 3 | 128K | 4K | — | ✅ | $0.15 | $0.60 |
| `glm-4-32b` | Z.ai: GLM 4 32B  | 128K | 4K | — | — | $0.10 | $0.10 |
| `ernie-4.5-21b-a3b` | Baidu: ERNIE 4.5 21B A3B | 120K | 8K | — | — | $0.07 | $0.28 |
| `llama-3.3-70b-instruct:free` | Meta: Llama 3.3 70B Instruct (free) | 66K | 4K | — | — | - | - |
| `mixtral-8x22b-instruct` | Mistral: Mixtral 8x22B Instruct | 66K | 4K | — | — | $2.00 | $6.00 |
| `glm-4.5v` | Z.ai: GLM 4.5V | 66K | 16K | ✅ | ✅ | $0.60 | $1.80 |
| `deepseek-r1` | DeepSeek: R1 | 64K | 16K | — | ✅ | $0.70 | $2.50 |
| `qwen3-14b` | Qwen: Qwen3 14B | 41K | 41K | — | ✅ | $0.06 | $0.24 |
| `qwen3-30b-a3b` | Qwen: Qwen3 30B A3B | 41K | 20K | — | ✅ | $0.09 | $0.45 |
| `qwen3-32b` | Qwen: Qwen3 32B | 41K | 16K | — | ✅ | $0.08 | $0.28 |
| `qwen3-8b` | Qwen: Qwen3 8B | 41K | 8K | — | ✅ | $0.05 | $0.40 |
| `rnj-1-instruct` | EssentialAI: Rnj 1 Instruct | 33K | 4K | — | — | $0.15 | $0.15 |
| `mistral-saba` | Mistral: Saba | 33K | 4K | — | — | $0.20 | $0.60 |
| `qwen-2.5-72b-instruct` | Qwen2.5 72B Instruct | 33K | 16K | — | — | $0.36 | $0.40 |
| `qwen-2.5-7b-instruct` | Qwen: Qwen2.5 7B Instruct | 33K | 33K | — | — | $0.04 | $0.10 |
| `qwen-max` | Qwen: Qwen-Max  | 33K | 8K | — | — | $1.04 | $4.16 |
| `rocinante-12b` | TheDrummer: Rocinante 12B | 33K | 33K | — | — | $0.17 | $0.43 |
| `unslopnemo-12b` | TheDrummer: UnslopNemo 12B | 33K | 33K | — | — | $0.40 | $0.40 |
| `voxtral-small-24b-2507` | Mistral: Voxtral Small 24B 2507 | 32K | 4K | — | — | $0.10 | $0.30 |
| `ernie-4.5-vl-28b-a3b` | Baidu: ERNIE 4.5 VL 28B A3B | 30K | 8K | ✅ | ✅ | $0.14 | $0.56 |
| `gpt-3.5-turbo` | OpenAI: GPT-3.5 Turbo | 16K | 4K | — | — | $0.50 | $1.50 |
| `gpt-3.5-turbo-16k` | OpenAI: GPT-3.5 Turbo 16k | 16K | 4K | — | — | $3.00 | $4.00 |
| `llama-3.1-8b-instruct` | Meta: Llama 3.1 8B Instruct | 16K | 16K | — | — | $0.02 | $0.05 |
| `reka-edge` | Reka Edge | 16K | 16K | ✅ | — | $0.10 | $0.10 |
| `l3-euryale-70b` | Sao10k: Llama 3 Euryale 70B v2.1 | 8K | 8K | — | — | $1.48 | $1.48 |
| `gpt-4` | OpenAI: GPT-4 | 8K | 4K | — | — | $30.00 | $60.00 |
| `gpt-4-0314` | OpenAI: GPT-4 (older v0314) | 8K | 4K | — | — | $30.00 | $60.00 |
| `gpt-3.5-turbo-0613` | OpenAI: GPT-3.5 Turbo (older v0613) | 4K | 4K | — | — | $1.00 | $2.00 |

### vercel-ai-gateway

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 | 输入 ($/1M) | 输出 ($/1M) |
|---|---|---|---|---|---|---|---|
| `grok-4-fast-non-reasoning` | Grok 4 Fast Non-Reasoning | 2M | 256K | ✅ | — | $0.20 | $0.50 |
| `grok-4-fast-reasoning` | Grok 4 Fast Reasoning | 2M | 256K | ✅ | ✅ | $0.20 | $0.50 |
| `grok-4.1-fast-non-reasoning` | Grok 4.1 Fast Non-Reasoning | 2M | 30K | ✅ | — | $0.20 | $0.50 |
| `grok-4.1-fast-reasoning` | Grok 4.1 Fast Reasoning | 2M | 30K | ✅ | ✅ | $0.20 | $0.50 |
| `grok-4.20-multi-agent` | Grok 4.20 Multi-Agent | 2M | 2M | ✅ | ✅ | $1.25 | $2.50 |
| `grok-4.20-multi-agent-beta` | Grok 4.20 Multi Agent Beta | 2M | 2M | ✅ | ✅ | $1.25 | $2.50 |
| `grok-4.20-non-reasoning` | Grok 4.20 Non-Reasoning | 2M | 2M | ✅ | — | $1.25 | $2.50 |
| `grok-4.20-non-reasoning-beta` | Grok 4.20 Beta Non-Reasoning | 2M | 2M | ✅ | — | $1.25 | $2.50 |
| `grok-4.20-reasoning` | Grok 4.20 Reasoning | 2M | 2M | ✅ | ✅ | $1.25 | $2.50 |
| `grok-4.20-reasoning-beta` | Grok 4.20 Beta Reasoning | 2M | 2M | ✅ | ✅ | $1.25 | $2.50 |
| `gpt-5.4` | GPT 5.4 | 1M | 128K | ✅ | ✅ | $2.50 | $15.00 |
| `gpt-5.4-pro` | GPT 5.4 Pro | 1M | 128K | ✅ | ✅ | $30.00 | $180.00 |
| `mimo-v2.5` | MiMo M2.5 | 1M | 131K | ✅ | ✅ | $0.40 | $2.00 |
| `mimo-v2.5-pro` | MiMo V2.5 Pro | 1M | 131K | ✅ | ✅ | $1.00 | $3.00 |
| `gemini-2.0-flash` | Gemini 2.0 Flash | 1M | 8K | ✅ | — | $0.15 | $0.60 |
| `gemini-2.0-flash-lite` | Gemini 2.0 Flash Lite | 1M | 8K | ✅ | — | $0.07 | $0.30 |
| `gemini-2.5-flash-lite` | Gemini 2.5 Flash Lite | 1M | 66K | ✅ | ✅ | $0.10 | $0.40 |
| `gemini-2.5-pro` | Gemini 2.5 Pro | 1M | 66K | ✅ | ✅ | $1.25 | $10.00 |
| `gpt-4.1` | GPT-4.1 | 1M | 33K | ✅ | — | $2.00 | $8.00 |
| `gpt-4.1-mini` | GPT-4.1 mini | 1M | 33K | ✅ | — | $0.40 | $1.60 |
| `gpt-4.1-nano` | GPT-4.1 nano | 1M | 33K | ✅ | — | $0.10 | $0.40 |
| `qwen3-coder-plus` | Qwen3 Coder Plus | 1M | 66K | — | — | $1.00 | $5.00 |
| `qwen3.5-flash` | Qwen 3.5 Flash | 1M | 64K | ✅ | ✅ | $0.10 | $0.40 |
| `qwen3.5-plus` | Qwen 3.5 Plus | 1M | 64K | ✅ | ✅ | $0.40 | $2.40 |
| `qwen3.6-plus` | Qwen 3.6 Plus | 1M | 64K | ✅ | ✅ | $0.50 | $3.00 |
| `claude-opus-4.6` | Claude Opus 4.6 | 1M | 128K | ✅ | ✅ | $5.00 | $25.00 |
| `claude-opus-4.7` | Claude Opus 4.7 | 1M | 128K | ✅ | ✅ | $5.00 | $25.00 |
| `claude-sonnet-4` | Claude Sonnet 4 | 1M | 64K | ✅ | ✅ | $3.00 | $15.00 |
| `claude-sonnet-4.5` | Claude Sonnet 4.5 | 1M | 64K | ✅ | ✅ | $3.00 | $15.00 |
| `claude-sonnet-4.6` | Claude Sonnet 4.6 | 1M | 128K | ✅ | ✅ | $3.00 | $15.00 |
| `deepseek-v4-flash` | DeepSeek V4 Flash | 1M | 384K | — | ✅ | $0.14 | $0.28 |
| `deepseek-v4-pro` | DeepSeek V4 Pro | 1M | 384K | — | ✅ | $0.43 | $0.87 |
| `gemini-2.5-flash` | Gemini 2.5 Flash | 1M | 66K | ✅ | ✅ | $0.30 | $2.50 |
| `gemini-3-flash` | Gemini 3 Flash | 1M | 65K | ✅ | ✅ | $0.50 | $3.00 |
| `gemini-3-pro-preview` | Gemini 3 Pro Preview | 1M | 64K | ✅ | ✅ | $2.00 | $12.00 |
| `gemini-3.1-flash-lite` | Gemini 3.1 Flash Lite | 1M | 65K | ✅ | ✅ | $0.25 | $1.50 |
| `gemini-3.1-flash-lite-preview` | Gemini 3.1 Flash Lite Preview | 1M | 65K | ✅ | ✅ | $0.25 | $1.50 |
| `gemini-3.1-pro-preview` | Gemini 3.1 Pro Preview | 1M | 64K | ✅ | ✅ | $2.00 | $12.00 |
| `gpt-5.5` | GPT 5.5 | 1M | 128K | ✅ | ✅ | $5.00 | $30.00 |
| `gpt-5.5-pro` | GPT 5.5 Pro | 1M | 128K | ✅ | ✅ | $30.00 | $180.00 |
| `grok-4.3` | Grok 4.3 | 1M | 1M | ✅ | ✅ | $1.25 | $2.50 |
| `mimo-v2-pro` | MiMo V2 Pro | 1M | 128K | — | ✅ | $1.00 | $3.00 |
| `gpt-5` | GPT-5 | 400K | 128K | ✅ | ✅ | $1.25 | $10.00 |
| `gpt-5-codex` | GPT-5-Codex | 400K | 128K | — | ✅ | $1.25 | $10.00 |
| `gpt-5-mini` | GPT-5 mini | 400K | 128K | ✅ | ✅ | $0.25 | $2.00 |
| `gpt-5-nano` | GPT-5 nano | 400K | 128K | ✅ | ✅ | $0.05 | $0.40 |
| `gpt-5-pro` | GPT-5 pro | 400K | 272K | ✅ | ✅ | $15.00 | $120.00 |
| `gpt-5.1-codex` | GPT-5.1-Codex | 400K | 128K | ✅ | ✅ | $1.25 | $10.00 |
| `gpt-5.1-codex-max` | GPT 5.1 Codex Max | 400K | 128K | ✅ | ✅ | $1.25 | $10.00 |
| `gpt-5.1-codex-mini` | GPT 5.1 Codex Mini | 400K | 128K | ✅ | ✅ | $0.25 | $2.00 |
| `gpt-5.1-thinking` | GPT 5.1 Thinking | 400K | 128K | ✅ | ✅ | $1.25 | $10.00 |
| `gpt-5.2` | GPT 5.2 | 400K | 128K | ✅ | ✅ | $1.75 | $14.00 |
| `gpt-5.2-codex` | GPT 5.2 Codex | 400K | 128K | ✅ | ✅ | $1.75 | $14.00 |
| `gpt-5.2-pro` | GPT 5.2  | 400K | 128K | ✅ | ✅ | $21.00 | $168.00 |
| `gpt-5.3-codex` | GPT 5.3 Codex | 400K | 128K | ✅ | ✅ | $1.75 | $14.00 |
| `gpt-5.4-mini` | GPT 5.4 Mini | 400K | 128K | ✅ | ✅ | $0.75 | $4.50 |
| `gpt-5.4-nano` | GPT 5.4 Nano | 400K | 128K | ✅ | ✅ | $0.20 | $1.25 |
| `qwen3-coder` | Qwen3 Coder 480B A35B Instruct | 262K | 66K | — | — | $1.50 | $7.50 |
| `qwen3-coder-30b-a3b` | Qwen 3 Coder 30B A3B Instruct | 262K | 8K | — | ✅ | $0.15 | $0.60 |
| `qwen3-max` | Qwen3 Max | 262K | 33K | — | — | $1.20 | $6.00 |
| `qwen3-max-preview` | Qwen3 Max Preview | 262K | 33K | — | — | $1.20 | $6.00 |
| `gemma-4-26b-a4b-it` | Gemma 4 26B A4B IT | 262K | 131K | ✅ | — | $0.13 | $0.40 |
| `gemma-4-31b-it` | Gemma 4 31B IT | 262K | 131K | ✅ | — | $0.14 | $0.40 |
| `mimo-v2-flash` | MiMo V2 Flash | 262K | 32K | — | ✅ | $0.10 | $0.30 |
| `kimi-k2-thinking` | Kimi K2 Thinking | 262K | 262K | — | ✅ | $0.60 | $2.50 |
| `kimi-k2-thinking-turbo` | Kimi K2 Thinking Turbo | 262K | 262K | — | ✅ | $1.15 | $8.00 |
| `kimi-k2.5` | Kimi K2.5 | 262K | 262K | ✅ | ✅ | $0.60 | $3.00 |
| `trinity-large-thinking` | Trinity Large Thinking | 262K | 80K | — | ✅ | $0.25 | $0.90 |
| `kimi-k2.6` | Kimi K2.6 | 262K | 262K | ✅ | ✅ | $0.95 | $4.00 |
| `qwen3-coder-next` | Qwen3 Coder Next | 256K | 256K | — | — | $0.50 | $1.20 |
| `qwen3-max-thinking` | Qwen 3 Max Thinking | 256K | 66K | — | ✅ | $1.20 | $6.00 |
| `qwen3.6-27b` | Qwen 3.6 27B | 256K | 256K | ✅ | ✅ | $0.60 | $3.60 |
| `seed-1.6` | Seed 1.6 | 256K | 32K | — | ✅ | $0.25 | $2.00 |
| `command-a` | Command A | 256K | 8K | — | — | $2.50 | $10.00 |
| `kat-coder-pro-v2` | Kat Coder Pro V2 | 256K | 256K | — | ✅ | $0.30 | $1.20 |
| `devstral-2` | Devstral 2 | 256K | 256K | — | — | $0.40 | $2.00 |
| `devstral-small-2` | Devstral Small 2 | 256K | 256K | — | — | $0.10 | $0.30 |
| `kimi-k2-turbo` | Kimi K2 Turbo | 256K | 16K | — | — | $1.15 | $8.00 |
| `grok-4` | Grok 4 | 256K | 256K | ✅ | ✅ | $3.00 | $15.00 |
| `grok-code-fast-1` | Grok Code Fast 1 | 256K | 256K | — | ✅ | $0.20 | $1.50 |
| `qwen-3.6-max-preview` | Qwen 3.6 Max Preview | 240K | 64K | ✅ | ✅ | $1.30 | $7.80 |
| `minimax-m2` | MiniMax M2 | 205K | 205K | — | ✅ | $0.30 | $1.20 |
| `minimax-m2.1` | MiniMax M2.1 | 205K | 131K | — | ✅ | $0.30 | $1.20 |
| `minimax-m2.1-lightning` | MiniMax M2.1 Lightning | 205K | 131K | — | ✅ | $0.30 | $2.40 |
| `minimax-m2.5` | MiniMax M2.5 | 205K | 131K | — | ✅ | $0.30 | $1.20 |
| `minimax-m2.5-highspeed` | MiniMax M2.5 High Speed | 205K | 131K | — | ✅ | $0.60 | $2.40 |
| `minimax-m2.7` | Minimax M2.7 | 205K | 131K | ✅ | ✅ | $0.30 | $1.20 |
| `minimax-m2.7-highspeed` | MiniMax M2.7 High Speed | 205K | 131K | ✅ | ✅ | $0.60 | $2.40 |
| `glm-5` | GLM 5 | 203K | 131K | — | ✅ | $1.00 | $3.20 |
| `glm-5-turbo` | GLM 5 Turbo | 203K | 131K | — | ✅ | $1.20 | $4.00 |
| `glm-5.1` | GLM 5.1 | 203K | 64K | — | ✅ | $1.40 | $4.40 |
| `claude-3-haiku` | Claude 3 Haiku | 200K | 4K | ✅ | — | $0.25 | $1.25 |
| `claude-3.5-haiku` | Claude 3.5 Haiku | 200K | 8K | ✅ | — | $0.80 | $4.00 |
| `claude-haiku-4.5` | Claude Haiku 4.5 | 200K | 64K | ✅ | ✅ | $1.00 | $5.00 |
| `claude-opus-4` | Claude Opus 4 | 200K | 32K | ✅ | ✅ | $15.00 | $75.00 |
| `claude-opus-4.1` | Claude Opus 4.1 | 200K | 32K | ✅ | ✅ | $15.00 | $75.00 |
| `claude-opus-4.5` | Claude Opus 4.5 | 200K | 64K | ✅ | ✅ | $5.00 | $25.00 |
| `o1` | o1 | 200K | 100K | ✅ | ✅ | $15.00 | $60.00 |
| `o3` | o3 | 200K | 100K | ✅ | ✅ | $2.00 | $8.00 |
| `o3-deep-research` | o3-deep-research | 200K | 100K | ✅ | ✅ | $10.00 | $40.00 |
| `o3-mini` | o3-mini | 200K | 100K | — | ✅ | $1.10 | $4.40 |
| `o3-pro` | o3 Pro | 200K | 100K | ✅ | ✅ | $20.00 | $80.00 |
| `o4-mini` | o4-mini | 200K | 100K | ✅ | ✅ | $1.10 | $4.40 |
| `sonar-pro` | Sonar Pro | 200K | 8K | ✅ | — | - | - |
| `glm-4.6` | GLM 4.6 | 200K | 96K | — | ✅ | $0.60 | $2.20 |
| `glm-4.7-flash` | GLM 4.7 Flash | 200K | 131K | — | ✅ | $0.07 | $0.40 |
| `glm-4.7-flashx` | GLM 4.7 FlashX | 200K | 128K | — | ✅ | $0.06 | $0.40 |
| `glm-5v-turbo` | GLM 5V Turbo | 200K | 128K | ✅ | ✅ | $1.20 | $4.00 |
| `deepseek-v3` | DeepSeek V3 0324 | 164K | 16K | — | — | $0.77 | $0.77 |
| `deepseek-v3.1` | DeepSeek-V3.1 | 164K | 8K | — | ✅ | $0.56 | $1.68 |
| `qwen3-235b-a22b-thinking` | Qwen3 VL 235B A22B Thinking | 131K | 33K | ✅ | ✅ | $0.40 | $4.00 |
| `qwen3-vl-thinking` | Qwen3 VL 235B A22B Thinking | 131K | 33K | ✅ | ✅ | $0.40 | $4.00 |
| `deepseek-v3.1-terminus` | DeepSeek V3.1 Terminus | 131K | 66K | — | ✅ | $0.27 | $1.00 |
| `kimi-k2` | Kimi K2 Instruct | 131K | 131K | — | — | $0.57 | $2.30 |
| `nemotron-nano-12b-v2-vl` | Nvidia Nemotron Nano 12B V2 VL | 131K | 131K | ✅ | ✅ | $0.20 | $0.60 |
| `nemotron-nano-9b-v2` | Nvidia Nemotron Nano 9B V2 | 131K | 131K | — | ✅ | $0.06 | $0.23 |
| `gpt-oss-20b` | GPT OSS 120B | 131K | 8K | — | ✅ | $0.05 | $0.20 |
| `gpt-oss-safeguard-20b` | GPT OSS Safeguard 20B | 131K | 66K | — | ✅ | $0.07 | $0.30 |
| `grok-3` | Grok 3 Beta | 131K | 131K | — | — | $3.00 | $15.00 |
| `grok-3-fast` | Grok 3 Fast Beta | 131K | 131K | — | — | $5.00 | $25.00 |
| `grok-3-mini` | Grok 3 Mini Beta | 131K | 131K | — | — | $0.30 | $0.50 |
| `grok-3-mini-fast` | Grok 3 Mini Fast Beta | 131K | 131K | — | — | $0.60 | $4.00 |
| `qwen-3-235b` | Qwen3 235B A22b Instruct 2507 | 131K | 40K | — | — | $0.60 | $1.20 |
| `trinity-large-preview` | Trinity Large Preview | 131K | 131K | — | — | $0.25 | $1.00 |
| `glm-4.7` | GLM 4.7 | 131K | 40K | — | ✅ | $2.25 | $2.75 |
| `qwen-3-32b` | Qwen 3 32B | 128K | 8K | — | ✅ | $0.16 | $0.64 |
| `deepseek-r1` | DeepSeek-R1 | 128K | 8K | — | ✅ | $1.35 | $5.40 |
| `deepseek-v3.2` | DeepSeek V3.2 | 128K | 8K | — | — | $0.28 | $0.42 |
| `deepseek-v3.2-thinking` | DeepSeek V3.2 Thinking | 128K | 8K | — | — | $0.62 | $1.85 |
| `mercury-2` | Mercury 2 | 128K | 128K | — | ✅ | $0.25 | $0.75 |
| `longcat-flash-chat` | LongCat Flash Chat | 128K | 100K | — | — | - | - |
| `llama-3.1-70b` | Llama 3.1 70B Instruct | 128K | 8K | — | — | $0.72 | $0.72 |
| `llama-3.1-8b` | Llama 3.1 8B Instruct | 128K | 8K | — | — | $0.22 | $0.22 |
| `llama-3.2-11b` | Llama 3.2 11B Vision Instruct | 128K | 8K | ✅ | — | $0.16 | $0.16 |
| `llama-3.2-90b` | Llama 3.2 90B Vision Instruct | 128K | 8K | ✅ | — | $0.72 | $0.72 |
| `llama-3.3-70b` | Llama 3.3 70B Instruct | 128K | 8K | — | — | $0.72 | $0.72 |
| `llama-4-maverick` | Llama 4 Maverick 17B Instruct | 128K | 8K | ✅ | — | $0.24 | $0.97 |
| `llama-4-scout` | Llama 4 Scout 17B Instruct | 128K | 8K | ✅ | — | $0.17 | $0.66 |
| `codestral` | Mistral Codestral | 128K | 4K | — | — | $0.30 | $0.90 |
| `devstral-small` | Devstral Small 1.1 | 128K | 64K | — | — | $0.10 | $0.30 |
| `ministral-3b` | Ministral 3B | 128K | 4K | — | — | $0.10 | $0.10 |
| `ministral-8b` | Ministral 8B | 128K | 4K | — | — | $0.15 | $0.15 |
| `mistral-medium` | Mistral Medium 3.1 | 128K | 64K | ✅ | — | $0.40 | $2.00 |
| `pixtral-12b` | Pixtral 12B 2409 | 128K | 4K | ✅ | — | $0.15 | $0.15 |
| `pixtral-large` | Pixtral Large | 128K | 4K | ✅ | — | $2.00 | $6.00 |
| `gpt-4-turbo` | GPT-4 Turbo | 128K | 4K | ✅ | — | $10.00 | $30.00 |
| `gpt-4o` | GPT-4o | 128K | 16K | ✅ | — | $2.50 | $10.00 |
| `gpt-4o-mini` | GPT-4o mini | 128K | 16K | ✅ | — | $0.15 | $0.60 |
| `gpt-5-chat` | GPT 5 Chat | 128K | 16K | ✅ | ✅ | $1.25 | $10.00 |
| `gpt-5.1-instant` | GPT-5.1 Instant | 128K | 16K | ✅ | ✅ | $1.25 | $10.00 |
| `gpt-5.2-chat` | GPT 5.2 Chat | 128K | 16K | ✅ | ✅ | $1.75 | $14.00 |
| `gpt-5.3-chat` | GPT-5.3 Chat | 128K | 16K | ✅ | ✅ | $1.75 | $14.00 |
| `glm-4.5` | GLM-4.5 | 128K | 96K | — | ✅ | $0.60 | $2.20 |
| `glm-4.5-air` | GLM 4.5 Air | 128K | 96K | — | ✅ | $0.20 | $1.10 |
| `glm-4.6v` | GLM-4.6V | 128K | 24K | ✅ | ✅ | $0.30 | $0.90 |
| `glm-4.6v-flash` | GLM-4.6V-Flash | 128K | 24K | ✅ | ✅ | - | - |
| `sonar` | Sonar | 127K | 8K | ✅ | — | - | - |
| `glm-4.5v` | GLM 4.5V | 66K | 16K | ✅ | — | $0.60 | $1.80 |
| `qwen-3-14b` | Qwen3-14B | 41K | 16K | — | ✅ | $0.12 | $0.24 |
| `qwen-3-30b` | Qwen3-30B-A3B | 41K | 16K | — | ✅ | $0.08 | $0.29 |
| `mercury-coder-small` | Mercury Coder Small Beta | 32K | 16K | — | — | $0.25 | $1.00 |
| `mistral-small` | Mistral Small | 32K | 4K | ✅ | — | $0.10 | $0.30 |

### xai

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 | 输入 ($/1M) | 输出 ($/1M) |
|---|---|---|---|---|---|---|---|
| `grok-4-1-fast` | Grok 4.1 Fast | 2M | 30K | ✅ | ✅ | $0.20 | $0.50 |
| `grok-4-1-fast-non-reasoning` | Grok 4.1 Fast (Non-Reasoning) | 2M | 30K | ✅ | — | $0.20 | $0.50 |
| `grok-4-fast` | Grok 4 Fast | 2M | 30K | ✅ | ✅ | $0.20 | $0.50 |
| `grok-4-fast-non-reasoning` | Grok 4 Fast (Non-Reasoning) | 2M | 30K | ✅ | — | $0.20 | $0.50 |
| `grok-4.20-0309-non-reasoning` | Grok 4.20 (Non-Reasoning) | 2M | 30K | ✅ | — | $2.00 | $6.00 |
| `grok-4.20-0309-reasoning` | Grok 4.20 (Reasoning) | 2M | 30K | ✅ | ✅ | $2.00 | $6.00 |
| `grok-4.3` | Grok 4.3 | 1M | 30K | ✅ | ✅ | $1.25 | $2.50 |
| `grok-4` | Grok 4 | 256K | 64K | — | ✅ | $3.00 | $15.00 |
| `grok-code-fast-1` | Grok Code Fast 1 | 256K | 10K | — | ✅ | $0.20 | $1.50 |
| `grok-2` | Grok 2 | 131K | 8K | — | — | $2.00 | $10.00 |
| `grok-2-1212` | Grok 2 (1212) | 131K | 8K | — | — | $2.00 | $10.00 |
| `grok-2-latest` | Grok 2 Latest | 131K | 8K | — | — | $2.00 | $10.00 |
| `grok-3` | Grok 3 | 131K | 8K | — | — | $3.00 | $15.00 |
| `grok-3-fast` | Grok 3 Fast | 131K | 8K | — | — | $5.00 | $25.00 |
| `grok-3-fast-latest` | Grok 3 Fast Latest | 131K | 8K | — | — | $5.00 | $25.00 |
| `grok-3-latest` | Grok 3 Latest | 131K | 8K | — | — | $3.00 | $15.00 |
| `grok-3-mini` | Grok 3 Mini | 131K | 8K | — | ✅ | $0.30 | $0.50 |
| `grok-3-mini-fast` | Grok 3 Mini Fast | 131K | 8K | — | ✅ | $0.60 | $4.00 |
| `grok-3-mini-fast-latest` | Grok 3 Mini Fast Latest | 131K | 8K | — | ✅ | $0.60 | $4.00 |
| `grok-3-mini-latest` | Grok 3 Mini Latest | 131K | 8K | — | ✅ | $0.30 | $0.50 |
| `grok-beta` | Grok Beta | 131K | 4K | — | — | $5.00 | $15.00 |
| `grok-2-vision` | Grok 2 Vision | 8K | 4K | ✅ | — | $2.00 | $10.00 |
| `grok-2-vision-1212` | Grok 2 Vision (1212) | 8K | 4K | ✅ | — | $2.00 | $10.00 |
| `grok-2-vision-latest` | Grok 2 Vision Latest | 8K | 4K | ✅ | — | $2.00 | $10.00 |
| `grok-vision-beta` | Grok Vision Beta | 8K | 4K | ✅ | — | $5.00 | $15.00 |

### xiaomi

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 | 输入 ($/1M) | 输出 ($/1M) |
|---|---|---|---|---|---|---|---|
| `mimo-v2-pro` | MiMo-V2-Pro | 1M | 131K | — | ✅ | $1.00 | $3.00 |
| `mimo-v2.5` | MiMo-V2.5 | 1M | 131K | ✅ | ✅ | $0.40 | $2.00 |
| `mimo-v2.5-pro` | MiMo-V2.5-Pro | 1M | 131K | — | ✅ | $1.00 | $3.00 |
| `mimo-v2-flash` | MiMo-V2-Flash | 262K | 66K | — | ✅ | $0.10 | $0.30 |
| `mimo-v2-omni` | MiMo-V2-Omni | 262K | 131K | ✅ | ✅ | $0.40 | $2.00 |

### xiaomi-token-plan-ams

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 | 输入 ($/1M) | 输出 ($/1M) |
|---|---|---|---|---|---|---|---|
| `mimo-v2-pro` | MiMo-V2-Pro | 1M | 131K | — | ✅ | - | - |
| `mimo-v2.5` | MiMo-V2.5 | 1M | 131K | ✅ | ✅ | - | - |
| `mimo-v2.5-pro` | MiMo-V2.5-Pro | 1M | 131K | — | ✅ | - | - |
| `mimo-v2-flash` | MiMo-V2-Flash | 262K | 66K | — | ✅ | - | - |
| `mimo-v2-omni` | MiMo-V2-Omni | 262K | 131K | ✅ | ✅ | - | - |

### xiaomi-token-plan-cn

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 | 输入 ($/1M) | 输出 ($/1M) |
|---|---|---|---|---|---|---|---|
| `mimo-v2-pro` | MiMo-V2-Pro | 1M | 131K | — | ✅ | - | - |
| `mimo-v2.5` | MiMo-V2.5 | 1M | 131K | ✅ | ✅ | - | - |
| `mimo-v2.5-pro` | MiMo-V2.5-Pro | 1M | 131K | — | ✅ | - | - |
| `mimo-v2-flash` | MiMo-V2-Flash | 262K | 66K | — | ✅ | - | - |
| `mimo-v2-omni` | MiMo-V2-Omni | 262K | 131K | ✅ | ✅ | - | - |

### xiaomi-token-plan-sgp

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 | 输入 ($/1M) | 输出 ($/1M) |
|---|---|---|---|---|---|---|---|
| `mimo-v2-pro` | MiMo-V2-Pro | 1M | 131K | — | ✅ | - | - |
| `mimo-v2.5` | MiMo-V2.5 | 1M | 131K | ✅ | ✅ | - | - |
| `mimo-v2.5-pro` | MiMo-V2.5-Pro | 1M | 131K | — | ✅ | - | - |
| `mimo-v2-flash` | MiMo-V2-Flash | 262K | 66K | — | ✅ | - | - |
| `mimo-v2-omni` | MiMo-V2-Omni | 262K | 131K | ✅ | ✅ | - | - |

### zai

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 | 输入 ($/1M) | 输出 ($/1M) |
|---|---|---|---|---|---|---|---|
| `glm-4.6` | GLM-4.6 | 205K | 131K | — | ✅ | $0.60 | $2.20 |
| `glm-4.7` | GLM-4.7 | 205K | 131K | — | ✅ | $0.60 | $2.20 |
| `glm-5` | GLM-5 | 205K | 131K | — | ✅ | $1.00 | $3.20 |
| `glm-4.7-flash` | GLM-4.7-Flash | 200K | 131K | — | ✅ | - | - |
| `glm-4.7-flashx` | GLM-4.7-FlashX | 200K | 131K | — | ✅ | $0.07 | $0.40 |
| `glm-5-turbo` | GLM-5-Turbo | 200K | 131K | — | ✅ | $1.20 | $4.00 |
| `glm-5.1` | GLM-5.1 | 200K | 131K | — | ✅ | $1.40 | $4.40 |
| `glm-5v-turbo` | GLM-5V-Turbo | 200K | 131K | ✅ | ✅ | $1.20 | $4.00 |
| `glm-4.5` | GLM-4.5 | 131K | 98K | — | ✅ | $0.60 | $2.20 |
| `glm-4.5-air` | GLM-4.5-Air | 131K | 98K | — | ✅ | $0.20 | $1.10 |
| `glm-4.5-flash` | GLM-4.5-Flash | 131K | 98K | — | ✅ | - | - |
| `glm-4.6v` | GLM-4.6V | 128K | 33K | ✅ | ✅ | $0.30 | $0.90 |
| `glm-4.5v` | GLM-4.5V | 64K | 16K | ✅ | ✅ | $0.60 | $1.80 |

### zhipuai

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 | 输入 ($/1M) | 输出 ($/1M) |
|---|---|---|---|---|---|---|---|
| `glm-4.6` | GLM-4.6 | 205K | 131K | — | ✅ | $0.60 | $2.20 |
| `glm-4.7` | GLM-4.7 | 205K | 131K | — | ✅ | $0.60 | $2.20 |
| `glm-5` | GLM-5 | 205K | 131K | — | ✅ | $1.00 | $3.20 |
| `glm-4.7-flash` | GLM-4.7-Flash | 200K | 131K | — | ✅ | - | - |
| `glm-4.7-flashx` | GLM-4.7-FlashX | 200K | 131K | — | ✅ | $0.07 | $0.40 |
| `glm-5.1` | GLM-5.1 | 200K | 131K | — | ✅ | $6.00 | $24.00 |
| `glm-5v-turbo` | GLM-5V-Turbo | 200K | 131K | ✅ | ✅ | $5.00 | $22.00 |
| `glm-4.5` | GLM-4.5 | 131K | 98K | — | ✅ | $0.60 | $2.20 |
| `glm-4.5-air` | GLM-4.5-Air | 131K | 98K | — | ✅ | $0.20 | $1.10 |
| `glm-4.5-flash` | GLM-4.5-Flash | 131K | 98K | — | ✅ | - | - |
| `glm-4.6v` | GLM-4.6V | 128K | 33K | ✅ | ✅ | $0.30 | $0.90 |
| `glm-4.5v` | GLM-4.5V | 64K | 16K | ✅ | ✅ | $0.60 | $1.80 |

