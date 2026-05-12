package models

// BuiltinConfig returns the built-in models in pi's provider-centric format.
func BuiltinConfig() *ModelsConfig {
	// Helper functions
	ptr := func(s string) *string { return &s }
	ptrB := func(b bool) *bool { return &b }
	ptrI := func(i int) *int { return &i }

	cfg := &ModelsConfig{Providers: make(map[string]ProviderConfig)}

	// --- OpenAI ---
	cfg.Providers["openai"] = ProviderConfig{
		API:     ptr("openai-completions"),
		BaseURL: ptr("https://api.openai.com/v1"),
		Compat: &Compat{
			SupportsDeveloperRole: ptrB(true),
		},
		Models: []ModelDefinition{
			{
				ID: "gpt-4o", Name: ptr("GPT-4o"),
				Reasoning: ptrB(false), Input: []string{"text", "image"},
				ContextWindow: ptrI(128000), MaxTokens: ptrI(16384),
				Cost: &ModelCost{Input: 2.50, Output: 10.00},
			},
			{
				ID: "gpt-4o-mini", Name: ptr("GPT-4o Mini"),
				Reasoning: ptrB(false), Input: []string{"text", "image"},
				ContextWindow: ptrI(128000), MaxTokens: ptrI(16384),
				Cost: &ModelCost{Input: 0.15, Output: 0.60},
			},
			{
				ID: "gpt-4.1", Name: ptr("GPT-4.1"),
				Reasoning: ptrB(false), Input: []string{"text", "image"},
				ContextWindow: ptrI(1000000), MaxTokens: ptrI(32768),
				Cost: &ModelCost{Input: 2.00, Output: 8.00},
			},
			{
				ID: "gpt-4.1-mini", Name: ptr("GPT-4.1 Mini"),
				Reasoning: ptrB(false), Input: []string{"text", "image"},
				ContextWindow: ptrI(1000000), MaxTokens: ptrI(32768),
				Cost: &ModelCost{Input: 0.40, Output: 1.60},
			},
			{
				ID: "gpt-4.1-nano", Name: ptr("GPT-4.1 Nano"),
				Reasoning: ptrB(false), Input: []string{"text", "image"},
				ContextWindow: ptrI(1000000), MaxTokens: ptrI(32768),
				Cost: &ModelCost{Input: 0.10, Output: 0.40},
			},
			{
				ID: "o1", Name: ptr("O1"),
				Reasoning: ptrB(true), Input: []string{"text", "image"},
				ContextWindow: ptrI(200000), MaxTokens: ptrI(100000),
				Cost: &ModelCost{Input: 15.00, Output: 60.00},
			},
			{
				ID: "o3-mini", Name: ptr("O3 Mini"),
				Reasoning: ptrB(true), Input: []string{"text"},
				ContextWindow: ptrI(200000), MaxTokens: ptrI(100000),
				Cost: &ModelCost{Input: 1.10, Output: 4.40},
			},
			{
				ID: "o4-mini", Name: ptr("O4 Mini"),
				Reasoning: ptrB(true), Input: []string{"text", "image"},
				ContextWindow: ptrI(200000), MaxTokens: ptrI(100000),
				Cost: &ModelCost{Input: 1.10, Output: 4.40},
			},
		},
	}

	// --- Anthropic ---
	cfg.Providers["anthropic"] = ProviderConfig{
		API:     ptr("anthropic-messages"),
		BaseURL: ptr("https://api.anthropic.com/v1"),
		Models: []ModelDefinition{
			{
				ID: "claude-sonnet-4", Name: ptr("Claude Sonnet 4"),
				Reasoning: ptrB(true), Input: []string{"text", "image"},
				ContextWindow: ptrI(200000), MaxTokens: ptrI(64000),
				Cost: &ModelCost{Input: 3.00, Output: 15.00},
			},
			{
				ID: "claude-opus-4", Name: ptr("Claude Opus 4"),
				Reasoning: ptrB(true), Input: []string{"text", "image"},
				ContextWindow: ptrI(200000), MaxTokens: ptrI(64000),
				Cost: &ModelCost{Input: 15.00, Output: 75.00},
			},
			{
				ID: "claude-3.5-haiku", Name: ptr("Claude Haiku 3.5"),
				Reasoning: ptrB(false), Input: []string{"text", "image"},
				ContextWindow: ptrI(200000), MaxTokens: ptrI(64000),
				Cost: &ModelCost{Input: 0.80, Output: 4.00},
			},
		},
	}

	// --- DeepSeek ---
	cfg.Providers["deepseek"] = ProviderConfig{
		API:     ptr("openai-completions"),
		BaseURL: ptr("https://api.deepseek.com/v1"),
		Models: []ModelDefinition{
			{
				ID: "deepseek-chat", Name: ptr("DeepSeek Chat"),
				Reasoning: ptrB(false), Input: []string{"text"},
				ContextWindow: ptrI(128000), MaxTokens: ptrI(8192),
				Cost: &ModelCost{Input: 0.27, Output: 1.10},
			},
			{
				ID: "deepseek-reasoner", Name: ptr("DeepSeek Reasoner"),
				Reasoning: ptrB(true), Input: []string{"text"},
				ContextWindow: ptrI(128000), MaxTokens: ptrI(8192),
				Cost: &ModelCost{Input: 0.55, Output: 2.19},
			},
		},
	}

	// --- Qwen (Alibaba) ---
	cfg.Providers["qwen"] = ProviderConfig{
		API:     ptr("openai-completions"),
		BaseURL: ptr("https://dashscope-intl.aliyuncs.com/compatible-mode/v1"),
		Compat: &Compat{
			SupportsDeveloperRole:   ptrB(false),
			SupportsReasoningEffort: ptrB(true),
		},
		Models: []ModelDefinition{
			{
				ID: "qwen3.6-plus", Name: ptr("Qwen 3.6 Plus"),
				Reasoning: ptrB(true), Input: []string{"text", "image"},
				ContextWindow: ptrI(131072), MaxTokens: ptrI(16384),
				Cost: &ModelCost{Input: 0.80, Output: 3.20},
			},
			{
				ID: "qwen3.6-max", Name: ptr("Qwen 3.6 Max"),
				Reasoning: ptrB(true), Input: []string{"text", "image"},
				ContextWindow: ptrI(131072), MaxTokens: ptrI(16384),
				Cost: &ModelCost{Input: 4.00, Output: 16.00},
			},
			{
				ID: "qwen-coder-plus", Name: ptr("Qwen Coder Plus"),
				Reasoning: ptrB(false), Input: []string{"text"},
				ContextWindow: ptrI(131072), MaxTokens: ptrI(16384),
				Cost: &ModelCost{Input: 1.50, Output: 6.00},
			},
		},
	}

	// --- Google Gemini ---
	cfg.Providers["google"] = ProviderConfig{
		API:     ptr("openai-completions"),
		BaseURL: ptr("https://generativelanguage.googleapis.com/v1beta/openai"),
		Models: []ModelDefinition{
			{
				ID: "gemini-2.5-flash", Name: ptr("Gemini 2.5 Flash"),
				Reasoning: ptrB(true), Input: []string{"text", "image"},
				ContextWindow: ptrI(1048576), MaxTokens: ptrI(65536),
				Cost: &ModelCost{Input: 0.15, Output: 0.60},
			},
			{
				ID: "gemini-2.5-pro", Name: ptr("Gemini 2.5 Pro"),
				Reasoning: ptrB(true), Input: []string{"text", "image"},
				ContextWindow: ptrI(1048576), MaxTokens: ptrI(65536),
				Cost: &ModelCost{Input: 1.25, Output: 10.00},
			},
		},
	}

	// --- xAI Grok ---
	cfg.Providers["xai"] = ProviderConfig{
		API:     ptr("openai-completions"),
		BaseURL: ptr("https://api.x.ai/v1"),
		Models: []ModelDefinition{
			{
				ID: "grok-3", Name: ptr("Grok 3"),
				Reasoning: ptrB(true), Input: []string{"text", "image"},
				ContextWindow: ptrI(131072), MaxTokens: ptrI(16384),
				Cost: &ModelCost{Input: 3.00, Output: 15.00},
			},
			{
				ID: "grok-3-mini", Name: ptr("Grok 3 Mini"),
				Reasoning: ptrB(true), Input: []string{"text", "image"},
				ContextWindow: ptrI(131072), MaxTokens: ptrI(16384),
				Cost: &ModelCost{Input: 0.30, Output: 0.50},
			},
		},
	}

	// --- Meta Llama ---
	cfg.Providers["meta"] = ProviderConfig{
		API:     ptr("openai-completions"),
		BaseURL: ptr("https://openrouter.ai/api/v1"),
		Models: []ModelDefinition{
			{
				ID: "llama-3.3-70b", Name: ptr("Llama 3.3 70B"),
				Reasoning: ptrB(false), Input: []string{"text"},
				ContextWindow: ptrI(131072), MaxTokens: ptrI(16384),
				Cost: &ModelCost{Input: 0.12, Output: 0.30},
			},
			{
				ID: "llama-4-maverick", Name: ptr("Llama 4 Maverick"),
				Reasoning: ptrB(false), Input: []string{"text", "image"},
				ContextWindow: ptrI(131072), MaxTokens: ptrI(16384),
				Cost: &ModelCost{Input: 0.20, Output: 0.80},
			},
		},
	}

	// --- Mistral ---
	cfg.Providers["mistral"] = ProviderConfig{
		API:     ptr("openai-completions"),
		BaseURL: ptr("https://api.mistral.ai/v1"),
		Models: []ModelDefinition{
			{
				ID: "mistral-large", Name: ptr("Mistral Large"),
				Reasoning: ptrB(false), Input: []string{"text", "image"},
				ContextWindow: ptrI(128000), MaxTokens: ptrI(16384),
				Cost: &ModelCost{Input: 2.00, Output: 6.00},
			},
			{
				ID: "codestral", Name: ptr("Codestral"),
				Reasoning: ptrB(false), Input: []string{"text"},
				ContextWindow: ptrI(256000), MaxTokens: ptrI(16384),
				Cost: &ModelCost{Input: 0.30, Output: 0.90},
			},
		},
	}

	return cfg
}
