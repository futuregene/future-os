# 内置模型目录

915 个模型，覆盖 30 个 Provider。

## Provider 概览

| Provider | Models |
|---|---|
| Amazon Bedrock | 89 |
| Anthropic | 24 |
| azure-openai-responses | 41 |
| cerebras | 4 |
| cloudflare-workers-ai | 8 |
| DeepSeek | 4 |
| github-copilot | 26 |
| Google | 28 |
| google-vertex | 43 |
| groq | 18 |
| huggingface | 22 |
| kimi-coding | 2 |
| minimax | 6 |
| minimax-cn | 6 |
| Mistral | 27 |
| moonshotai | 7 |
| moonshotai-cn | 7 |
| OpenAI | 44 |
| openai-codex | 1 |
| opencode | 2 |
| opencode-go | 2 |
| OpenRouter | 272 |
| vercel-ai-gateway | 162 |
| xai | 25 |
| xiaomi | 5 |
| xiaomi-token-plan-ams | 5 |
| xiaomi-token-plan-cn | 5 |
| xiaomi-token-plan-sgp | 5 |
| zai | 13 |
| zhipuai | 12 |

---

## 各 Provider 详情

### Amazon Bedrock

**Base URL:** `https://bedrock-runtime.eu-central-1.amazonaws.com`, `https://bedrock-runtime.us-east-1.amazonaws.com`

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 |
|---|---|---|---|---|---|
| `meta.llama4-scout-17b-instruct-v1:0` | Llama 4 Scout 17B Instruct | 4M | 16K | ✅ | — |
| `us.meta.llama4-scout-17b-instruct-v1:0` | Llama 4 Scout 17B Instruct (US) | 4M | 16K | ✅ | — |
| `writer.palmyra-x5-v1:0` | Palmyra X5 | 1M | 8K | — | ✅ |
| `anthropic.claude-opus-4-6-v1` | Claude Opus 4.6 | 1M | 128K | ✅ | ✅ |
| `anthropic.claude-opus-4-7` | Claude Opus 4.7 | 1M | 128K | ✅ | ✅ |
| `anthropic.claude-opus-4-8` | Claude Opus 4.8 | 1M | 128K | ✅ | ✅ |
| `anthropic.claude-sonnet-4-6` | Claude Sonnet 4.6 | 1M | 64K | ✅ | ✅ |
| `au.anthropic.claude-opus-4-6-v1` | AU Anthropic Claude Opus 4.6 | 1M | 128K | ✅ | ✅ |
| `au.anthropic.claude-sonnet-4-6` | AU Anthropic Claude Sonnet 4.6 | 1M | 128K | ✅ | ✅ |
| `eu.anthropic.claude-opus-4-6-v1` | Claude Opus 4.6 (EU) | 1M | 128K | ✅ | ✅ |
| `eu.anthropic.claude-opus-4-7` | Claude Opus 4.7 (EU) | 1M | 128K | ✅ | ✅ |
| `eu.anthropic.claude-opus-4-8` | Claude Opus 4.8 (EU) | 1M | 128K | ✅ | ✅ |
| `eu.anthropic.claude-sonnet-4-6` | Claude Sonnet 4.6 (EU) | 1M | 64K | ✅ | ✅ |
| `global.anthropic.claude-opus-4-6-v1` | Claude Opus 4.6 (Global) | 1M | 128K | ✅ | ✅ |
| `global.anthropic.claude-opus-4-7` | Claude Opus 4.7 (Global) | 1M | 128K | ✅ | ✅ |
| `global.anthropic.claude-opus-4-8` | Claude Opus 4.8 (Global) | 1M | 128K | ✅ | ✅ |
| `global.anthropic.claude-sonnet-4-6` | Claude Sonnet 4.6 (Global) | 1M | 64K | ✅ | ✅ |
| `jp.anthropic.claude-opus-4-7` | Claude Opus 4.7 (JP) | 1M | 128K | ✅ | ✅ |
| `jp.anthropic.claude-opus-4-8` | Claude Opus 4.8 (JP) | 1M | 128K | ✅ | ✅ |
| `jp.anthropic.claude-sonnet-4-6` | Claude Sonnet 4.6 (JP) | 1M | 64K | ✅ | ✅ |
| `meta.llama4-maverick-17b-instruct-v1:0` | Llama 4 Maverick 17B Instruct | 1M | 16K | ✅ | — |
| `us.anthropic.claude-opus-4-6-v1` | Claude Opus 4.6 (US) | 1M | 128K | ✅ | ✅ |
| `us.anthropic.claude-opus-4-7` | Claude Opus 4.7 (US) | 1M | 128K | ✅ | ✅ |
| `us.anthropic.claude-opus-4-8` | Claude Opus 4.8 (US) | 1M | 128K | ✅ | ✅ |
| `us.anthropic.claude-sonnet-4-6` | Claude Sonnet 4.6 (US) | 1M | 64K | ✅ | ✅ |
| `us.meta.llama4-maverick-17b-instruct-v1:0` | Llama 4 Maverick 17B Instruct (US) | 1M | 16K | ✅ | — |
| `amazon.nova-lite-v1:0` | Nova Lite | 300K | 8K | ✅ | — |
| `amazon.nova-pro-v1:0` | Nova Pro | 300K | 8K | ✅ | — |
| `nvidia.nemotron-super-3-120b` | NVIDIA Nemotron 3 Super 120B A12B | 262K | 131K | — | ✅ |
| `qwen.qwen3-235b-a22b-2507-v1:0` | Qwen3 235B A22B 2507 | 262K | 131K | — | — |
| `qwen.qwen3-coder-30b-a3b-v1:0` | Qwen3 Coder 30B A3B Instruct | 262K | 131K | — | — |
| `qwen.qwen3-next-80b-a3b` | Qwen/Qwen3-Next-80B-A3B-Instruct | 262K | 262K | — | — |
| `qwen.qwen3-vl-235b-a22b` | Qwen/Qwen3-VL-235B-A22B-Instruct | 262K | 262K | ✅ | — |
| `mistral.devstral-2-123b` | Devstral 2 123B | 256K | 8K | — | — |
| `mistral.ministral-3-3b-instruct` | Ministral 3 3B | 256K | 8K | ✅ | — |
| `mistral.mistral-large-3-675b-instruct` | Mistral Large 3 | 256K | 8K | ✅ | — |
| `moonshot.kimi-k2-thinking` | Kimi K2 Thinking | 256K | 256K | — | ✅ |
| `moonshotai.kimi-k2.5` | Kimi K2.5 | 256K | 256K | ✅ | ✅ |
| `minimax.minimax-m2.1` | MiniMax M2.1 | 205K | 131K | — | ✅ |
| `zai.glm-4.7` | GLM-4.7 | 205K | 131K | — | ✅ |
| `minimax.minimax-m2` | MiniMax M2 | 205K | 128K | — | ✅ |
| `google.gemma-3-27b-it` | Google Gemma 3 27B Instruct | 203K | 8K | ✅ | — |
| `zai.glm-5` | GLM-5 | 203K | 101K | — | ✅ |
| `anthropic.claude-haiku-4-5-20251001-v1:0` | Claude Haiku 4.5 | 200K | 64K | ✅ | ✅ |
| `anthropic.claude-opus-4-1-20250805-v1:0` | Claude Opus 4.1 | 200K | 32K | ✅ | ✅ |
| `anthropic.claude-opus-4-5-20251101-v1:0` | Claude Opus 4.5 | 200K | 64K | ✅ | ✅ |
| `anthropic.claude-sonnet-4-5-20250929-v1:0` | Claude Sonnet 4.5 | 200K | 64K | ✅ | ✅ |
| `au.anthropic.claude-haiku-4-5-20251001-v1:0` | Claude Haiku 4.5 (AU) | 200K | 64K | ✅ | ✅ |
| `au.anthropic.claude-sonnet-4-5-20250929-v1:0` | Claude Sonnet 4.5 (AU) | 200K | 64K | ✅ | ✅ |
| `eu.anthropic.claude-haiku-4-5-20251001-v1:0` | Claude Haiku 4.5 (EU) | 200K | 64K | ✅ | ✅ |
| `eu.anthropic.claude-opus-4-5-20251101-v1:0` | Claude Opus 4.5 (EU) | 200K | 64K | ✅ | ✅ |
| `eu.anthropic.claude-sonnet-4-5-20250929-v1:0` | Claude Sonnet 4.5 (EU) | 200K | 64K | ✅ | ✅ |
| `global.anthropic.claude-haiku-4-5-20251001-v1:0` | Claude Haiku 4.5 (Global) | 200K | 64K | ✅ | ✅ |
| `global.anthropic.claude-opus-4-5-20251101-v1:0` | Claude Opus 4.5 (Global) | 200K | 64K | ✅ | ✅ |
| `global.anthropic.claude-sonnet-4-5-20250929-v1:0` | Claude Sonnet 4.5 (Global) | 200K | 64K | ✅ | ✅ |
| `jp.anthropic.claude-sonnet-4-5-20250929-v1:0` | Claude Sonnet 4.5 (JP) | 200K | 64K | ✅ | ✅ |
| `us.anthropic.claude-haiku-4-5-20251001-v1:0` | Claude Haiku 4.5 (US) | 200K | 64K | ✅ | ✅ |
| `us.anthropic.claude-opus-4-1-20250805-v1:0` | Claude Opus 4.1 (US) | 200K | 32K | ✅ | ✅ |
| `us.anthropic.claude-opus-4-5-20251101-v1:0` | Claude Opus 4.5 (US) | 200K | 64K | ✅ | ✅ |
| `us.anthropic.claude-sonnet-4-5-20250929-v1:0` | Claude Sonnet 4.5 (US) | 200K | 64K | ✅ | ✅ |
| `zai.glm-4.7-flash` | GLM-4.7-Flash | 200K | 131K | — | ✅ |
| `minimax.minimax-m2.5` | MiniMax M2.5 | 197K | 98K | — | ✅ |
| `deepseek.v3-v1:0` | DeepSeek-V3.1 | 164K | 82K | — | ✅ |
| `deepseek.v3.2` | DeepSeek-V3.2 | 164K | 82K | — | ✅ |
| `qwen.qwen3-coder-480b-a35b-v1:0` | Qwen3 Coder 480B A35B Instruct | 131K | 66K | — | — |
| `qwen.qwen3-coder-next` | Qwen3 Coder Next | 131K | 66K | — | ✅ |
| `amazon.nova-2-lite-v1:0` | Nova 2 Lite | 128K | 4K | ✅ | — |
| `amazon.nova-micro-v1:0` | Nova Micro | 128K | 8K | — | — |
| `deepseek.r1-v1:0` | DeepSeek-R1 | 128K | 33K | — | ✅ |
| `google.gemma-3-4b-it` | Gemma 3 4B IT | 128K | 4K | ✅ | — |
| `meta.llama3-1-70b-instruct-v1:0` | Llama 3.1 70B Instruct | 128K | 4K | — | — |
| `meta.llama3-1-8b-instruct-v1:0` | Llama 3.1 8B Instruct | 128K | 4K | — | — |
| `meta.llama3-3-70b-instruct-v1:0` | Llama 3.3 70B Instruct | 128K | 4K | — | — |
| `mistral.magistral-small-2509` | Magistral Small 1.2 | 128K | 40K | ✅ | ✅ |
| `mistral.ministral-3-14b-instruct` | Ministral 14B 3.0 | 128K | 4K | — | — |
| `mistral.ministral-3-8b-instruct` | Ministral 3 8B | 128K | 4K | — | — |
| `mistral.pixtral-large-2502-v1:0` | Pixtral Large (25.02) | 128K | 8K | ✅ | — |
| `mistral.voxtral-mini-3b-2507` | Voxtral Mini 3B 2507 | 128K | 4K | — | — |
| `nvidia.nemotron-nano-12b-v2` | NVIDIA Nemotron Nano 12B v2 VL BF16 | 128K | 4K | ✅ | — |
| `nvidia.nemotron-nano-3-30b` | NVIDIA Nemotron Nano 3 30B | 128K | 4K | — | ✅ |
| `nvidia.nemotron-nano-9b-v2` | NVIDIA Nemotron Nano 9B v2 | 128K | 4K | — | — |
| `openai.gpt-oss-120b-1:0` | gpt-oss-120b | 128K | 4K | — | — |
| `openai.gpt-oss-20b-1:0` | gpt-oss-20b | 128K | 4K | — | — |
| `openai.gpt-oss-safeguard-120b` | GPT OSS Safeguard 120B | 128K | 4K | — | — |
| `openai.gpt-oss-safeguard-20b` | GPT OSS Safeguard 20B | 128K | 4K | — | — |
| `us.deepseek.r1-v1:0` | DeepSeek-R1 (US) | 128K | 33K | — | ✅ |
| `writer.palmyra-x4-v1:0` | Palmyra X4 | 123K | 8K | — | ✅ |
| `mistral.voxtral-small-24b-2507` | Voxtral Small 24B 2507 | 32K | 8K | — | — |
| `qwen.qwen3-32b-v1:0` | Qwen3 32B (dense) | 16K | 16K | — | ✅ |

### Anthropic

