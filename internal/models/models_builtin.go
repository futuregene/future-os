package models

// BuiltinModels returns a hardcoded list of 20+ common models.
// This serves as both the default set and documentation for users.
func BuiltinModels() []ModelInfo {
	return []ModelInfo{
		// --- OpenAI ---
		{
			ID: "gpt-4o", Name: "GPT-4o", Provider: "openai", API: "openai-completions",
			BaseURL: "https://api.openai.com/v1", MaxTokens: 128000,
			SupportsVision: true, SupportsThinking: false, SupportsTools: true,
			Pricing: Pricing{Prompt: 2.50, Completion: 10.00},
		},
		{
			ID: "gpt-4o-mini", Name: "GPT-4o Mini", Provider: "openai", API: "openai-completions",
			BaseURL: "https://api.openai.com/v1", MaxTokens: 128000,
			SupportsVision: true, SupportsThinking: false, SupportsTools: true,
			Pricing: Pricing{Prompt: 0.15, Completion: 0.60},
		},
		{
			ID: "gpt-4.1", Name: "GPT-4.1", Provider: "openai", API: "openai-completions",
			BaseURL: "https://api.openai.com/v1", MaxTokens: 1000000,
			SupportsVision: true, SupportsThinking: false, SupportsTools: true,
			Pricing: Pricing{Prompt: 2.00, Completion: 8.00},
		},
		{
			ID: "gpt-4.1-mini", Name: "GPT-4.1 Mini", Provider: "openai", API: "openai-completions",
			BaseURL: "https://api.openai.com/v1", MaxTokens: 1000000,
			SupportsVision: true, SupportsThinking: false, SupportsTools: true,
			Pricing: Pricing{Prompt: 0.40, Completion: 1.60},
		},
		{
			ID: "gpt-4.1-nano", Name: "GPT-4.1 Nano", Provider: "openai", API: "openai-completions",
			BaseURL: "https://api.openai.com/v1", MaxTokens: 1000000,
			SupportsVision: true, SupportsThinking: false, SupportsTools: true,
			Pricing: Pricing{Prompt: 0.10, Completion: 0.40},
		},
		{
			ID: "o1", Name: "O1", Provider: "openai", API: "openai-completions",
			BaseURL: "https://api.openai.com/v1", MaxTokens: 200000,
			SupportsVision: true, SupportsThinking: true, SupportsTools: true,
			Pricing: Pricing{Prompt: 15.00, Completion: 60.00},
		},
		{
			ID: "o3-mini", Name: "O3 Mini", Provider: "openai", API: "openai-completions",
			BaseURL: "https://api.openai.com/v1", MaxTokens: 200000,
			SupportsVision: false, SupportsThinking: true, SupportsTools: true,
			Pricing: Pricing{Prompt: 1.10, Completion: 4.40},
		},
		{
			ID: "o4-mini", Name: "O4 Mini", Provider: "openai", API: "openai-completions",
			BaseURL: "https://api.openai.com/v1", MaxTokens: 200000,
			SupportsVision: true, SupportsThinking: true, SupportsTools: true,
			Pricing: Pricing{Prompt: 1.10, Completion: 4.40},
		},

		// --- Anthropic ---
		{
			ID: "claude-sonnet-4", Name: "Claude Sonnet 4", Provider: "anthropic", API: "anthropic-messages",
			BaseURL: "https://api.anthropic.com/v1", MaxTokens: 200000,
			SupportsVision: true, SupportsThinking: true, SupportsTools: true,
			Pricing: Pricing{Prompt: 3.00, Completion: 15.00},
		},
		{
			ID: "claude-opus-4", Name: "Claude Opus 4", Provider: "anthropic", API: "anthropic-messages",
			BaseURL: "https://api.anthropic.com/v1", MaxTokens: 200000,
			SupportsVision: true, SupportsThinking: true, SupportsTools: true,
			Pricing: Pricing{Prompt: 15.00, Completion: 75.00},
		},
		{
			ID: "claude-3.5-haiku", Name: "Claude Haiku 3.5", Provider: "anthropic", API: "anthropic-messages",
			BaseURL: "https://api.anthropic.com/v1", MaxTokens: 200000,
			SupportsVision: true, SupportsThinking: false, SupportsTools: true,
			Pricing: Pricing{Prompt: 0.80, Completion: 4.00},
		},

		// --- DeepSeek ---
		{
			ID: "deepseek-chat", Name: "DeepSeek Chat", Provider: "deepseek", API: "openai-completions",
			BaseURL: "https://api.deepseek.com/v1", MaxTokens: 128000,
			SupportsVision: false, SupportsThinking: false, SupportsTools: true,
			Pricing: Pricing{Prompt: 0.27, Completion: 1.10},
		},
		{
			ID: "deepseek-reasoner", Name: "DeepSeek Reasoner", Provider: "deepseek", API: "openai-completions",
			BaseURL: "https://api.deepseek.com/v1", MaxTokens: 128000,
			SupportsVision: false, SupportsThinking: true, SupportsTools: false,
			Pricing: Pricing{Prompt: 0.55, Completion: 2.19},
		},

		// --- Qwen (Alibaba) ---
		{
			ID: "qwen3.6-plus", Name: "Qwen 3.6 Plus", Provider: "qwen", API: "openai-completions",
			BaseURL: "https://dashscope-intl.aliyuncs.com/compatible-mode/v1", MaxTokens: 131072,
			SupportsVision: true, SupportsThinking: true, SupportsTools: true,
			Pricing: Pricing{Prompt: 0.80, Completion: 3.20},
		},
		{
			ID: "qwen3.6-max", Name: "Qwen 3.6 Max", Provider: "qwen", API: "openai-completions",
			BaseURL: "https://dashscope-intl.aliyuncs.com/compatible-mode/v1", MaxTokens: 131072,
			SupportsVision: true, SupportsThinking: true, SupportsTools: true,
			Pricing: Pricing{Prompt: 4.00, Completion: 16.00},
		},
		{
			ID: "qwen-coder-plus", Name: "Qwen Coder Plus", Provider: "qwen", API: "openai-completions",
			BaseURL: "https://dashscope-intl.aliyuncs.com/compatible-mode/v1", MaxTokens: 131072,
			SupportsVision: false, SupportsThinking: false, SupportsTools: true,
			Pricing: Pricing{Prompt: 1.50, Completion: 6.00},
		},

		// --- Google Gemini ---
		{
			ID: "gemini-2.5-flash", Name: "Gemini 2.5 Flash", Provider: "google", API: "openai-completions",
			BaseURL: "https://generativelanguage.googleapis.com/v1beta/openai", MaxTokens: 1048576,
			SupportsVision: true, SupportsThinking: true, SupportsTools: true,
			Pricing: Pricing{Prompt: 0.15, Completion: 0.60},
		},
		{
			ID: "gemini-2.5-pro", Name: "Gemini 2.5 Pro", Provider: "google", API: "openai-completions",
			BaseURL: "https://generativelanguage.googleapis.com/v1beta/openai", MaxTokens: 1048576,
			SupportsVision: true, SupportsThinking: true, SupportsTools: true,
			Pricing: Pricing{Prompt: 1.25, Completion: 10.00},
		},

		// --- xAI Grok ---
		{
			ID: "grok-3", Name: "Grok 3", Provider: "xai", API: "openai-completions",
			BaseURL: "https://api.x.ai/v1", MaxTokens: 131072,
			SupportsVision: true, SupportsThinking: true, SupportsTools: true,
			Pricing: Pricing{Prompt: 3.00, Completion: 15.00},
		},
		{
			ID: "grok-3-mini", Name: "Grok 3 Mini", Provider: "xai", API: "openai-completions",
			BaseURL: "https://api.x.ai/v1", MaxTokens: 131072,
			SupportsVision: true, SupportsThinking: true, SupportsTools: true,
			Pricing: Pricing{Prompt: 0.30, Completion: 0.50},
		},

		// --- Meta Llama (via OpenRouter-style base URL) ---
		{
			ID: "llama-3.3-70b", Name: "Llama 3.3 70B", Provider: "meta", API: "openai-completions",
			BaseURL: "https://openrouter.ai/api/v1", MaxTokens: 131072,
			SupportsVision: false, SupportsThinking: false, SupportsTools: true,
			Pricing: Pricing{Prompt: 0.12, Completion: 0.30},
		},
		{
			ID: "llama-4-maverick", Name: "Llama 4 Maverick", Provider: "meta", API: "openai-completions",
			BaseURL: "https://openrouter.ai/api/v1", MaxTokens: 131072,
			SupportsVision: true, SupportsThinking: false, SupportsTools: true,
			Pricing: Pricing{Prompt: 0.20, Completion: 0.80},
		},

		// --- Mistral ---
		{
			ID: "mistral-large", Name: "Mistral Large", Provider: "mistral", API: "openai-completions",
			BaseURL: "https://api.mistral.ai/v1", MaxTokens: 128000,
			SupportsVision: true, SupportsThinking: false, SupportsTools: true,
			Pricing: Pricing{Prompt: 2.00, Completion: 6.00},
		},
		{
			ID: "codestral", Name: "Codestral", Provider: "mistral", API: "openai-completions",
			BaseURL: "https://api.mistral.ai/v1", MaxTokens: 256000,
			SupportsVision: false, SupportsThinking: false, SupportsTools: true,
			Pricing: Pricing{Prompt: 0.30, Completion: 0.90},
		},
	}
}

// builtinMap returns a map from model ID to ModelInfo for quick lookup.
func builtinMap() map[string]ModelInfo {
	builtins := BuiltinModels()
	m := make(map[string]ModelInfo, len(builtins))
	for _, mi := range builtins {
		m[mi.ID] = mi
	}
	return m
}
