package modelregistry

import "github.com/huichen/xihu/pkg/types"

// modelsCatalog is the embedded static model catalog.
// Mirrors TS pi-mono's models.generated.ts.
//
// To add a model, append to this slice. Fields:
//   - ID: model identifier (without provider prefix)
//   - Name: display name
//   - Provider: canonical provider name
//   - API: "openai-completions" | "anthropic-messages" | "mistral" | "google-gemini" | "cloudflare-workers"
//   - BaseURL: provider API endpoint
//   - ContextWindow: max context tokens
//   - MaxTokens: max output tokens
//   - Reasoning: supports thinking/reasoning
//
//nolint:lll
var modelsCatalog = []types.Model{
	// ── OpenAI ──────────────────────────────────────────────────────────────
	{ID: "gpt-5.4", Name: "GPT-5.4", Provider: "openai", API: "openai-completions", BaseURL: "https://api.openai.com/v1", ContextWindow: 400000, MaxTokens: 128000, Reasoning: true},
	{ID: "gpt-4o", Name: "GPT-4o", Provider: "openai", API: "openai-completions", BaseURL: "https://api.openai.com/v1", ContextWindow: 128000, MaxTokens: 16384, Reasoning: false,
		Cost: struct {
			Input     float64 `json:"input"`
			Output    float64 `json:"output"`
			CacheRead float64 `json:"cacheRead"`
			CacheWrite float64 `json:"cacheWrite"`
		}{Input: 2.50, Output: 10.00}},
	{ID: "gpt-4o-mini", Name: "GPT-4o Mini", Provider: "openai", API: "openai-completions", BaseURL: "https://api.openai.com/v1", ContextWindow: 128000, MaxTokens: 16384, Reasoning: false,
		Cost: struct {
			Input     float64 `json:"input"`
			Output    float64 `json:"output"`
			CacheRead float64 `json:"cacheRead"`
			CacheWrite float64 `json:"cacheWrite"`
		}{Input: 0.15, Output: 0.60}},
	{ID: "o4-mini", Name: "o4-mini", Provider: "openai", API: "openai-completions", BaseURL: "https://api.openai.com/v1", ContextWindow: 200000, MaxTokens: 100000, Reasoning: true},
	{ID: "o3", Name: "o3", Provider: "openai", API: "openai-completions", BaseURL: "https://api.openai.com/v1", ContextWindow: 200000, MaxTokens: 100000, Reasoning: true},
	{ID: "gpt-4.1", Name: "GPT-4.1", Provider: "openai", API: "openai-completions", BaseURL: "https://api.openai.com/v1", ContextWindow: 1000000, MaxTokens: 32768, Reasoning: false,
		Cost: struct {
			Input     float64 `json:"input"`
			Output    float64 `json:"output"`
			CacheRead float64 `json:"cacheRead"`
			CacheWrite float64 `json:"cacheWrite"`
		}{Input: 2.00, Output: 8.00}},

	// ── Anthropic ───────────────────────────────────────────────────────────
	{ID: "claude-sonnet-4-20250514", Name: "Claude Sonnet 4", Provider: "anthropic", API: "anthropic-messages", BaseURL: "https://api.anthropic.com/v1", ContextWindow: 200000, MaxTokens: 64000, Reasoning: true,
		Cost: struct {
			Input     float64 `json:"input"`
			Output    float64 `json:"output"`
			CacheRead float64 `json:"cacheRead"`
			CacheWrite float64 `json:"cacheWrite"`
		}{Input: 3.00, Output: 15.00, CacheRead: 0.30, CacheWrite: 3.75}},
	{ID: "claude-opus-4-20250514", Name: "Claude Opus 4", Provider: "anthropic", API: "anthropic-messages", BaseURL: "https://api.anthropic.com/v1", ContextWindow: 200000, MaxTokens: 64000, Reasoning: true,
		Cost: struct {
			Input     float64 `json:"input"`
			Output    float64 `json:"output"`
			CacheRead float64 `json:"cacheRead"`
			CacheWrite float64 `json:"cacheWrite"`
		}{Input: 15.00, Output: 75.00}},
	{ID: "claude-3.5-haiku", Name: "Claude 3.5 Haiku", Provider: "anthropic", API: "anthropic-messages", BaseURL: "https://api.anthropic.com/v1", ContextWindow: 200000, MaxTokens: 8192, Reasoning: false,
		Cost: struct {
			Input     float64 `json:"input"`
			Output    float64 `json:"output"`
			CacheRead float64 `json:"cacheRead"`
			CacheWrite float64 `json:"cacheWrite"`
		}{Input: 0.25, Output: 1.25}},

	// ── DeepSeek ────────────────────────────────────────────────────────────
	{ID: "deepseek-chat", Name: "DeepSeek V3", Provider: "deepseek", API: "openai-completions", BaseURL: "https://api.deepseek.com/v1", ContextWindow: 65536, MaxTokens: 8192, Reasoning: false,
		Cost: struct {
			Input     float64 `json:"input"`
			Output    float64 `json:"output"`
			CacheRead float64 `json:"cacheRead"`
			CacheWrite float64 `json:"cacheWrite"`
		}{Input: 0.14, Output: 0.28}},
	{ID: "deepseek-reasoner", Name: "DeepSeek R1", Provider: "deepseek", API: "openai-completions", BaseURL: "https://api.deepseek.com/v1", ContextWindow: 65536, MaxTokens: 8192, Reasoning: true,
		Cost: struct {
			Input     float64 `json:"input"`
			Output    float64 `json:"output"`
			CacheRead float64 `json:"cacheRead"`
			CacheWrite float64 `json:"cacheWrite"`
		}{Input: 0.55, Output: 2.19}},
	{ID: "deepseek-v4-pro", Name: "DeepSeek V4 Pro", Provider: "deepseek", API: "openai-completions", BaseURL: "https://api.deepseek.com/v1", ContextWindow: 131072, MaxTokens: 32000, Reasoning: false},
	{ID: "deepseek-v4-flash", Name: "DeepSeek V4 Flash", Provider: "deepseek", API: "openai-completions", BaseURL: "https://api.deepseek.com/v1", ContextWindow: 131072, MaxTokens: 32000, Reasoning: false},
	{ID: "deepseek-v4.5", Name: "DeepSeek V4.5", Provider: "deepseek", API: "openai-completions", BaseURL: "https://api.deepseek.com/v1", ContextWindow: 131072, MaxTokens: 32000, Reasoning: false},

	// ─── Google ─────────────────────────────────────────────────────────────
	{ID: "gemini-2.5-pro", Name: "Gemini 2.5 Pro", Provider: "google", API: "openai-completions", BaseURL: "https://generativelanguage.googleapis.com/v1beta/openai", ContextWindow: 1048576, MaxTokens: 65536, Reasoning: true},
	{ID: "gemini-2.5-flash", Name: "Gemini 2.5 Flash", Provider: "google", API: "openai-completions", BaseURL: "https://generativelanguage.googleapis.com/v1beta/openai", ContextWindow: 1048576, MaxTokens: 65536, Reasoning: false},

	// ─── Qwen / Alibaba ─────────────────────────────────────────────────────
	{ID: "qwen3.6-plus", Name: "Qwen 3.6 Plus", Provider: "alibaba", API: "openai-completions", BaseURL: "https://dashscope-intl.aliyuncs.com/compatible-mode/v1", ContextWindow: 131072, MaxTokens: 16384, Reasoning: false},
	{ID: "qwen3.6-max", Name: "Qwen 3.6 Max", Provider: "alibaba", API: "openai-completions", BaseURL: "https://dashscope-intl.aliyuncs.com/compatible-mode/v1", ContextWindow: 131072, MaxTokens: 16384, Reasoning: true},
	{ID: "qwen3.6-235b-a22b", Name: "Qwen 3.6 235B", Provider: "alibaba", API: "openai-completions", BaseURL: "https://dashscope-intl.aliyuncs.com/compatible-mode/v1", ContextWindow: 131072, MaxTokens: 32768, Reasoning: true},
	{ID: "qwen3.6-35b-a3b-q4", Name: "Qwen 3.6 35B Quant", Provider: "alibaba", API: "openai-completions", BaseURL: "https://dashscope-intl.aliyuncs.com/compatible-mode/v1", ContextWindow: 131072, MaxTokens: 8192, Reasoning: false},

	// ─── xAI (Grok) ─────────────────────────────────────────────────────────
	{ID: "grok-5", Name: "Grok 5", Provider: "xai", API: "openai-completions", BaseURL: "https://api.x.ai/v1", ContextWindow: 1000000, MaxTokens: 32000, Reasoning: true},
	{ID: "grok-5-mini", Name: "Grok 5 Mini", Provider: "xai", API: "openai-completions", BaseURL: "https://api.x.ai/v1", ContextWindow: 1000000, MaxTokens: 16384, Reasoning: false},

	// ─── Moonshot (Kimi) ────────────────────────────────────────────────────
	{ID: "kimi-k2-turbo", Name: "Kimi K2 Turbo", Provider: "moonshot", API: "openai-completions", BaseURL: "https://api.moonshot.cn/v1", ContextWindow: 131072, MaxTokens: 16384, Reasoning: false},
	{ID: "kimi-k2.5", Name: "Kimi K2.5", Provider: "moonshot", API: "openai-completions", BaseURL: "https://api.moonshot.cn/v1", ContextWindow: 262144, MaxTokens: 32768, Reasoning: true},

	// ─── Zhipu (GLM) ────────────────────────────────────────────────────────
	{ID: "glm-5", Name: "GLM-5", Provider: "zhipu", API: "openai-completions", BaseURL: "https://open.bigmodel.cn/api/paas/v4", ContextWindow: 131072, MaxTokens: 16384, Reasoning: false},
	{ID: "glm-5-reasoning", Name: "GLM-5 Reasoning", Provider: "zhipu", API: "openai-completions", BaseURL: "https://open.bigmodel.cn/api/paas/v4", ContextWindow: 131072, MaxTokens: 32768, Reasoning: true},

	// ─── MiniMax ────────────────────────────────────────────────────────────
	{ID: "minimax-m2.5", Name: "MiniMax M2.5", Provider: "minimax", API: "openai-completions", BaseURL: "https://api.minimax.chat/v1", ContextWindow: 262144, MaxTokens: 16384, Reasoning: false},

	// ─── ByteDance (Doubao) ─────────────────────────────────────────────────
	{ID: "doubao-thought-pro", Name: "Doubao Thought Pro", Provider: "bytedance", API: "openai-completions", BaseURL: "https://ark.cn-beijing.volces.com/api/v3", ContextWindow: 131072, MaxTokens: 32768, Reasoning: true},
	{ID: "doubao-pro-256k", Name: "Doubao Pro 256K", Provider: "bytedance", API: "openai-completions", BaseURL: "https://ark.cn-beijing.volces.com/api/v3", ContextWindow: 262144, MaxTokens: 16384, Reasoning: false},

	// ─── Mistral ─────────────────────────────────────────────────────────────
	{ID: "mistral-large", Name: "Mistral Large", Provider: "mistral", API: "mistral", BaseURL: "https://api.mistral.ai/v1", ContextWindow: 131072, MaxTokens: 131072, Reasoning: false,
		Cost: struct {
			Input     float64 `json:"input"`
			Output    float64 `json:"output"`
			CacheRead float64 `json:"cacheRead"`
			CacheWrite float64 `json:"cacheWrite"`
		}{Input: 2.00, Output: 6.00}},
	{ID: "mistral-small", Name: "Mistral Small", Provider: "mistral", API: "mistral", BaseURL: "https://api.mistral.ai/v1", ContextWindow: 32768, MaxTokens: 32768, Reasoning: false},
	{ID: "codestral", Name: "Codestral", Provider: "mistral", API: "mistral", BaseURL: "https://api.mistral.ai/v1", ContextWindow: 262144, MaxTokens: 262144, Reasoning: false},

	// ─── Cloudflare Workers AI ────────────────────────────────────────────────
	{ID: "@cf/meta/llama-4-maverick-17b-128e-instruct", Name: "Llama 4 Maverick (CF)", Provider: "cloudflare", API: "cloudflare-workers", BaseURL: "https://api.cloudflare.com/client/v4", ContextWindow: 131072, MaxTokens: 8192, Reasoning: false},
	{ID: "@cf/deepseek-ai/deepseek-r1-distill-qwen-32b", Name: "DeepSeek R1 Distill (CF)", Provider: "cloudflare", API: "cloudflare-workers", BaseURL: "https://api.cloudflare.com/client/v4", ContextWindow: 32768, MaxTokens: 16384, Reasoning: true},

	// ─── AWS Bedrock ─────────────────────────────────────────────────────────
	{ID: "us.anthropic.claude-sonnet-4-20250514-v1:0", Name: "Claude Sonnet 4 (Bedrock)", Provider: "aws", API: "anthropic-messages", BaseURL: "https://bedrock-runtime.us-east-1.amazonaws.com", ContextWindow: 200000, MaxTokens: 64000, Reasoning: true},
	{ID: "us.anthropic.claude-opus-4-20250514-v1:0", Name: "Claude Opus 4 (Bedrock)", Provider: "aws", API: "anthropic-messages", BaseURL: "https://bedrock-runtime.us-east-1.amazonaws.com", ContextWindow: 200000, MaxTokens: 64000, Reasoning: true},

	// ─── Groq ────────────────────────────────────────────────────────────────
	{ID: "llama-4-maverick-17b-128e-instruct", Name: "Llama 4 Maverick (Groq)", Provider: "groq", API: "openai-completions", BaseURL: "https://api.groq.com/openai/v1", ContextWindow: 131072, MaxTokens: 8192, Reasoning: false},
	{ID: "deepseek-r1-distill-llama-70b", Name: "DeepSeek R1 70B (Groq)", Provider: "groq", API: "openai-completions", BaseURL: "https://api.groq.com/openai/v1", ContextWindow: 131072, MaxTokens: 32768, Reasoning: true},

	// ─── Fireworks ───────────────────────────────────────────────────────────
	{ID: "accounts/fireworks/models/llama-v3p1-405b-instruct", Name: "Llama 3.1 405B (Fireworks)", Provider: "fireworks", API: "openai-completions", BaseURL: "https://api.fireworks.ai/inference/v1", ContextWindow: 131072, MaxTokens: 16384, Reasoning: false},
	{ID: "accounts/fireworks/models/deepseek-r1", Name: "DeepSeek R1 (Fireworks)", Provider: "fireworks", API: "openai-completions", BaseURL: "https://api.fireworks.ai/inference/v1", ContextWindow: 131072, MaxTokens: 32768, Reasoning: true},

	// ─── Together AI ─────────────────────────────────────────────────────────
	{ID: "meta-llama/Llama-4-Maverick-17B-128E-Instruct-FP8", Name: "Llama 4 Maverick (Together)", Provider: "together", API: "openai-completions", BaseURL: "https://api.together.xyz/v1", ContextWindow: 131072, MaxTokens: 8192, Reasoning: false},
	{ID: "deepseek-ai/DeepSeek-R1", Name: "DeepSeek R1 (Together)", Provider: "together", API: "openai-completions", BaseURL: "https://api.together.xyz/v1", ContextWindow: 131072, MaxTokens: 32768, Reasoning: true},
}