**Base URL:** `https://api.anthropic.com/v1`

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 |
|---|---|---|---|---|---|
| `claude-opus-4-6` | Claude Opus 4.6 | 1M | 128K | ✅ | ✅ |
| `claude-opus-4-7` | Claude Opus 4.7 | 1M | 128K | ✅ | ✅ |
| `claude-opus-4-8` | Claude Opus 4.8 | 1M | 128K | ✅ | ✅ |
| `claude-sonnet-4-6` | Claude Sonnet 4.6 | 1M | 64K | ✅ | ✅ |
| `claude-3-5-haiku-20241022` | Claude Haiku 3.5 | 200K | 8K | ✅ | — |
| `claude-3-5-haiku-latest` | Claude Haiku 3.5 (latest) | 200K | 8K | ✅ | — |
| `claude-3-5-sonnet-20240620` | Claude Sonnet 3.5 | 200K | 8K | ✅ | — |
| `claude-3-5-sonnet-20241022` | Claude Sonnet 3.5 v2 | 200K | 8K | ✅ | — |
| `claude-3-7-sonnet-20250219` | Claude Sonnet 3.7 | 200K | 64K | ✅ | ✅ |
| `claude-3-haiku-20240307` | Claude Haiku 3 | 200K | 4K | ✅ | — |
| `claude-3-opus-20240229` | Claude Opus 3 | 200K | 4K | ✅ | — |
| `claude-3-sonnet-20240229` | Claude Sonnet 3 | 200K | 4K | ✅ | — |
| `claude-haiku-4-5` | Claude Haiku 4.5 (latest) | 200K | 64K | ✅ | ✅ |
| `claude-haiku-4-5-20251001` | Claude Haiku 4.5 | 200K | 64K | ✅ | ✅ |
| `claude-opus-4-0` | Claude Opus 4 (latest) | 200K | 32K | ✅ | ✅ |
| `claude-opus-4-1` | Claude Opus 4.1 (latest) | 200K | 32K | ✅ | ✅ |
| `claude-opus-4-1-20250805` | Claude Opus 4.1 | 200K | 32K | ✅ | ✅ |
| `claude-opus-4-20250514` | Claude Opus 4 | 200K | 32K | ✅ | ✅ |
| `claude-opus-4-5` | Claude Opus 4.5 (latest) | 200K | 64K | ✅ | ✅ |
| `claude-opus-4-5-20251101` | Claude Opus 4.5 | 200K | 64K | ✅ | ✅ |
| `claude-sonnet-4-0` | Claude Sonnet 4 (latest) | 200K | 64K | ✅ | ✅ |
| `claude-sonnet-4-20250514` | Claude Sonnet 4 | 200K | 64K | ✅ | ✅ |
| `claude-sonnet-4-5` | Claude Sonnet 4.5 (latest) | 200K | 64K | ✅ | ✅ |
| `claude-sonnet-4-5-20250929` | Claude Sonnet 4.5 | 200K | 64K | ✅ | ✅ |

### azure-openai-responses

**Base URL:** `https://YOUR_RESOURCE.openai.azure.com/openai/v1`

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 |
|---|---|---|---|---|---|
| `gpt-5.4` | GPT-5.4 | 1M | 128K | ✅ | ✅ |
| `gpt-5.4-pro` | GPT-5.4 Pro | 1M | 128K | ✅ | ✅ |
| `gpt-5.5` | GPT-5.5 | 1M | 128K | ✅ | ✅ |
| `gpt-5.5-pro` | GPT-5.5 Pro | 1M | 128K | ✅ | ✅ |
| `gpt-4.1` | GPT-4.1 | 1M | 33K | ✅ | — |
| `gpt-4.1-mini` | GPT-4.1 mini | 1M | 33K | ✅ | — |
| `gpt-4.1-nano` | GPT-4.1 nano | 1M | 33K | ✅ | — |
| `gpt-5` | GPT-5 | 400K | 128K | ✅ | ✅ |
| `gpt-5-codex` | GPT-5-Codex | 400K | 128K | ✅ | ✅ |
| `gpt-5-mini` | GPT-5 Mini | 400K | 128K | ✅ | ✅ |
| `gpt-5-nano` | GPT-5 Nano | 400K | 128K | ✅ | ✅ |
| `gpt-5-pro` | GPT-5 Pro | 400K | 272K | ✅ | ✅ |
| `gpt-5.1` | GPT-5.1 | 400K | 128K | ✅ | ✅ |
| `gpt-5.1-codex` | GPT-5.1 Codex | 400K | 128K | ✅ | ✅ |
| `gpt-5.1-codex-max` | GPT-5.1 Codex Max | 400K | 128K | ✅ | ✅ |
| `gpt-5.1-codex-mini` | GPT-5.1 Codex mini | 400K | 128K | ✅ | ✅ |
| `gpt-5.2` | GPT-5.2 | 400K | 128K | ✅ | ✅ |
| `gpt-5.2-codex` | GPT-5.2 Codex | 400K | 128K | ✅ | ✅ |
| `gpt-5.2-pro` | GPT-5.2 Pro | 400K | 128K | ✅ | ✅ |
| `gpt-5.3-codex` | GPT-5.3 Codex | 400K | 128K | ✅ | ✅ |
| `gpt-5.4-mini` | GPT-5.4 mini | 400K | 128K | ✅ | ✅ |
| `gpt-5.4-nano` | GPT-5.4 nano | 400K | 128K | ✅ | ✅ |
| `o1` | o1 | 200K | 100K | ✅ | ✅ |
| `o1-pro` | o1-pro | 200K | 100K | ✅ | ✅ |
| `o3` | o3 | 200K | 100K | ✅ | ✅ |
| `o3-deep-research` | o3-deep-research | 200K | 100K | ✅ | ✅ |
| `o3-mini` | o3-mini | 200K | 100K | — | ✅ |
| `o3-pro` | o3-pro | 200K | 100K | ✅ | ✅ |
| `o4-mini` | o4-mini | 200K | 100K | ✅ | ✅ |
| `o4-mini-deep-research` | o4-mini-deep-research | 200K | 100K | ✅ | ✅ |
| `gpt-4-turbo` | GPT-4 Turbo | 128K | 4K | ✅ | — |
| `gpt-4o` | GPT-4o | 128K | 16K | ✅ | — |
| `gpt-4o-2024-05-13` | GPT-4o (2024-05-13) | 128K | 4K | ✅ | — |
| `gpt-4o-2024-08-06` | GPT-4o (2024-08-06) | 128K | 16K | ✅ | — |
| `gpt-4o-2024-11-20` | GPT-4o (2024-11-20) | 128K | 16K | ✅ | — |
| `gpt-4o-mini` | GPT-4o mini | 128K | 16K | ✅ | — |
| `gpt-5.1-chat-latest` | GPT-5.1 Chat | 128K | 16K | ✅ | ✅ |
| `gpt-5.2-chat-latest` | GPT-5.2 Chat | 128K | 16K | ✅ | ✅ |
| `gpt-5.3-chat-latest` | GPT-5.3 Chat (latest) | 128K | 16K | ✅ | — |
| `gpt-5.3-codex-spark` | GPT-5.3 Codex Spark | 128K | 32K | ✅ | ✅ |
| `gpt-4` | GPT-4 | 8K | 8K | — | — |

### cerebras

**Base URL:** `https://api.cerebras.ai/v1`

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 |
|---|---|---|---|---|---|
| `gpt-oss-120b` | GPT OSS 120B | 131K | 33K | — | ✅ |
| `zai-glm-4.7` | Z.AI GLM-4.7 | 131K | 40K | — | — |
| `qwen-3-235b-a22b-instruct-2507` | Qwen 3 235B Instruct | 131K | 32K | — | — |
| `llama3.1-8b` | Llama 3.1 8B | 32K | 8K | — | — |

### cloudflare-workers-ai

**Base URL:** `https://api.cloudflare.com/client/v4/accounts`

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 |
|---|---|---|---|---|---|
| `gemma-4-26b-a4b-it` | Gemma 4 26B A4B IT | 256K | 16K | ✅ | ✅ |
| `kimi-k2.5` | Kimi K2.5 | 256K | 256K | ✅ | ✅ |
| `kimi-k2.6` | Kimi K2.6 | 256K | 256K | ✅ | ✅ |
| `nemotron-3-120b-a12b` | Nemotron 3 Super 120B | 256K | 256K | — | ✅ |
| `glm-4.7-flash` | GLM-4.7-Flash | 131K | 131K | — | ✅ |
| `llama-4-scout-17b-16e-instruct` | Llama 4 Scout 17B 16E Instruct | 128K | 16K | ✅ | — |
| `gpt-oss-120b` | GPT OSS 120B | 128K | 16K | — | ✅ |
| `gpt-oss-20b` | GPT OSS 20B | 128K | 16K | — | ✅ |

### DeepSeek

**Base URL:** `https://api.deepseek.com/v1`

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 |
|---|---|---|---|---|---|
| `deepseek-chat` | DeepSeek Chat | 1M | 384K | — | — |
| `deepseek-reasoner` | DeepSeek Reasoner | 1M | 384K | — | ✅ |
| `deepseek-v4-flash` | DeepSeek V4 Flash | 1M | 384K | — | ✅ |
| `deepseek-v4-pro` | DeepSeek V4 Pro | 1M | 384K | — | ✅ |

### github-copilot

**Base URL:** `https://models.githubcopilot.com/v1`

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 |
|---|---|---|---|---|---|
| `gpt-5.1-codex` | GPT-5.1-Codex | 400K | 128K | ✅ | ✅ |
| `gpt-5.1-codex-max` | GPT-5.1-Codex-max | 400K | 128K | ✅ | ✅ |
| `gpt-5.1-codex-mini` | GPT-5.1-Codex-mini | 400K | 128K | ✅ | ✅ |
| `gpt-5.2-codex` | GPT-5.2-Codex | 400K | 128K | ✅ | ✅ |
| `gpt-5.3-codex` | GPT-5.3-Codex | 400K | 128K | ✅ | ✅ |
| `gpt-5.4` | GPT-5.4 | 400K | 128K | ✅ | ✅ |
| `gpt-5.4-mini` | GPT-5.4 Mini | 400K | 128K | ✅ | ✅ |
| `gpt-5.5` | GPT-5.5 | 400K | 128K | ✅ | ✅ |
| `gpt-5-mini` | GPT-5-mini | 264K | 64K | ✅ | ✅ |
| `gpt-5.1` | GPT-5.1 | 264K | 64K | ✅ | ✅ |
| `gpt-5.2` | GPT-5.2 | 264K | 64K | ✅ | ✅ |
| `claude-sonnet-4` | Claude Sonnet 4 | 216K | 16K | ✅ | ✅ |
| `claude-sonnet-4.6` | Claude Sonnet 4.6 | 200K | 32K | ✅ | ✅ |
| `claude-opus-4.5` | Claude Opus 4.5 | 160K | 32K | ✅ | ✅ |
| `claude-haiku-4.5` | Claude Haiku 4.5 | 144K | 32K | ✅ | ✅ |
| `claude-opus-4.6` | Claude Opus 4.6 | 144K | 64K | ✅ | ✅ |
| `claude-opus-4.7` | Claude Opus 4.7 | 144K | 64K | ✅ | ✅ |
| `claude-sonnet-4.5` | Claude Sonnet 4.5 | 144K | 32K | ✅ | ✅ |
| `gemini-2.5-pro` | Gemini 2.5 Pro | 128K | 64K | ✅ | — |
| `gemini-3-flash-preview` | Gemini 3 Flash | 128K | 64K | ✅ | ✅ |
| `gemini-3-pro-preview` | Gemini 3 Pro Preview | 128K | 64K | ✅ | ✅ |
| `gemini-3.1-pro-preview` | Gemini 3.1 Pro Preview | 128K | 64K | ✅ | ✅ |
| `gpt-4.1` | GPT-4.1 | 128K | 16K | ✅ | — |
| `gpt-4o` | GPT-4o | 128K | 4K | ✅ | — |
| `gpt-5` | GPT-5 | 128K | 128K | ✅ | ✅ |
| `grok-code-fast-1` | Grok Code Fast 1 | 128K | 64K | — | ✅ |

### Google

**Base URL:** `https://generativelanguage.googleapis.com/v1beta/openai`

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 |
|---|---|---|---|---|---|
| `gemini-2.0-flash` | Gemini 2.0 Flash | 1M | 8K | ✅ | — |
| `gemini-2.0-flash-lite` | Gemini 2.0 Flash Lite | 1M | 8K | ✅ | — |
| `gemini-2.5-flash` | Gemini 2.5 Flash | 1M | 66K | ✅ | ✅ |
| `gemini-2.5-flash-lite` | Gemini 2.5 Flash Lite | 1M | 66K | ✅ | ✅ |
| `gemini-2.5-flash-lite-preview-06-17` | Gemini 2.5 Flash Lite Preview 06-17 | 1M | 66K | ✅ | ✅ |
| `gemini-2.5-flash-lite-preview-09-2025` | Gemini 2.5 Flash Lite Preview 09-25 | 1M | 66K | ✅ | ✅ |
| `gemini-2.5-flash-preview-04-17` | Gemini 2.5 Flash Preview 04-17 | 1M | 66K | ✅ | ✅ |
| `gemini-2.5-flash-preview-05-20` | Gemini 2.5 Flash Preview 05-20 | 1M | 66K | ✅ | ✅ |
| `gemini-2.5-flash-preview-09-2025` | Gemini 2.5 Flash Preview 09-25 | 1M | 66K | ✅ | ✅ |
| `gemini-2.5-pro` | Gemini 2.5 Pro | 1M | 66K | ✅ | ✅ |
| `gemini-2.5-pro-preview-05-06` | Gemini 2.5 Pro Preview 05-06 | 1M | 66K | ✅ | ✅ |
| `gemini-2.5-pro-preview-06-05` | Gemini 2.5 Pro Preview 06-05 | 1M | 66K | ✅ | ✅ |
| `gemini-3-flash-preview` | Gemini 3 Flash Preview | 1M | 66K | ✅ | ✅ |
| `gemini-3.1-flash-lite` | Gemini 3.1 Flash Lite | 1M | 66K | ✅ | ✅ |
| `gemini-3.1-flash-lite-preview` | Gemini 3.1 Flash Lite Preview | 1M | 66K | ✅ | ✅ |
| `gemini-3.1-pro-preview` | Gemini 3.1 Pro Preview | 1M | 66K | ✅ | ✅ |
| `gemini-3.1-pro-preview-customtools` | Gemini 3.1 Pro Preview Custom Tools | 1M | 66K | ✅ | ✅ |
| `gemini-flash-latest` | Gemini Flash Latest | 1M | 66K | ✅ | ✅ |
| `gemini-flash-lite-latest` | Gemini Flash-Lite Latest | 1M | 66K | ✅ | ✅ |
| `gemini-1.5-flash` | Gemini 1.5 Flash | 1M | 8K | ✅ | — |
| `gemini-1.5-flash-8b` | Gemini 1.5 Flash-8B | 1M | 8K | ✅ | — |
| `gemini-1.5-pro` | Gemini 1.5 Pro | 1M | 8K | ✅ | — |
| `gemini-3-pro-preview` | Gemini 3 Pro Preview | 1M | 64K | ✅ | ✅ |
| `gemma-4-26b-a4b-it` | Gemma 4 26B | 256K | 8K | ✅ | ✅ |
| `gemma-4-31b-it` | Gemma 4 31B | 256K | 8K | ✅ | ✅ |
| `gemini-live-2.5-flash-preview-native-audio` | Gemini Live 2.5 Flash Preview Native Audio | 131K | 66K | — | ✅ |
| `gemma-3-27b-it` | Gemma 3 27B | 131K | 8K | ✅ | — |
| `gemini-live-2.5-flash` | Gemini Live 2.5 Flash | 128K | 8K | ✅ | ✅ |

### google-vertex

**Base URL:** `https://LOCATION-aiplatform.googleapis.com/v1beta1/projects/PROJECT_ID/locations/LOCATION/endpoints/openapi`

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 |
|---|---|---|---|---|---|
| `gemini-2.0-flash` | Gemini 2.0 Flash | 1M | 8K | ✅ | — |
| `gemini-2.0-flash-lite` | Gemini 2.0 Flash Lite | 1M | 8K | ✅ | — |
| `gemini-2.5-flash` | Gemini 2.5 Flash | 1M | 66K | ✅ | ✅ |
| `gemini-2.5-flash-lite` | Gemini 2.5 Flash Lite | 1M | 66K | ✅ | ✅ |
| `gemini-2.5-flash-lite-preview-09-2025` | Gemini 2.5 Flash Lite Preview 09-25 | 1M | 66K | ✅ | ✅ |
| `gemini-2.5-flash-preview-04-17` | Gemini 2.5 Flash Preview 04-17 | 1M | 66K | ✅ | ✅ |
| `gemini-2.5-flash-preview-05-20` | Gemini 2.5 Flash Preview 05-20 | 1M | 66K | ✅ | ✅ |
| `gemini-2.5-flash-preview-09-2025` | Gemini 2.5 Flash Preview 09-25 | 1M | 66K | ✅ | ✅ |
| `gemini-2.5-pro` | Gemini 2.5 Pro | 1M | 66K | ✅ | ✅ |
| `gemini-2.5-pro-preview-05-06` | Gemini 2.5 Pro Preview 05-06 | 1M | 66K | ✅ | ✅ |
| `gemini-2.5-pro-preview-06-05` | Gemini 2.5 Pro Preview 06-05 | 1M | 66K | ✅ | ✅ |
| `gemini-3-flash-preview` | Gemini 3 Flash Preview | 1M | 66K | ✅ | ✅ |
| `gemini-3-pro-preview` | Gemini 3 Pro Preview | 1M | 66K | ✅ | ✅ |
| `gemini-3.1-flash-lite` | Gemini 3.1 Flash Lite | 1M | 66K | ✅ | ✅ |
| `gemini-3.1-flash-lite-preview` | Gemini 3.1 Flash Lite Preview | 1M | 66K | ✅ | ✅ |
| `gemini-3.1-pro-preview` | Gemini 3.1 Pro Preview | 1M | 66K | ✅ | ✅ |
| `gemini-3.1-pro-preview-customtools` | Gemini 3.1 Pro Preview Custom Tools | 1M | 66K | ✅ | ✅ |
| `gemini-flash-latest` | Gemini Flash Latest | 1M | 66K | ✅ | ✅ |
| `gemini-flash-lite-latest` | Gemini Flash-Lite Latest | 1M | 66K | ✅ | ✅ |
| `claude-opus-4-6@default` | Claude Opus 4.6 | 1M | 128K | ✅ | ✅ |
| `claude-opus-4-7@default` | Claude Opus 4.7 | 1M | 128K | ✅ | ✅ |
| `claude-opus-4-8@default` | Claude Opus 4.8 | 1M | 128K | ✅ | ✅ |
| `llama-4-maverick-17b-128e-instruct-maas` | Llama 4 Maverick 17B 128E Instruct | 524K | 8K | ✅ | — |
| `kimi-k2-thinking-maas` | Kimi K2 Thinking | 262K | 262K | — | ✅ |
| `qwen3-235b-a22b-instruct-2507-maas` | Qwen3 235B A22B Instruct | 262K | 16K | — | ✅ |
| `glm-5-maas` | GLM-5 | 203K | 131K | — | ✅ |
| `claude-3-5-haiku@20241022` | Claude Haiku 3.5 | 200K | 8K | ✅ | — |
| `claude-3-5-sonnet@20241022` | Claude Sonnet 3.5 v2 | 200K | 8K | ✅ | — |
| `claude-3-7-sonnet@20250219` | Claude Sonnet 3.7 | 200K | 64K | ✅ | ✅ |
| `claude-haiku-4-5@20251001` | Claude Haiku 4.5 | 200K | 64K | ✅ | ✅ |
| `claude-opus-4-1@20250805` | Claude Opus 4.1 | 200K | 32K | ✅ | ✅ |
| `claude-opus-4-5@20251101` | Claude Opus 4.5 | 200K | 64K | ✅ | ✅ |
| `claude-opus-4@20250514` | Claude Opus 4 | 200K | 32K | ✅ | ✅ |
| `claude-sonnet-4-5@20250929` | Claude Sonnet 4.5 | 200K | 64K | ✅ | ✅ |
| `claude-sonnet-4-6@default` | Claude Sonnet 4.6 | 200K | 64K | ✅ | ✅ |
| `claude-sonnet-4@20250514` | Claude Sonnet 4 | 200K | 64K | ✅ | ✅ |
| `glm-4.7-maas` | GLM-4.7 | 200K | 128K | — | ✅ |
| `deepseek-v3.1-maas` | DeepSeek V3.1 | 164K | 33K | — | ✅ |
| `deepseek-v3.2-maas` | DeepSeek V3.2 | 164K | 66K | — | ✅ |
| `gpt-oss-120b-maas` | GPT OSS 120B | 131K | 33K | — | ✅ |
| `gpt-oss-20b-maas` | GPT OSS 20B | 131K | 33K | — | ✅ |
| `llama-3.3-70b-instruct-maas` | Llama 3.3 70B Instruct | 128K | 8K | — | — |
| `gemini-2.5-flash-lite-preview-06-17` | Gemini 2.5 Flash Lite Preview 06-17 | 66K | 66K | ✅ | ✅ |

### groq

**Base URL:** `https://api.groq.com/openai/v1`

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 |
|---|---|---|---|---|---|
| `kimi-k2-instruct-0905` | Kimi K2 Instruct 0905 | 262K | 16K | — | — |
| `deepseek-r1-distill-llama-70b` | DeepSeek R1 Distill Llama 70B | 131K | 8K | — | ✅ |
| `compound` | Compound | 131K | 8K | — | ✅ |
| `compound-mini` | Compound Mini | 131K | 8K | — | ✅ |
| `llama-3.1-8b-instant` | Llama 3.1 8B Instant | 131K | 131K | — | — |
| `llama-3.3-70b-versatile` | Llama 3.3 70B Versatile | 131K | 33K | — | — |
| `llama-4-maverick-17b-128e-instruct` | Llama 4 Maverick 17B | 131K | 8K | ✅ | — |
| `llama-4-scout-17b-16e-instruct` | Llama 4 Scout 17B | 131K | 8K | ✅ | — |
| `kimi-k2-instruct` | Kimi K2 Instruct | 131K | 16K | — | — |
| `gpt-oss-120b` | GPT OSS 120B | 131K | 66K | — | ✅ |
| `gpt-oss-20b` | GPT OSS 20B | 131K | 66K | — | ✅ |
| `gpt-oss-safeguard-20b` | Safety GPT OSS 20B | 131K | 66K | — | ✅ |
| `qwen-qwq-32b` | Qwen QwQ 32B | 131K | 16K | — | ✅ |
| `qwen3-32b` | Qwen3 32B | 131K | 41K | — | ✅ |
| `mistral-saba-24b` | Mistral Saba 24B | 33K | 33K | — | — |
| `gemma2-9b-it` | Gemma 2 9B | 8K | 8K | — | — |
| `llama3-70b-8192` | Llama 3 70B | 8K | 8K | — | — |
| `llama3-8b-8192` | Llama 3 8B | 8K | 8K | — | — |

### huggingface

**Base URL:** `https://api-inference.huggingface.co/v1`

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 |
|---|---|---|---|---|---|
| `DeepSeek-V4-Pro` | DeepSeek V4 Pro | 1M | 393K | — | ✅ |
| `Qwen3-235B-A22B-Thinking-2507` | Qwen3-235B-A22B-Thinking-2507 | 262K | 131K | — | ✅ |
| `Qwen3-Coder-480B-A35B-Instruct` | Qwen3-Coder-480B-A35B-Instruct | 262K | 67K | — | — |
| `Qwen3-Coder-Next` | Qwen3-Coder-Next | 262K | 66K | — | — |
| `Qwen3-Next-80B-A3B-Instruct` | Qwen3-Next-80B-A3B-Instruct | 262K | 67K | — | — |
| `Qwen3-Next-80B-A3B-Thinking` | Qwen3-Next-80B-A3B-Thinking | 262K | 131K | — | — |
| `Qwen3.5-397B-A17B` | Qwen3.5-397B-A17B | 262K | 33K | ✅ | ✅ |
| `MiMo-V2-Flash` | MiMo-V2-Flash | 262K | 4K | — | ✅ |
| `Kimi-K2-Instruct-0905` | Kimi-K2-Instruct-0905 | 262K | 16K | — | — |
| `Kimi-K2-Thinking` | Kimi-K2-Thinking | 262K | 262K | — | ✅ |
| `Kimi-K2.5` | Kimi-K2.5 | 262K | 262K | ✅ | ✅ |
| `Kimi-K2.6` | Kimi-K2.6 | 262K | 262K | ✅ | ✅ |
| `MiniMax-M2.1` | MiniMax-M2.1 | 205K | 131K | — | ✅ |
| `MiniMax-M2.5` | MiniMax-M2.5 | 205K | 131K | — | ✅ |
| `MiniMax-M2.7` | MiniMax-M2.7 | 205K | 131K | — | ✅ |
| `GLM-4.7` | GLM-4.7 | 205K | 131K | — | ✅ |
| `GLM-5` | GLM-5 | 203K | 131K | — | ✅ |
| `GLM-5.1` | GLM-5.1 | 203K | 131K | — | ✅ |
| `GLM-4.7-Flash` | GLM-4.7-Flash | 200K | 128K | — | ✅ |
| `DeepSeek-R1-0528` | DeepSeek-R1-0528 | 164K | 164K | — | ✅ |
| `DeepSeek-V3.2` | DeepSeek-V3.2 | 164K | 66K | — | ✅ |
| `Kimi-K2-Instruct` | Kimi-K2-Instruct | 131K | 16K | — | — |

### kimi-coding

**Base URL:** `https://api.kimi.com/coding`

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 |
|---|---|---|---|---|---|
| `kimi-for-coding` | Kimi For Coding | 262K | 33K | — | ✅ |
| `kimi-k2-thinking` | Kimi K2 Thinking | 262K | 33K | — | ✅ |

### minimax

**Base URL:** `https://api.minimax.io/anthropic`

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 |
|---|---|---|---|---|---|
| `MiniMax-M2.1` | MiniMax-M2.1 | 205K | 131K | — | ✅ |
| `MiniMax-M2.5` | MiniMax-M2.5 | 205K | 131K | — | ✅ |
| `MiniMax-M2.5-highspeed` | MiniMax-M2.5-highspeed | 205K | 131K | — | ✅ |
| `MiniMax-M2.7` | MiniMax-M2.7 | 205K | 131K | — | ✅ |
| `MiniMax-M2.7-highspeed` | MiniMax-M2.7-highspeed | 205K | 131K | — | ✅ |
| `MiniMax-M2` | MiniMax-M2 | 197K | 128K | — | ✅ |

### minimax-cn

**Base URL:** `https://api.minimaxi.com/anthropic`

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 |
|---|---|---|---|---|---|
| `MiniMax-M2.1` | MiniMax-M2.1 | 205K | 131K | — | ✅ |
| `MiniMax-M2.5` | MiniMax-M2.5 | 205K | 131K | — | ✅ |
| `MiniMax-M2.5-highspeed` | MiniMax-M2.5-highspeed | 205K | 131K | — | ✅ |
| `MiniMax-M2.7` | MiniMax-M2.7 | 205K | 131K | — | ✅ |
| `MiniMax-M2.7-highspeed` | MiniMax-M2.7-highspeed | 205K | 131K | — | ✅ |
| `MiniMax-M2` | MiniMax-M2 | 197K | 128K | — | ✅ |

### Mistral

**Base URL:** `https://api.mistral.ai/v1`

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 |
|---|---|---|---|---|---|
| `devstral-2512` | Devstral 2 | 262K | 262K | — | — |
| `devstral-medium-latest` | Devstral 2 (latest) | 262K | 262K | — | — |
| `mistral-large-2512` | Mistral Large 3 | 262K | 262K | ✅ | — |
| `mistral-large-latest` | Mistral Large (latest) | 262K | 262K | ✅ | — |
| `mistral-medium-2508` | Mistral Medium 3.1 | 262K | 262K | ✅ | — |
| `mistral-medium-2604` | Mistral Medium 3.5 | 262K | 262K | ✅ | ✅ |
| `mistral-medium-latest` | Mistral Medium (latest) | 262K | 262K | ✅ | ✅ |
| `codestral-latest` | Codestral (latest) | 256K | 4K | — | — |
| `labs-devstral-small-2512` | Devstral Small 2 | 256K | 256K | ✅ | — |
| `mistral-small-2603` | Mistral Small 4 | 256K | 256K | ✅ | ✅ |
| `mistral-small-latest` | Mistral Small (latest) | 256K | 256K | ✅ | ✅ |
| `mistral-large-2411` | Mistral Large 2.1 | 131K | 16K | — | — |
| `mistral-medium-2505` | Mistral Medium 3 | 131K | 131K | ✅ | — |
| `devstral-medium-2507` | Devstral Medium | 128K | 128K | — | — |
| `devstral-small-2505` | Devstral Small 2505 | 128K | 128K | — | — |
| `devstral-small-2507` | Devstral Small | 128K | 128K | — | — |
| `magistral-medium-latest` | Magistral Medium (latest) | 128K | 16K | — | ✅ |
| `magistral-small` | Magistral Small | 128K | 128K | — | ✅ |
| `ministral-3b-latest` | Ministral 3B (latest) | 128K | 128K | — | — |
| `ministral-8b-latest` | Ministral 8B (latest) | 128K | 128K | — | — |
| `mistral-nemo` | Mistral Nemo | 128K | 128K | — | — |
| `mistral-small-2506` | Mistral Small 3.2 | 128K | 16K | ✅ | — |
| `pixtral-12b` | Pixtral 12B | 128K | 128K | ✅ | — |
| `pixtral-large-latest` | Pixtral Large (latest) | 128K | 128K | ✅ | — |
| `open-mixtral-8x22b` | Mixtral 8x22B | 64K | 64K | — | — |
| `open-mixtral-8x7b` | Mixtral 8x7B | 32K | 32K | — | — |
| `open-mistral-7b` | Mistral 7B | 8K | 8K | — | — |

### moonshotai

**Base URL:** `https://api.moonshot.ai/v1`

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 |
|---|---|---|---|---|---|
| `kimi-k2-0905-preview` | Kimi K2 0905 | 262K | 262K | — | — |
| `kimi-k2-thinking` | Kimi K2 Thinking | 262K | 262K | — | ✅ |
| `kimi-k2-thinking-turbo` | Kimi K2 Thinking Turbo | 262K | 262K | — | ✅ |
| `kimi-k2-turbo-preview` | Kimi K2 Turbo | 262K | 262K | — | — |
| `kimi-k2.5` | Kimi K2.5 | 262K | 262K | ✅ | ✅ |
| `kimi-k2.6` | Kimi K2.6 | 262K | 262K | ✅ | ✅ |
| `kimi-k2-0711-preview` | Kimi K2 0711 | 131K | 16K | — | — |

### moonshotai-cn

**Base URL:** `https://api.moonshot.cn/v1`

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 |
|---|---|---|---|---|---|
| `kimi-k2-0905-preview` | Kimi K2 0905 | 262K | 262K | — | — |
| `kimi-k2-thinking` | Kimi K2 Thinking | 262K | 262K | — | ✅ |
| `kimi-k2-thinking-turbo` | Kimi K2 Thinking Turbo | 262K | 262K | — | ✅ |
| `kimi-k2-turbo-preview` | Kimi K2 Turbo | 262K | 262K | — | — |
| `kimi-k2.5` | Kimi K2.5 | 262K | 262K | ✅ | ✅ |
| `kimi-k2.6` | Kimi K2.6 | 262K | 262K | ✅ | ✅ |
| `kimi-k2-0711-preview` | Kimi K2 0711 | 131K | 16K | — | — |

### OpenAI

**Base URL:** `https://api.openai.com/v1`

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 |
|---|---|---|---|---|---|
| `gpt-5.4-pro` | GPT-5.4 Pro | 1M | 128K | ✅ | ✅ |
| `gpt-5.5-pro` | GPT-5.5 Pro | 1M | 128K | ✅ | ✅ |
| `gpt-5.6-terra` | GPT-5.6 Terra | 1M | 128K | ✅ | ✅ |
| `gpt-4.1` | GPT-4.1 | 1M | 33K | ✅ | — |
| `gpt-4.1-mini` | GPT-4.1 mini | 1M | 33K | ✅ | — |
| `gpt-4.1-nano` | GPT-4.1 nano | 1M | 33K | ✅ | — |
| `gpt-5` | GPT-5 | 400K | 128K | ✅ | ✅ |
| `gpt-5-codex` | GPT-5-Codex | 400K | 128K | ✅ | ✅ |
| `gpt-5-mini` | GPT-5 Mini | 400K | 128K | ✅ | ✅ |
| `gpt-5-nano` | GPT-5 Nano | 400K | 128K | ✅ | ✅ |
| `gpt-5-pro` | GPT-5 Pro | 400K | 272K | ✅ | ✅ |
| `gpt-5.1` | GPT-5.1 | 400K | 128K | ✅ | ✅ |
| `gpt-5.1-codex` | GPT-5.1 Codex | 400K | 128K | ✅ | ✅ |
| `gpt-5.1-codex-max` | GPT-5.1 Codex Max | 400K | 128K | ✅ | ✅ |
| `gpt-5.1-codex-mini` | GPT-5.1 Codex mini | 400K | 128K | ✅ | ✅ |
| `gpt-5.2` | GPT-5.2 | 400K | 128K | ✅ | ✅ |
| `gpt-5.2-codex` | GPT-5.2 Codex | 400K | 128K | ✅ | ✅ |
| `gpt-5.2-pro` | GPT-5.2 Pro | 400K | 128K | ✅ | ✅ |
| `gpt-5.3-codex` | GPT-5.3 Codex | 400K | 128K | ✅ | ✅ |
| `gpt-5.4-mini` | GPT-5.4 mini | 400K | 128K | ✅ | ✅ |
| `gpt-5.4-nano` | GPT-5.4 nano | 400K | 128K | ✅ | ✅ |
| `gpt-5.4` | GPT-5.4 | 272K | 128K | ✅ | ✅ |
| `gpt-5.5` | GPT-5.5 | 272K | 128K | ✅ | ✅ |
| `gpt-5.6-luna` | GPT-5.6 Luna | 272K | 128K | ✅ | ✅ |
| `gpt-5.6-sol` | GPT-5.6 Sol | 272K | 128K | ✅ | ✅ |
| `o1` | o1 | 200K | 100K | ✅ | ✅ |
| `o1-pro` | o1-pro | 200K | 100K | ✅ | ✅ |
| `o3` | o3 | 200K | 100K | ✅ | ✅ |
| `o3-deep-research` | o3-deep-research | 200K | 100K | ✅ | ✅ |
| `o3-mini` | o3-mini | 200K | 100K | — | ✅ |
| `o3-pro` | o3-pro | 200K | 100K | ✅ | ✅ |
| `o4-mini` | o4-mini | 200K | 100K | ✅ | ✅ |
| `o4-mini-deep-research` | o4-mini-deep-research | 200K | 100K | ✅ | ✅ |
| `gpt-4-turbo` | GPT-4 Turbo | 128K | 4K | ✅ | — |
| `gpt-4o` | GPT-4o | 128K | 16K | ✅ | — |
| `gpt-4o-2024-05-13` | GPT-4o (2024-05-13) | 128K | 4K | ✅ | — |
| `gpt-4o-2024-08-06` | GPT-4o (2024-08-06) | 128K | 16K | ✅ | — |
| `gpt-4o-2024-11-20` | GPT-4o (2024-11-20) | 128K | 16K | ✅ | — |
| `gpt-4o-mini` | GPT-4o mini | 128K | 16K | ✅ | — |
| `gpt-5.1-chat-latest` | GPT-5.1 Chat | 128K | 16K | ✅ | ✅ |
| `gpt-5.2-chat-latest` | GPT-5.2 Chat | 128K | 16K | ✅ | ✅ |
| `gpt-5.3-chat-latest` | GPT-5.3 Chat (latest) | 128K | 16K | ✅ | — |
| `gpt-5.3-codex-spark` | GPT-5.3 Codex Spark | 128K | 32K | ✅ | ✅ |
| `gpt-4` | GPT-4 | 8K | 8K | — | — |

### openai-codex

**Base URL:** `https://api.openai.com/v1`

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 |
|---|---|---|---|---|---|
| `gpt-5.5` |  | 272K | 128K | ✅ | ✅ |

### opencode

**Base URL:** `https://opencode.ai/zen`

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 |
|---|---|---|---|---|---|
| `deepseek-v4-pro` |  | 1M | 66K | ✅ | ✅ |
| `kimi-k2.6` |  | 128K | 66K | — | — |

### opencode-go

**Base URL:** `https://opencode.ai/zen/go`

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 |
|---|---|---|---|---|---|
| `deepseek-v4-pro` |  | 1M | 66K | ✅ | ✅ |
| `kimi-k2.6` |  | 128K | 66K | — | — |

### OpenRouter

**Base URL:** `https://openrouter.ai/api/v1`

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 |
|---|---|---|---|---|---|
| `auto` | Auto Router | 2M | 4K | ✅ | ✅ |
| `grok-4-fast` | xAI: Grok 4 Fast | 2M | 30K | ✅ | ✅ |
| `grok-4.1-fast` | xAI: Grok 4.1 Fast | 2M | 30K | ✅ | ✅ |
| `grok-4.20` | xAI: Grok 4.20 | 2M | 4K | ✅ | ✅ |
| `gpt-5.4` | OpenAI: GPT-5.4 | 1M | 128K | ✅ | ✅ |
| `gpt-5.4-pro` | OpenAI: GPT-5.4 Pro | 1M | 128K | ✅ | ✅ |
| `gpt-5.5` | OpenAI: GPT-5.5 | 1M | 128K | ✅ | ✅ |
| `gpt-5.5-pro` | OpenAI: GPT-5.5 Pro | 1M | 128K | ✅ | ✅ |
| `gpt-latest` | OpenAI GPT Latest | 1M | 128K | ✅ | ✅ |
| `owl-alpha` | Owl Alpha | 1M | 262K | — | — |
| `deepseek-v4-flash` | DeepSeek: DeepSeek V4 Flash | 1M | 384K | — | ✅ |
| `deepseek-v4-pro` | DeepSeek: DeepSeek V4 Pro | 1M | 384K | — | ✅ |
| `gemini-2.0-flash-001` | Google: Gemini 2.0 Flash | 1M | 8K | ✅ | — |
| `gemini-2.0-flash-lite-001` | Google: Gemini 2.0 Flash Lite | 1M | 8K | ✅ | — |
| `gemini-2.5-flash` | Google: Gemini 2.5 Flash | 1M | 66K | ✅ | ✅ |
| `gemini-2.5-flash-lite` | Google: Gemini 2.5 Flash Lite | 1M | 66K | ✅ | ✅ |
| `gemini-2.5-flash-lite-preview-09-2025` | Google: Gemini 2.5 Flash Lite Preview 09-2025 | 1M | 66K | ✅ | ✅ |
| `gemini-2.5-pro` | Google: Gemini 2.5 Pro | 1M | 66K | ✅ | ✅ |
| `gemini-2.5-pro-preview` | Google: Gemini 2.5 Pro Preview 06-05 | 1M | 66K | ✅ | ✅ |
| `gemini-2.5-pro-preview-05-06` | Google: Gemini 2.5 Pro Preview 05-06 | 1M | 66K | ✅ | ✅ |
| `gemini-3-flash-preview` | Google: Gemini 3 Flash Preview | 1M | 66K | ✅ | ✅ |
| `gemini-3.1-flash-lite` | Google: Gemini 3.1 Flash Lite | 1M | 66K | ✅ | ✅ |
| `gemini-3.1-flash-lite-preview` | Google: Gemini 3.1 Flash Lite Preview | 1M | 66K | ✅ | ✅ |
| `gemini-3.1-pro-preview` | Google: Gemini 3.1 Pro Preview | 1M | 66K | ✅ | ✅ |
| `gemini-3.1-pro-preview-customtools` | Google: Gemini 3.1 Pro Preview Custom Tools | 1M | 66K | ✅ | ✅ |
| `mimo-v2-pro` | Xiaomi: MiMo-V2-Pro | 1M | 131K | — | ✅ |
| `mimo-v2.5` | Xiaomi: MiMo-V2.5 | 1M | 131K | ✅ | ✅ |
| `mimo-v2.5-pro` | Xiaomi: MiMo-V2.5-Pro | 1M | 16K | — | ✅ |
| `gemini-flash-latest` | Google Gemini Flash Latest | 1M | 66K | ✅ | ✅ |
| `gemini-pro-latest` | Google Gemini Pro Latest | 1M | 66K | ✅ | ✅ |
| `gpt-4.1` | OpenAI: GPT-4.1 | 1M | 4K | ✅ | — |
| `gpt-4.1-mini` | OpenAI: GPT-4.1 Mini | 1M | 33K | ✅ | — |
| `gpt-4.1-nano` | OpenAI: GPT-4.1 Nano | 1M | 33K | ✅ | — |
| `nova-2-lite-v1` | Amazon: Nova 2 Lite | 1M | 66K | ✅ | ✅ |
| `nova-premier-v1` | Amazon: Nova Premier 1.0 | 1M | 32K | ✅ | — |
| `claude-opus-4.6` | Anthropic: Claude Opus 4.6 | 1M | 128K | ✅ | ✅ |
| `claude-opus-4.6-fast` | Anthropic: Claude Opus 4.6 (Fast) | 1M | 128K | ✅ | ✅ |
| `claude-opus-4.7` | Anthropic: Claude Opus 4.7 | 1M | 128K | ✅ | ✅ |
| `claude-sonnet-4` | Anthropic: Claude Sonnet 4 | 1M | 64K | ✅ | ✅ |
| `claude-sonnet-4.5` | Anthropic: Claude Sonnet 4.5 | 1M | 64K | ✅ | ✅ |
| `claude-sonnet-4.6` | Anthropic: Claude Sonnet 4.6 | 1M | 128K | ✅ | ✅ |
| `minimax-m1` | MiniMax: MiniMax M1 | 1M | 40K | — | ✅ |
| `qwen-plus` | Qwen: Qwen-Plus | 1M | 33K | — | — |
| `qwen-plus-2025-07-28` | Qwen: Qwen Plus 0728 | 1M | 33K | — | — |
| `qwen-plus-2025-07-28:thinking` | Qwen: Qwen Plus 0728 (thinking) | 1M | 33K | — | ✅ |
| `qwen3-coder-flash` | Qwen: Qwen3 Coder Flash | 1M | 66K | — | — |
| `qwen3-coder-plus` | Qwen: Qwen3 Coder Plus | 1M | 66K | — | — |
| `qwen3.5-flash-02-23` | Qwen: Qwen3.5-Flash | 1M | 66K | ✅ | ✅ |
| `qwen3.5-plus-02-15` | Qwen: Qwen3.5 Plus 2026-02-15 | 1M | 66K | ✅ | ✅ |
| `qwen3.5-plus-20260420` | Qwen: Qwen3.5 Plus 2026-04-20 | 1M | 66K | ✅ | ✅ |
| `qwen3.6-flash` | Qwen: Qwen3.6 Flash | 1M | 66K | ✅ | ✅ |
| `qwen3.6-plus` | Qwen: Qwen3.6 Plus | 1M | 66K | ✅ | ✅ |
| `grok-4.3` | xAI: Grok 4.3 | 1M | 4K | ✅ | ✅ |
| `claude-opus-latest` | Anthropic: Claude Opus Latest | 1M | 128K | ✅ | ✅ |
| `claude-sonnet-latest` | Anthropic Claude Sonnet Latest | 1M | 128K | ✅ | ✅ |
| `gpt-5` | OpenAI: GPT-5 | 400K | 128K | ✅ | ✅ |
| `gpt-5-codex` | OpenAI: GPT-5 Codex | 400K | 128K | ✅ | ✅ |
| `gpt-5-mini` | OpenAI: GPT-5 Mini | 400K | 128K | ✅ | ✅ |
| `gpt-5-nano` | OpenAI: GPT-5 Nano | 400K | 4K | ✅ | ✅ |
| `gpt-5-pro` | OpenAI: GPT-5 Pro | 400K | 128K | ✅ | ✅ |
| `gpt-5.1` | OpenAI: GPT-5.1 | 400K | 128K | ✅ | ✅ |
| `gpt-5.1-codex` | OpenAI: GPT-5.1-Codex | 400K | 128K | ✅ | ✅ |
| `gpt-5.1-codex-max` | OpenAI: GPT-5.1-Codex-Max | 400K | 128K | ✅ | ✅ |
| `gpt-5.1-codex-mini` | OpenAI: GPT-5.1-Codex-Mini | 400K | 128K | ✅ | ✅ |
| `gpt-5.2` | OpenAI: GPT-5.2 | 400K | 128K | ✅ | ✅ |
| `gpt-5.2-codex` | OpenAI: GPT-5.2-Codex | 400K | 128K | ✅ | ✅ |
| `gpt-5.2-pro` | OpenAI: GPT-5.2 Pro | 400K | 128K | ✅ | ✅ |
| `gpt-5.3-codex` | OpenAI: GPT-5.3-Codex | 400K | 128K | ✅ | ✅ |
| `gpt-5.4-mini` | OpenAI: GPT-5.4 Mini | 400K | 128K | ✅ | ✅ |
| `gpt-5.4-nano` | OpenAI: GPT-5.4 Nano | 400K | 128K | ✅ | ✅ |
| `gpt-chat-latest` | OpenAI: GPT Chat Latest | 400K | 128K | ✅ | — |
| `gpt-mini-latest` | OpenAI GPT Mini Latest | 400K | 128K | ✅ | ✅ |
| `llama-4-scout` | Meta: Llama 4 Scout | 328K | 16K | ✅ | — |
| `nova-lite-v1` | Amazon: Nova Lite 1.0 | 300K | 5K | ✅ | — |
| `nova-pro-v1` | Amazon: Nova Pro 1.0 | 300K | 5K | ✅ | — |
| `trinity-large-thinking` | Arcee AI: Trinity Large Thinking | 262K | 262K | — | ✅ |
| `trinity-large-thinking:free` | Arcee AI: Trinity Large Thinking (free) | 262K | 80K | — | ✅ |
| `seed-1.6` | ByteDance Seed: Seed 1.6 | 262K | 33K | ✅ | ✅ |
| `seed-1.6-flash` | ByteDance Seed: Seed 1.6 Flash | 262K | 33K | ✅ | ✅ |
| `seed-2.0-lite` | ByteDance Seed: Seed-2.0-Lite | 262K | 131K | ✅ | ✅ |
| `seed-2.0-mini` | ByteDance Seed: Seed-2.0-Mini | 262K | 131K | ✅ | ✅ |
| `gemma-4-26b-a4b-it` | Google: Gemma 4 26B A4B  | 262K | 4K | ✅ | ✅ |
| `gemma-4-26b-a4b-it:free` | Google: Gemma 4 26B A4B  (free) | 262K | 33K | ✅ | ✅ |
| `gemma-4-31b-it` | Google: Gemma 4 31B | 262K | 16K | ✅ | ✅ |
| `gemma-4-31b-it:free` | Google: Gemma 4 31B (free) | 262K | 33K | ✅ | ✅ |
| `ling-2.6-1t` | inclusionAI: Ling-2.6-1T | 262K | 33K | — | — |
| `ling-2.6-flash` | inclusionAI: Ling-2.6-flash | 262K | 33K | — | — |
| `ring-2.6-1t:free` | inclusionAI: Ring-2.6-1T (free) | 262K | 66K | — | ✅ |
| `devstral-2512` | Mistral: Devstral 2 2512 | 262K | 4K | — | — |
| `ministral-14b-2512` | Mistral: Ministral 3 14B 2512 | 262K | 4K | ✅ | — |
| `ministral-8b-2512` | Mistral: Ministral 3 8B 2512 | 262K | 4K | ✅ | — |
| `mistral-large-2512` | Mistral: Mistral Large 3 2512 | 262K | 4K | ✅ | — |
| `mistral-medium-3-5` | Mistral: Mistral Medium 3.5 | 262K | 4K | ✅ | ✅ |
| `mistral-small-2603` | Mistral: Mistral Small 4 | 262K | 4K | ✅ | ✅ |
| `kimi-k2-0905` | MoonshotAI: Kimi K2 0905 | 262K | 262K | — | — |
| `kimi-k2-thinking` | MoonshotAI: Kimi K2 Thinking | 262K | 262K | — | ✅ |
| `kimi-k2.5` | MoonshotAI: Kimi K2.5 | 262K | 4K | ✅ | ✅ |
| `kimi-k2.6` | MoonshotAI: Kimi K2.6 | 262K | 66K | ✅ | ✅ |
| `nemotron-3-nano-30b-a3b` | NVIDIA: Nemotron 3 Nano 30B A3B | 262K | 228K | — | ✅ |
| `nemotron-3-super-120b-a12b` | NVIDIA: Nemotron 3 Super | 262K | 4K | — | ✅ |
| `nemotron-3-super-120b-a12b:free` | NVIDIA: Nemotron 3 Super (free) | 262K | 262K | — | ✅ |
| `qwen3-235b-a22b-2507` | Qwen: Qwen3 235B A22B Instruct 2507 | 262K | 16K | — | — |
| `qwen3-30b-a3b-instruct-2507` | Qwen: Qwen3 30B A3B Instruct 2507 | 262K | 262K | — | — |
| `qwen3-coder` | Qwen: Qwen3 Coder 480B A35B | 262K | 66K | — | — |
| `qwen3-coder-next` | Qwen: Qwen3 Coder Next | 262K | 262K | — | — |
| `qwen3-max` | Qwen: Qwen3 Max | 262K | 33K | — | — |
| `qwen3-max-thinking` | Qwen: Qwen3 Max Thinking | 262K | 33K | — | ✅ |
| `qwen3-next-80b-a3b-instruct` | Qwen: Qwen3 Next 80B A3B Instruct | 262K | 16K | — | — |
| `qwen3-next-80b-a3b-instruct:free` | Qwen: Qwen3 Next 80B A3B Instruct (free) | 262K | 4K | — | — |
| `qwen3-vl-235b-a22b-instruct` | Qwen: Qwen3 VL 235B A22B Instruct | 262K | 16K | ✅ | — |
| `qwen3.5-122b-a10b` | Qwen: Qwen3.5-122B-A10B | 262K | 66K | ✅ | ✅ |
| `qwen3.5-27b` | Qwen: Qwen3.5-27B | 262K | 66K | ✅ | ✅ |
| `qwen3.5-35b-a3b` | Qwen: Qwen3.5-35B-A3B | 262K | 82K | ✅ | ✅ |
| `qwen3.5-397b-a17b` | Qwen: Qwen3.5 397B A17B | 262K | 66K | ✅ | ✅ |
| `qwen3.5-9b` | Qwen: Qwen3.5-9B | 262K | 82K | ✅ | ✅ |
| `qwen3.6-27b` | Qwen: Qwen3.6 27B | 262K | 82K | ✅ | ✅ |
| `qwen3.6-35b-a3b` | Qwen: Qwen3.6 35B A3B | 262K | 262K | ✅ | ✅ |
| `qwen3.6-max-preview` | Qwen: Qwen3.6 Max Preview | 262K | 66K | — | ✅ |
| `step-3.5-flash` | StepFun: Step 3.5 Flash | 262K | 66K | — | ✅ |
| `hy3-preview` | Tencent: Hy3 preview | 262K | 262K | — | ✅ |
| `mimo-v2-flash` | Xiaomi: MiMo-V2-Flash | 262K | 66K | — | ✅ |
| `mimo-v2-omni` | Xiaomi: MiMo-V2-Omni | 262K | 66K | ✅ | ✅ |
| `kimi-latest` | MoonshotAI Kimi Latest | 262K | 66K | ✅ | ✅ |
| `qwen3-coder:free` | Qwen: Qwen3 Coder 480B A35B (free) | 262K | 262K | — | — |
| `jamba-large-1.7` | AI21: Jamba Large 1.7 | 256K | 4K | — | — |
| `kat-coder-pro-v2` | Kwaipilot: KAT-Coder-Pro V2 | 256K | 80K | — | — |
| `codestral-2508` | Mistral: Codestral 2508 | 256K | 4K | — | — |
| `nemotron-3-nano-30b-a3b:free` | NVIDIA: Nemotron 3 Nano 30B A3B (free) | 256K | 4K | — | ✅ |
| `nemotron-3-nano-omni-30b-a3b-reasoning:free` | NVIDIA: Nemotron 3 Nano Omni (free) | 256K | 66K | ✅ | — |
| `relace-search` | Relace: Relace Search | 256K | 128K | — | — |
| `grok-4` | xAI: Grok 4 | 256K | 4K | ✅ | ✅ |
| `grok-code-fast-1` | xAI: Grok Code Fast 1 | 256K | 10K | — | ✅ |
| `glm-4.6` | Z.ai: GLM 4.6 | 205K | 205K | — | ✅ |
| `glm-4.7` | Z.ai: GLM 4.7 | 203K | 131K | — | ✅ |
| `glm-4.7-flash` | Z.ai: GLM 4.7 Flash | 203K | 16K | — | ✅ |
| `glm-5` | Z.ai: GLM 5 | 203K | 4K | — | ✅ |
| `glm-5-turbo` | Z.ai: GLM 5 Turbo | 203K | 131K | — | ✅ |
| `glm-5.1` | Z.ai: GLM 5.1 | 203K | 4K | — | ✅ |
| `glm-5v-turbo` | Z.ai: GLM 5V Turbo | 203K | 131K | ✅ | ✅ |
| `claude-3-haiku` | Anthropic: Claude 3 Haiku | 200K | 4K | ✅ | — |
| `claude-3.5-haiku` | Anthropic: Claude 3.5 Haiku | 200K | 8K | ✅ | — |
| `claude-haiku-4.5` | Anthropic: Claude Haiku 4.5 | 200K | 64K | ✅ | ✅ |
| `claude-opus-4` | Anthropic: Claude Opus 4 | 200K | 32K | ✅ | ✅ |
| `claude-opus-4.1` | Anthropic: Claude Opus 4.1 | 200K | 32K | ✅ | ✅ |
| `claude-opus-4.5` | Anthropic: Claude Opus 4.5 | 200K | 64K | ✅ | ✅ |
| `o1` | OpenAI: o1 | 200K | 100K | ✅ | ✅ |
| `o3` | OpenAI: o3 | 200K | 100K | ✅ | ✅ |
| `o3-deep-research` | OpenAI: o3 Deep Research | 200K | 100K | ✅ | ✅ |
| `o3-mini` | OpenAI: o3 Mini | 200K | 100K | — | ✅ |
| `o3-mini-high` | OpenAI: o3 Mini High | 200K | 100K | — | ✅ |
| `o3-pro` | OpenAI: o3 Pro | 200K | 100K | ✅ | ✅ |
| `o4-mini` | OpenAI: o4 Mini | 200K | 100K | ✅ | ✅ |
| `o4-mini-deep-research` | OpenAI: o4 Mini Deep Research | 200K | 100K | ✅ | ✅ |
| `o4-mini-high` | OpenAI: o4 Mini High | 200K | 100K | ✅ | ✅ |
| `free` | Free Models Router | 200K | 4K | ✅ | ✅ |
| `claude-haiku-latest` | Anthropic Claude Haiku Latest | 200K | 64K | ✅ | ✅ |
| `minimax-m2` | MiniMax: MiniMax M2 | 197K | 197K | — | ✅ |
| `minimax-m2.1` | MiniMax: MiniMax M2.1 | 197K | 197K | — | ✅ |
| `minimax-m2.5` | MiniMax: MiniMax M2.5 | 197K | 197K | — | ✅ |
| `minimax-m2.5:free` | MiniMax: MiniMax M2.5 (free) | 197K | 8K | — | ✅ |
| `minimax-m2.7` | MiniMax: MiniMax M2.7 | 197K | 4K | — | ✅ |
| `deepseek-chat` | DeepSeek: DeepSeek V3 | 164K | 16K | — | — |
| `deepseek-chat-v3-0324` | DeepSeek: DeepSeek V3 0324 | 164K | 16K | — | — |
| `deepseek-chat-v3.1` | DeepSeek: DeepSeek V3.1 | 164K | 33K | — | ✅ |
| `deepseek-r1-0528` | DeepSeek: R1 0528 | 164K | 33K | — | ✅ |
| `deepseek-v3.1-terminus` | DeepSeek: DeepSeek V3.1 Terminus | 164K | 33K | — | ✅ |
| `deepseek-v3.2-exp` | DeepSeek: DeepSeek V3.2 Exp | 164K | 66K | — | ✅ |
| `qwen3-coder-30b-a3b-instruct` | Qwen: Qwen3 Coder 30B A3B Instruct | 160K | 33K | — | — |
| `tongyi-deepresearch-30b-a3b` | Tongyi DeepResearch 30B A3B | 131K | 131K | — | ✅ |
| `trinity-mini` | Arcee AI: Trinity Mini | 131K | 131K | — | ✅ |
| `virtuoso-large` | Arcee AI: Virtuoso Large | 131K | 64K | — | — |
| `cobuddy:free` | Baidu Qianfan: CoBuddy (free) | 131K | 66K | — | ✅ |
| `deepseek-v3.2` | DeepSeek: DeepSeek V3.2 | 131K | 66K | — | ✅ |
| `gemma-3-12b-it` | Google: Gemma 3 12B | 131K | 16K | ✅ | — |
| `gemma-3-27b-it` | Google: Gemma 3 27B | 131K | 16K | ✅ | — |
| `granite-4.1-8b` | IBM: Granite 4.1 8B | 131K | 131K | — | — |
| `llama-3.1-70b-instruct` | Meta: Llama 3.1 70B Instruct | 131K | 16K | — | — |
| `llama-3.3-70b-instruct` | Meta: Llama 3.3 70B Instruct | 131K | 16K | — | — |
| `devstral-medium` | Mistral: Devstral Medium | 131K | 4K | — | — |
| `devstral-small` | Mistral: Devstral Small 1.1 | 131K | 4K | — | — |
| `ministral-3b-2512` | Mistral: Ministral 3 3B 2512 | 131K | 4K | ✅ | — |
| `mistral-large-2407` | Mistral Large 2407 | 131K | 4K | — | — |
| `mistral-large-2411` | Mistral Large 2411 | 131K | 4K | — | — |
| `mistral-medium-3` | Mistral: Mistral Medium 3 | 131K | 4K | ✅ | — |
| `mistral-medium-3.1` | Mistral: Mistral Medium 3.1 | 131K | 4K | ✅ | — |
| `mistral-nemo` | Mistral: Mistral Nemo | 131K | 4K | — | — |
| `pixtral-large-2411` | Mistral: Pixtral Large 2411 | 131K | 4K | ✅ | — |
| `kimi-k2` | MoonshotAI: Kimi K2 0711 | 131K | 33K | — | — |
| `deepseek-v3.1-nex-n1` | Nex AGI: DeepSeek V3.1 Nex N1 | 131K | 164K | — | — |
| `llama-3.3-nemotron-super-49b-v1.5` | NVIDIA: Llama 3.3 Nemotron Super 49B V1.5 | 131K | 16K | — | ✅ |
| `nemotron-nano-9b-v2` | NVIDIA: Nemotron Nano 9B V2 | 131K | 16K | — | ✅ |
| `gpt-oss-120b` | OpenAI: gpt-oss-120b | 131K | 4K | — | ✅ |
| `gpt-oss-120b:free` | OpenAI: gpt-oss-120b (free) | 131K | 131K | — | ✅ |
| `gpt-oss-20b` | OpenAI: gpt-oss-20b | 131K | 131K | — | ✅ |
| `gpt-oss-20b:free` | OpenAI: gpt-oss-20b (free) | 131K | 8K | — | ✅ |
| `gpt-oss-safeguard-20b` | OpenAI: gpt-oss-safeguard-20b | 131K | 66K | — | ✅ |
| `laguna-m.1:free` | Poolside: Laguna M.1 (free) | 131K | 8K | — | ✅ |
| `laguna-xs.2:free` | Poolside: Laguna XS.2 (free) | 131K | 8K | — | ✅ |
| `intellect-3` | Prime Intellect: INTELLECT-3 | 131K | 131K | — | ✅ |
| `qwen-turbo` | Qwen: Qwen-Turbo | 131K | 8K | — | — |
| `qwen-vl-max` | Qwen: Qwen VL Max | 131K | 33K | ✅ | — |
| `qwen3-235b-a22b` | Qwen: Qwen3 235B A22B | 131K | 8K | — | ✅ |
| `qwen3-235b-a22b-thinking-2507` | Qwen: Qwen3 235B A22B Thinking 2507 | 131K | 4K | — | ✅ |
| `qwen3-30b-a3b-thinking-2507` | Qwen: Qwen3 30B A3B Thinking 2507 | 131K | 131K | — | ✅ |
| `qwen3-next-80b-a3b-thinking` | Qwen: Qwen3 Next 80B A3B Thinking | 131K | 33K | — | ✅ |
| `qwen3-vl-235b-a22b-thinking` | Qwen: Qwen3 VL 235B A22B Thinking | 131K | 33K | ✅ | ✅ |
| `qwen3-vl-30b-a3b-instruct` | Qwen: Qwen3 VL 30B A3B Instruct | 131K | 33K | ✅ | — |
| `qwen3-vl-30b-a3b-thinking` | Qwen: Qwen3 VL 30B A3B Thinking | 131K | 33K | ✅ | ✅ |
| `qwen3-vl-32b-instruct` | Qwen: Qwen3 VL 32B Instruct | 131K | 33K | ✅ | — |
| `qwen3-vl-8b-instruct` | Qwen: Qwen3 VL 8B Instruct | 131K | 33K | ✅ | — |
| `qwen3-vl-8b-thinking` | Qwen: Qwen3 VL 8B Thinking | 131K | 33K | ✅ | ✅ |
| `l3.1-euryale-70b` | Sao10K: Llama 3.1 Euryale 70B v2.2 | 131K | 16K | — | — |
| `grok-3` | xAI: Grok 3 | 131K | 4K | — | — |
| `grok-3-beta` | xAI: Grok 3 Beta | 131K | 4K | — | — |
| `grok-3-mini` | xAI: Grok 3 Mini | 131K | 4K | — | ✅ |
| `grok-3-mini-beta` | xAI: Grok 3 Mini Beta | 131K | 4K | — | ✅ |
| `glm-4.5` | Z.ai: GLM 4.5 | 131K | 98K | — | ✅ |
| `glm-4.5-air` | Z.ai: GLM 4.5 Air | 131K | 98K | — | ✅ |
| `glm-4.5-air:free` | Z.ai: GLM 4.5 Air (free) | 131K | 96K | — | ✅ |
| `glm-4.6v` | Z.ai: GLM 4.6V | 131K | 24K | ✅ | ✅ |
| `trinity-large-preview` | Arcee AI: Trinity Large Preview | 131K | 4K | — | — |
| `nova-micro-v1` | Amazon: Nova Micro 1.0 | 128K | 5K | — | — |
| `command-r-08-2024` | Cohere: Command R (08-2024) | 128K | 4K | — | — |
| `command-r-plus-08-2024` | Cohere: Command R+ (08-2024) | 128K | 4K | — | — |
| `mercury-2` | Inception: Mercury 2 | 128K | 50K | — | ✅ |
| `mistral-large` | Mistral Large | 128K | 4K | — | — |
| `mistral-small-3.2-24b-instruct` | Mistral: Mistral Small 3.2 24B | 128K | 16K | ✅ | — |
| `nemotron-nano-12b-v2-vl:free` | NVIDIA: Nemotron Nano 12B 2 VL (free) | 128K | 128K | ✅ | ✅ |
| `nemotron-nano-9b-v2:free` | NVIDIA: Nemotron Nano 9B V2 (free) | 128K | 4K | — | ✅ |
| `gpt-4-1106-preview` | OpenAI: GPT-4 Turbo (older v1106) | 128K | 4K | — | — |
| `gpt-4-turbo` | OpenAI: GPT-4 Turbo | 128K | 4K | ✅ | — |
| `gpt-4-turbo-preview` | OpenAI: GPT-4 Turbo Preview | 128K | 4K | — | — |
| `gpt-4o` | OpenAI: GPT-4o | 128K | 16K | ✅ | — |
| `gpt-4o-2024-05-13` | OpenAI: GPT-4o (2024-05-13) | 128K | 4K | ✅ | — |
| `gpt-4o-2024-08-06` | OpenAI: GPT-4o (2024-08-06) | 128K | 16K | ✅ | — |
| `gpt-4o-2024-11-20` | OpenAI: GPT-4o (2024-11-20) | 128K | 16K | ✅ | — |
| `gpt-4o-audio-preview` | OpenAI: GPT-4o Audio | 128K | 16K | — | — |
| `gpt-4o-mini` | OpenAI: GPT-4o-mini | 128K | 16K | ✅ | — |
| `gpt-4o-mini-2024-07-18` | OpenAI: GPT-4o-mini (2024-07-18) | 128K | 16K | ✅ | — |
| `gpt-5.1-chat` | OpenAI: GPT-5.1 Chat | 128K | 16K | ✅ | — |
| `gpt-5.2-chat` | OpenAI: GPT-5.2 Chat | 128K | 32K | ✅ | — |
| `gpt-5.3-chat` | OpenAI: GPT-5.3 Chat | 128K | 16K | ✅ | — |
| `gpt-audio` | OpenAI: GPT Audio | 128K | 16K | — | — |
| `gpt-audio-mini` | OpenAI: GPT Audio Mini | 128K | 16K | — | — |
| `solar-pro-3` | Upstage: Solar Pro 3 | 128K | 4K | — | ✅ |
| `glm-4-32b` | Z.ai: GLM 4 32B  | 128K | 4K | — | — |
| `ernie-4.5-21b-a3b` | Baidu: ERNIE 4.5 21B A3B | 120K | 8K | — | — |
| `llama-3.3-70b-instruct:free` | Meta: Llama 3.3 70B Instruct (free) | 66K | 4K | — | — |
| `mixtral-8x22b-instruct` | Mistral: Mixtral 8x22B Instruct | 66K | 4K | — | — |
| `glm-4.5v` | Z.ai: GLM 4.5V | 66K | 16K | ✅ | ✅ |
| `deepseek-r1` | DeepSeek: R1 | 64K | 16K | — | ✅ |
| `qwen3-14b` | Qwen: Qwen3 14B | 41K | 41K | — | ✅ |
| `qwen3-30b-a3b` | Qwen: Qwen3 30B A3B | 41K | 20K | — | ✅ |
| `qwen3-32b` | Qwen: Qwen3 32B | 41K | 16K | — | ✅ |
| `qwen3-8b` | Qwen: Qwen3 8B | 41K | 8K | — | ✅ |
| `rnj-1-instruct` | EssentialAI: Rnj 1 Instruct | 33K | 4K | — | — |
| `mistral-saba` | Mistral: Saba | 33K | 4K | — | — |
| `qwen-2.5-72b-instruct` | Qwen2.5 72B Instruct | 33K | 16K | — | — |
| `qwen-2.5-7b-instruct` | Qwen: Qwen2.5 7B Instruct | 33K | 33K | — | — |
| `qwen-max` | Qwen: Qwen-Max  | 33K | 8K | — | — |
| `rocinante-12b` | TheDrummer: Rocinante 12B | 33K | 33K | — | — |
| `unslopnemo-12b` | TheDrummer: UnslopNemo 12B | 33K | 33K | — | — |
| `voxtral-small-24b-2507` | Mistral: Voxtral Small 24B 2507 | 32K | 4K | — | — |
| `ernie-4.5-vl-28b-a3b` | Baidu: ERNIE 4.5 VL 28B A3B | 30K | 8K | ✅ | ✅ |
| `gpt-3.5-turbo` | OpenAI: GPT-3.5 Turbo | 16K | 4K | — | — |
| `gpt-3.5-turbo-16k` | OpenAI: GPT-3.5 Turbo 16k | 16K | 4K | — | — |
| `llama-3.1-8b-instruct` | Meta: Llama 3.1 8B Instruct | 16K | 16K | — | — |
| `reka-edge` | Reka Edge | 16K | 16K | ✅ | — |
| `l3-euryale-70b` | Sao10k: Llama 3 Euryale 70B v2.1 | 8K | 8K | — | — |
| `gpt-4` | OpenAI: GPT-4 | 8K | 4K | — | — |
| `gpt-4-0314` | OpenAI: GPT-4 (older v0314) | 8K | 4K | — | — |
| `gpt-3.5-turbo-0613` | OpenAI: GPT-3.5 Turbo (older v0613) | 4K | 4K | — | — |

### vercel-ai-gateway

**Base URL:** `https://ai-gateway.vercel.sh`

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 |
|---|---|---|---|---|---|
| `grok-4-fast-non-reasoning` | Grok 4 Fast Non-Reasoning | 2M | 256K | ✅ | — |
| `grok-4-fast-reasoning` | Grok 4 Fast Reasoning | 2M | 256K | ✅ | ✅ |
| `grok-4.1-fast-non-reasoning` | Grok 4.1 Fast Non-Reasoning | 2M | 30K | ✅ | — |
| `grok-4.1-fast-reasoning` | Grok 4.1 Fast Reasoning | 2M | 30K | ✅ | ✅ |
| `grok-4.20-multi-agent` | Grok 4.20 Multi-Agent | 2M | 2M | ✅ | ✅ |
| `grok-4.20-multi-agent-beta` | Grok 4.20 Multi Agent Beta | 2M | 2M | ✅ | ✅ |
| `grok-4.20-non-reasoning` | Grok 4.20 Non-Reasoning | 2M | 2M | ✅ | — |
| `grok-4.20-non-reasoning-beta` | Grok 4.20 Beta Non-Reasoning | 2M | 2M | ✅ | — |
| `grok-4.20-reasoning` | Grok 4.20 Reasoning | 2M | 2M | ✅ | ✅ |
| `grok-4.20-reasoning-beta` | Grok 4.20 Beta Reasoning | 2M | 2M | ✅ | ✅ |
| `gpt-5.4` | GPT 5.4 | 1M | 128K | ✅ | ✅ |
| `gpt-5.4-pro` | GPT 5.4 Pro | 1M | 128K | ✅ | ✅ |
| `mimo-v2.5` | MiMo M2.5 | 1M | 131K | ✅ | ✅ |
| `mimo-v2.5-pro` | MiMo V2.5 Pro | 1M | 131K | ✅ | ✅ |
| `gemini-2.0-flash` | Gemini 2.0 Flash | 1M | 8K | ✅ | — |
| `gemini-2.0-flash-lite` | Gemini 2.0 Flash Lite | 1M | 8K | ✅ | — |
| `gemini-2.5-flash-lite` | Gemini 2.5 Flash Lite | 1M | 66K | ✅ | ✅ |
| `gemini-2.5-pro` | Gemini 2.5 Pro | 1M | 66K | ✅ | ✅ |
| `gpt-4.1` | GPT-4.1 | 1M | 33K | ✅ | — |
| `gpt-4.1-mini` | GPT-4.1 mini | 1M | 33K | ✅ | — |
| `gpt-4.1-nano` | GPT-4.1 nano | 1M | 33K | ✅ | — |
| `qwen3-coder-plus` | Qwen3 Coder Plus | 1M | 66K | — | — |
| `qwen3.5-flash` | Qwen 3.5 Flash | 1M | 64K | ✅ | ✅ |
| `qwen3.5-plus` | Qwen 3.5 Plus | 1M | 64K | ✅ | ✅ |
| `qwen3.6-plus` | Qwen 3.6 Plus | 1M | 64K | ✅ | ✅ |
| `claude-opus-4.6` | Claude Opus 4.6 | 1M | 128K | ✅ | ✅ |
| `claude-opus-4.7` | Claude Opus 4.7 | 1M | 128K | ✅ | ✅ |
| `claude-sonnet-4` | Claude Sonnet 4 | 1M | 64K | ✅ | ✅ |
| `claude-sonnet-4.5` | Claude Sonnet 4.5 | 1M | 64K | ✅ | ✅ |
| `claude-sonnet-4.6` | Claude Sonnet 4.6 | 1M | 128K | ✅ | ✅ |
| `deepseek-v4-flash` | DeepSeek V4 Flash | 1M | 384K | — | ✅ |
| `deepseek-v4-pro` | DeepSeek V4 Pro | 1M | 384K | — | ✅ |
| `gemini-2.5-flash` | Gemini 2.5 Flash | 1M | 66K | ✅ | ✅ |
| `gemini-3-flash` | Gemini 3 Flash | 1M | 65K | ✅ | ✅ |
| `gemini-3-pro-preview` | Gemini 3 Pro Preview | 1M | 64K | ✅ | ✅ |
| `gemini-3.1-flash-lite` | Gemini 3.1 Flash Lite | 1M | 65K | ✅ | ✅ |
| `gemini-3.1-flash-lite-preview` | Gemini 3.1 Flash Lite Preview | 1M | 65K | ✅ | ✅ |
| `gemini-3.1-pro-preview` | Gemini 3.1 Pro Preview | 1M | 64K | ✅ | ✅ |
| `gpt-5.5` | GPT 5.5 | 1M | 128K | ✅ | ✅ |
| `gpt-5.5-pro` | GPT 5.5 Pro | 1M | 128K | ✅ | ✅ |
| `grok-4.3` | Grok 4.3 | 1M | 1M | ✅ | ✅ |
| `mimo-v2-pro` | MiMo V2 Pro | 1M | 128K | — | ✅ |
| `gpt-5` | GPT-5 | 400K | 128K | ✅ | ✅ |
| `gpt-5-codex` | GPT-5-Codex | 400K | 128K | — | ✅ |
| `gpt-5-mini` | GPT-5 mini | 400K | 128K | ✅ | ✅ |
| `gpt-5-nano` | GPT-5 nano | 400K | 128K | ✅ | ✅ |
| `gpt-5-pro` | GPT-5 pro | 400K | 272K | ✅ | ✅ |
| `gpt-5.1-codex` | GPT-5.1-Codex | 400K | 128K | ✅ | ✅ |
| `gpt-5.1-codex-max` | GPT 5.1 Codex Max | 400K | 128K | ✅ | ✅ |
| `gpt-5.1-codex-mini` | GPT 5.1 Codex Mini | 400K | 128K | ✅ | ✅ |
| `gpt-5.1-thinking` | GPT 5.1 Thinking | 400K | 128K | ✅ | ✅ |
| `gpt-5.2` | GPT 5.2 | 400K | 128K | ✅ | ✅ |
| `gpt-5.2-codex` | GPT 5.2 Codex | 400K | 128K | ✅ | ✅ |
| `gpt-5.2-pro` | GPT 5.2  | 400K | 128K | ✅ | ✅ |
| `gpt-5.3-codex` | GPT 5.3 Codex | 400K | 128K | ✅ | ✅ |
| `gpt-5.4-mini` | GPT 5.4 Mini | 400K | 128K | ✅ | ✅ |
| `gpt-5.4-nano` | GPT 5.4 Nano | 400K | 128K | ✅ | ✅ |
| `qwen3-coder` | Qwen3 Coder 480B A35B Instruct | 262K | 66K | — | — |
| `qwen3-coder-30b-a3b` | Qwen 3 Coder 30B A3B Instruct | 262K | 8K | — | ✅ |
| `qwen3-max` | Qwen3 Max | 262K | 33K | — | — |
| `qwen3-max-preview` | Qwen3 Max Preview | 262K | 33K | — | — |
| `gemma-4-26b-a4b-it` | Gemma 4 26B A4B IT | 262K | 131K | ✅ | — |
| `gemma-4-31b-it` | Gemma 4 31B IT | 262K | 131K | ✅ | — |
| `mimo-v2-flash` | MiMo V2 Flash | 262K | 32K | — | ✅ |
| `kimi-k2-thinking` | Kimi K2 Thinking | 262K | 262K | — | ✅ |
| `kimi-k2-thinking-turbo` | Kimi K2 Thinking Turbo | 262K | 262K | — | ✅ |
| `kimi-k2.5` | Kimi K2.5 | 262K | 262K | ✅ | ✅ |
| `trinity-large-thinking` | Trinity Large Thinking | 262K | 80K | — | ✅ |
| `kimi-k2.6` | Kimi K2.6 | 262K | 262K | ✅ | ✅ |
| `qwen3-coder-next` | Qwen3 Coder Next | 256K | 256K | — | — |
| `qwen3-max-thinking` | Qwen 3 Max Thinking | 256K | 66K | — | ✅ |
| `qwen3.6-27b` | Qwen 3.6 27B | 256K | 256K | ✅ | ✅ |
| `seed-1.6` | Seed 1.6 | 256K | 32K | — | ✅ |
| `command-a` | Command A | 256K | 8K | — | — |
| `kat-coder-pro-v2` | Kat Coder Pro V2 | 256K | 256K | — | ✅ |
| `devstral-2` | Devstral 2 | 256K | 256K | — | — |
| `devstral-small-2` | Devstral Small 2 | 256K | 256K | — | — |
| `kimi-k2-turbo` | Kimi K2 Turbo | 256K | 16K | — | — |
| `grok-4` | Grok 4 | 256K | 256K | ✅ | ✅ |
| `grok-code-fast-1` | Grok Code Fast 1 | 256K | 256K | — | ✅ |
| `qwen-3.6-max-preview` | Qwen 3.6 Max Preview | 240K | 64K | ✅ | ✅ |
| `minimax-m2` | MiniMax M2 | 205K | 205K | — | ✅ |
| `minimax-m2.1` | MiniMax M2.1 | 205K | 131K | — | ✅ |
| `minimax-m2.1-lightning` | MiniMax M2.1 Lightning | 205K | 131K | — | ✅ |
| `minimax-m2.5` | MiniMax M2.5 | 205K | 131K | — | ✅ |
| `minimax-m2.5-highspeed` | MiniMax M2.5 High Speed | 205K | 131K | — | ✅ |
| `minimax-m2.7` | Minimax M2.7 | 205K | 131K | ✅ | ✅ |
| `minimax-m2.7-highspeed` | MiniMax M2.7 High Speed | 205K | 131K | ✅ | ✅ |
| `glm-5` | GLM 5 | 203K | 131K | — | ✅ |
| `glm-5-turbo` | GLM 5 Turbo | 203K | 131K | — | ✅ |
| `glm-5.1` | GLM 5.1 | 203K | 64K | — | ✅ |
| `claude-3-haiku` | Claude 3 Haiku | 200K | 4K | ✅ | — |
| `claude-3.5-haiku` | Claude 3.5 Haiku | 200K | 8K | ✅ | — |
| `claude-haiku-4.5` | Claude Haiku 4.5 | 200K | 64K | ✅ | ✅ |
| `claude-opus-4` | Claude Opus 4 | 200K | 32K | ✅ | ✅ |
| `claude-opus-4.1` | Claude Opus 4.1 | 200K | 32K | ✅ | ✅ |
| `claude-opus-4.5` | Claude Opus 4.5 | 200K | 64K | ✅ | ✅ |
| `o1` | o1 | 200K | 100K | ✅ | ✅ |
| `o3` | o3 | 200K | 100K | ✅ | ✅ |
| `o3-deep-research` | o3-deep-research | 200K | 100K | ✅ | ✅ |
| `o3-mini` | o3-mini | 200K | 100K | — | ✅ |
| `o3-pro` | o3 Pro | 200K | 100K | ✅ | ✅ |
| `o4-mini` | o4-mini | 200K | 100K | ✅ | ✅ |
| `sonar-pro` | Sonar Pro | 200K | 8K | ✅ | — |
| `glm-4.6` | GLM 4.6 | 200K | 96K | — | ✅ |
| `glm-4.7-flash` | GLM 4.7 Flash | 200K | 131K | — | ✅ |
| `glm-4.7-flashx` | GLM 4.7 FlashX | 200K | 128K | — | ✅ |
| `glm-5v-turbo` | GLM 5V Turbo | 200K | 128K | ✅ | ✅ |
| `deepseek-v3` | DeepSeek V3 0324 | 164K | 16K | — | — |
| `deepseek-v3.1` | DeepSeek-V3.1 | 164K | 8K | — | ✅ |
| `qwen3-235b-a22b-thinking` | Qwen3 VL 235B A22B Thinking | 131K | 33K | ✅ | ✅ |
| `qwen3-vl-thinking` | Qwen3 VL 235B A22B Thinking | 131K | 33K | ✅ | ✅ |
| `deepseek-v3.1-terminus` | DeepSeek V3.1 Terminus | 131K | 66K | — | ✅ |
| `kimi-k2` | Kimi K2 Instruct | 131K | 131K | — | — |
| `nemotron-nano-12b-v2-vl` | Nvidia Nemotron Nano 12B V2 VL | 131K | 131K | ✅ | ✅ |
| `nemotron-nano-9b-v2` | Nvidia Nemotron Nano 9B V2 | 131K | 131K | — | ✅ |
| `gpt-oss-20b` | GPT OSS 120B | 131K | 8K | — | ✅ |
| `gpt-oss-safeguard-20b` | GPT OSS Safeguard 20B | 131K | 66K | — | ✅ |
| `grok-3` | Grok 3 Beta | 131K | 131K | — | — |
| `grok-3-fast` | Grok 3 Fast Beta | 131K | 131K | — | — |
| `grok-3-mini` | Grok 3 Mini Beta | 131K | 131K | — | — |
| `grok-3-mini-fast` | Grok 3 Mini Fast Beta | 131K | 131K | — | — |
| `qwen-3-235b` | Qwen3 235B A22b Instruct 2507 | 131K | 40K | — | — |
| `trinity-large-preview` | Trinity Large Preview | 131K | 131K | — | — |
| `glm-4.7` | GLM 4.7 | 131K | 40K | — | ✅ |
| `qwen-3-32b` | Qwen 3 32B | 128K | 8K | — | ✅ |
| `deepseek-r1` | DeepSeek-R1 | 128K | 8K | — | ✅ |
| `deepseek-v3.2` | DeepSeek V3.2 | 128K | 8K | — | — |
| `deepseek-v3.2-thinking` | DeepSeek V3.2 Thinking | 128K | 8K | — | — |
| `mercury-2` | Mercury 2 | 128K | 128K | — | ✅ |
| `longcat-flash-chat` | LongCat Flash Chat | 128K | 100K | — | — |
| `llama-3.1-70b` | Llama 3.1 70B Instruct | 128K | 8K | — | — |
| `llama-3.1-8b` | Llama 3.1 8B Instruct | 128K | 8K | — | — |
| `llama-3.2-11b` | Llama 3.2 11B Vision Instruct | 128K | 8K | ✅ | — |
| `llama-3.2-90b` | Llama 3.2 90B Vision Instruct | 128K | 8K | ✅ | — |
| `llama-3.3-70b` | Llama 3.3 70B Instruct | 128K | 8K | — | — |
| `llama-4-maverick` | Llama 4 Maverick 17B Instruct | 128K | 8K | ✅ | — |
| `llama-4-scout` | Llama 4 Scout 17B Instruct | 128K | 8K | ✅ | — |
| `codestral` | Mistral Codestral | 128K | 4K | — | — |
| `devstral-small` | Devstral Small 1.1 | 128K | 64K | — | — |
| `ministral-3b` | Ministral 3B | 128K | 4K | — | — |
| `ministral-8b` | Ministral 8B | 128K | 4K | — | — |
| `mistral-medium` | Mistral Medium 3.1 | 128K | 64K | ✅ | — |
| `pixtral-12b` | Pixtral 12B 2409 | 128K | 4K | ✅ | — |
| `pixtral-large` | Pixtral Large | 128K | 4K | ✅ | — |
| `gpt-4-turbo` | GPT-4 Turbo | 128K | 4K | ✅ | — |
| `gpt-4o` | GPT-4o | 128K | 16K | ✅ | — |
| `gpt-4o-mini` | GPT-4o mini | 128K | 16K | ✅ | — |
| `gpt-5-chat` | GPT 5 Chat | 128K | 16K | ✅ | ✅ |
| `gpt-5.1-instant` | GPT-5.1 Instant | 128K | 16K | ✅ | ✅ |
| `gpt-5.2-chat` | GPT 5.2 Chat | 128K | 16K | ✅ | ✅ |
| `gpt-5.3-chat` | GPT-5.3 Chat | 128K | 16K | ✅ | ✅ |
| `glm-4.5` | GLM-4.5 | 128K | 96K | — | ✅ |
| `glm-4.5-air` | GLM 4.5 Air | 128K | 96K | — | ✅ |
| `glm-4.6v` | GLM-4.6V | 128K | 24K | ✅ | ✅ |
| `glm-4.6v-flash` | GLM-4.6V-Flash | 128K | 24K | ✅ | ✅ |
| `sonar` | Sonar | 127K | 8K | ✅ | — |
| `glm-4.5v` | GLM 4.5V | 66K | 16K | ✅ | — |
| `qwen-3-14b` | Qwen3-14B | 41K | 16K | — | ✅ |
| `qwen-3-30b` | Qwen3-30B-A3B | 41K | 16K | — | ✅ |
| `mercury-coder-small` | Mercury Coder Small Beta | 32K | 16K | — | — |
| `mistral-small` | Mistral Small | 32K | 4K | ✅ | — |

### xai

**Base URL:** `https://api.x.ai/v1`

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 |
|---|---|---|---|---|---|
| `grok-4-1-fast` | Grok 4.1 Fast | 2M | 30K | ✅ | ✅ |
| `grok-4-1-fast-non-reasoning` | Grok 4.1 Fast (Non-Reasoning) | 2M | 30K | ✅ | — |
| `grok-4-fast` | Grok 4 Fast | 2M | 30K | ✅ | ✅ |
| `grok-4-fast-non-reasoning` | Grok 4 Fast (Non-Reasoning) | 2M | 30K | ✅ | — |
| `grok-4.20-0309-non-reasoning` | Grok 4.20 (Non-Reasoning) | 2M | 30K | ✅ | — |
| `grok-4.20-0309-reasoning` | Grok 4.20 (Reasoning) | 2M | 30K | ✅ | ✅ |
| `grok-4.3` | Grok 4.3 | 1M | 30K | ✅ | ✅ |
| `grok-4` | Grok 4 | 256K | 64K | — | ✅ |
| `grok-code-fast-1` | Grok Code Fast 1 | 256K | 10K | — | ✅ |
| `grok-2` | Grok 2 | 131K | 8K | — | — |
| `grok-2-1212` | Grok 2 (1212) | 131K | 8K | — | — |
| `grok-2-latest` | Grok 2 Latest | 131K | 8K | — | — |
| `grok-3` | Grok 3 | 131K | 8K | — | — |
| `grok-3-fast` | Grok 3 Fast | 131K | 8K | — | — |
| `grok-3-fast-latest` | Grok 3 Fast Latest | 131K | 8K | — | — |
| `grok-3-latest` | Grok 3 Latest | 131K | 8K | — | — |
| `grok-3-mini` | Grok 3 Mini | 131K | 8K | — | ✅ |
| `grok-3-mini-fast` | Grok 3 Mini Fast | 131K | 8K | — | ✅ |
| `grok-3-mini-fast-latest` | Grok 3 Mini Fast Latest | 131K | 8K | — | ✅ |
| `grok-3-mini-latest` | Grok 3 Mini Latest | 131K | 8K | — | ✅ |
| `grok-beta` | Grok Beta | 131K | 4K | — | — |
| `grok-2-vision` | Grok 2 Vision | 8K | 4K | ✅ | — |
| `grok-2-vision-1212` | Grok 2 Vision (1212) | 8K | 4K | ✅ | — |
| `grok-2-vision-latest` | Grok 2 Vision Latest | 8K | 4K | ✅ | — |
| `grok-vision-beta` | Grok Vision Beta | 8K | 4K | ✅ | — |

### xiaomi

**Base URL:** `https://api.xiaomimimo.com/anthropic`

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 |
|---|---|---|---|---|---|
| `mimo-v2-pro` | MiMo-V2-Pro | 1M | 131K | — | ✅ |
| `mimo-v2.5` | MiMo-V2.5 | 1M | 131K | ✅ | ✅ |
| `mimo-v2.5-pro` | MiMo-V2.5-Pro | 1M | 131K | — | ✅ |
| `mimo-v2-flash` | MiMo-V2-Flash | 262K | 66K | — | ✅ |
| `mimo-v2-omni` | MiMo-V2-Omni | 262K | 131K | ✅ | ✅ |

### xiaomi-token-plan-ams

**Base URL:** `https://token-plan-ams.xiaomimimo.com/anthropic`

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 |
|---|---|---|---|---|---|
| `mimo-v2-pro` | MiMo-V2-Pro | 1M | 131K | — | ✅ |
| `mimo-v2.5` | MiMo-V2.5 | 1M | 131K | ✅ | ✅ |
| `mimo-v2.5-pro` | MiMo-V2.5-Pro | 1M | 131K | — | ✅ |
| `mimo-v2-flash` | MiMo-V2-Flash | 262K | 66K | — | ✅ |
| `mimo-v2-omni` | MiMo-V2-Omni | 262K | 131K | ✅ | ✅ |

### xiaomi-token-plan-cn

**Base URL:** `https://token-plan-cn.xiaomimimo.com/anthropic`

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 |
|---|---|---|---|---|---|
| `mimo-v2-pro` | MiMo-V2-Pro | 1M | 131K | — | ✅ |
| `mimo-v2.5` | MiMo-V2.5 | 1M | 131K | ✅ | ✅ |
| `mimo-v2.5-pro` | MiMo-V2.5-Pro | 1M | 131K | — | ✅ |
| `mimo-v2-flash` | MiMo-V2-Flash | 262K | 66K | — | ✅ |
| `mimo-v2-omni` | MiMo-V2-Omni | 262K | 131K | ✅ | ✅ |

### xiaomi-token-plan-sgp

**Base URL:** `https://token-plan-sgp.xiaomimimo.com/anthropic`

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 |
|---|---|---|---|---|---|
| `mimo-v2-pro` | MiMo-V2-Pro | 1M | 131K | — | ✅ |
| `mimo-v2.5` | MiMo-V2.5 | 1M | 131K | ✅ | ✅ |
| `mimo-v2.5-pro` | MiMo-V2.5-Pro | 1M | 131K | — | ✅ |
| `mimo-v2-flash` | MiMo-V2-Flash | 262K | 66K | — | ✅ |
| `mimo-v2-omni` | MiMo-V2-Omni | 262K | 131K | ✅ | ✅ |

### zai

**Base URL:** `https://api.z.ai/api/paas/v4`

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 |
|---|---|---|---|---|---|
| `glm-4.6` | GLM-4.6 | 205K | 131K | — | ✅ |
| `glm-4.7` | GLM-4.7 | 205K | 131K | — | ✅ |
| `glm-5` | GLM-5 | 205K | 131K | — | ✅ |
| `glm-4.7-flash` | GLM-4.7-Flash | 200K | 131K | — | ✅ |
| `glm-4.7-flashx` | GLM-4.7-FlashX | 200K | 131K | — | ✅ |
| `glm-5-turbo` | GLM-5-Turbo | 200K | 131K | — | ✅ |
| `glm-5.1` | GLM-5.1 | 200K | 131K | — | ✅ |
| `glm-5v-turbo` | GLM-5V-Turbo | 200K | 131K | ✅ | ✅ |
| `glm-4.5` | GLM-4.5 | 131K | 98K | — | ✅ |
| `glm-4.5-air` | GLM-4.5-Air | 131K | 98K | — | ✅ |
| `glm-4.5-flash` | GLM-4.5-Flash | 131K | 98K | — | ✅ |
| `glm-4.6v` | GLM-4.6V | 128K | 33K | ✅ | ✅ |
| `glm-4.5v` | GLM-4.5V | 64K | 16K | ✅ | ✅ |

### zhipuai

**Base URL:** `https://open.bigmodel.cn/api/paas/v4`

| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 |
|---|---|---|---|---|---|
| `glm-4.6` | GLM-4.6 | 205K | 131K | — | ✅ |
| `glm-4.7` | GLM-4.7 | 205K | 131K | — | ✅ |
| `glm-5` | GLM-5 | 205K | 131K | — | ✅ |
| `glm-4.7-flash` | GLM-4.7-Flash | 200K | 131K | — | ✅ |
| `glm-4.7-flashx` | GLM-4.7-FlashX | 200K | 131K | — | ✅ |
| `glm-5.1` | GLM-5.1 | 200K | 131K | — | ✅ |
| `glm-5v-turbo` | GLM-5V-Turbo | 200K | 131K | ✅ | ✅ |
| `glm-4.5` | GLM-4.5 | 131K | 98K | — | ✅ |
| `glm-4.5-air` | GLM-4.5-Air | 131K | 98K | — | ✅ |
| `glm-4.5-flash` | GLM-4.5-Flash | 131K | 98K | — | ✅ |
| `glm-4.6v` | GLM-4.6V | 128K | 33K | ✅ | ✅ |
| `glm-4.5v` | GLM-4.5V | 64K | 16K | ✅ | ✅ |

