//go:build ignore

// generate_models.go fetches model data from external APIs and generates
// models_generated.go. This mirrors pi-mono's packages/ai/scripts/generate-models.ts.
//
// Usage:
//
//	go run generate_models.go
//
// Data sources (same as pi):
//   - https://models.dev/api.json → all providers except openrouter/vercel
//   - https://openrouter.ai/api/v1/models → OpenRouter
//   - https://ai-gateway.vercel.sh/v1/models → Vercel AI Gateway
//
// The script filters to tool-capable models only and applies provider-specific
// configurations matching pi's generate-models.ts exactly.
package main

import (
	"encoding/json"
	"fmt"
	"net/http"
	"os"
	"sort"
	"strings"
	"time"
)

// Model mirrors the generated Go model definition.
type Model struct {
	ID            string
	Name          string
	API           string
	Provider      string
	BaseURL       string
	Reasoning     bool
	Input         []string
	ContextWindow int
	MaxTokens     int
	CostInput     float64
	CostOutput    float64
	CostCacheRead  float64
	CostCacheWrite float64
	// CompatJSON is pre-serialized JSON for the model's compat field.
	CompatJSON string
	// TLMJSON is pre-serialized JSON for the model's thinkingLevelMap field.
	TLMJSON string
	// HeadersJSON is pre-serialized JSON for the model's headers field.
	HeadersJSON string
}

// ─── models.dev types ────────────────────────────────────────────────────────

type ModelsDevResponse map[string]struct {
	Models map[string]ModelsDevModel `json:"models"`
}

type ModelsDevModel struct {
	Name      *string `json:"name"`
	ToolCall  *bool   `json:"tool_call"`
	Reasoning *bool   `json:"reasoning"`
	Modalities *struct {
		Input []string `json:"input"`
	} `json:"modalities"`
	Cost *struct {
		Input      *float64 `json:"input"`
		Output     *float64 `json:"output"`
		CacheRead  *float64 `json:"cache_read"`
		CacheWrite *float64 `json:"cache_write"`
	} `json:"cost"`
	Limit *struct {
		Context *int `json:"context"`
		Output  *int `json:"output"`
	} `json:"limit"`
}

func (m ModelsDevModel) name() string {
	if m.Name != nil {
		return *m.Name
	}
	return ""
}

func (m ModelsDevModel) toolCall() bool { return m.ToolCall != nil && *m.ToolCall }
func (m ModelsDevModel) reasoning() bool { return m.Reasoning != nil && *m.Reasoning }

func (m ModelsDevModel) contextWindow() int {
	if m.Limit != nil && m.Limit.Context != nil {
		return *m.Limit.Context
	}
	return 4096
}

func (m ModelsDevModel) maxTokens() int {
	if m.Limit != nil && m.Limit.Output != nil {
		return *m.Limit.Output
	}
	return 4096
}

func (m ModelsDevModel) costInput() float64 {
	if m.Cost != nil && m.Cost.Input != nil {
		return *m.Cost.Input
	}
	return 0
}

func (m ModelsDevModel) costOutput() float64 {
	if m.Cost != nil && m.Cost.Output != nil {
		return *m.Cost.Output
	}
	return 0
}

func (m ModelsDevModel) costCacheRead() float64 {
	if m.Cost != nil && m.Cost.CacheRead != nil {
		return *m.Cost.CacheRead
	}
	return 0
}

func (m ModelsDevModel) costCacheWrite() float64 {
	if m.Cost != nil && m.Cost.CacheWrite != nil {
		return *m.Cost.CacheWrite
	}
	return 0
}

func (m ModelsDevModel) hasImageInput() bool {
	if m.Modalities == nil {
		return false
	}
	for _, mod := range m.Modalities.Input {
		if mod == "image" || mod == "image_url" {
			return true
		}
	}
	return false
}

// ─── Thinking level map constants (mirrors pi's generate-models.ts) ────────

// deepseekV4TLM matches pi's DEEPSEEK_V4_THINKING_LEVEL_MAP.
var deepseekV4TLM = map[string]interface{}{
	"minimal": nil,
	"low":     nil,
	"medium":  nil,
	"high":    "high",
	"xhigh":   "max",
}

func tlmJSON(m map[string]interface{}) string {
	b, _ := json.Marshal(m)
	return string(b)
}

func mergeTLM(model *Model, m map[string]interface{}) {
	existing := make(map[string]interface{})
	if model.TLMJSON != "" {
		json.Unmarshal([]byte(model.TLMJSON), &existing)
	}
	for k, v := range m {
		existing[k] = v
	}
	model.TLMJSON = tlmJSON(existing)
}

func supportsOpenAiXhigh(modelID string) bool {
	return strings.Contains(modelID, "gpt-5.2") ||
		strings.Contains(modelID, "gpt-5.3") ||
		strings.Contains(modelID, "gpt-5.4") ||
		strings.Contains(modelID, "gpt-5.5")
}

func isGoogleThinkingAPI(api string) bool {
	return api == "google-generative-ai" || api == "google-vertex"
}

func isGemini3ProModel(id string) bool {
	return strings.Contains(strings.ToLower(id), "gemini-3") && strings.Contains(strings.ToLower(id), "pro")
}

func isGemini3FlashModel(id string) bool {
	return strings.Contains(strings.ToLower(id), "gemini-3") && strings.Contains(strings.ToLower(id), "flash")
}

func isGemma4Model(id string) bool {
	return strings.Contains(strings.ToLower(id), "gemma-4") || strings.Contains(strings.ToLower(id), "gemma4")
}

// applyThinkingLevelMetadata mirrors pi's applyThinkingLevelMetadata.
func applyThinkingLevelMetadata(m *Model) {
	api := m.API
	id := m.ID
	provider := m.Provider

	// gpt-5.x — off is null (thinking off mode maps to reasoning_effort=none)
	if strings.HasPrefix(id, "gpt-5") && (api == "openai-responses" || api == "azure-openai-responses" || api == "openai-completions") {
		mergeTLM(m, map[string]interface{}{"off": nil})
	}
	// GPT-5.2/5.3/5.4/5.5 → xhigh support
	if supportsOpenAiXhigh(id) {
		mergeTLM(m, map[string]interface{}{"xhigh": "xhigh"})
	}
	// Claude Opus 4.6
	if strings.Contains(id, "opus-4-6") || strings.Contains(id, "opus-4.6") {
		mergeTLM(m, map[string]interface{}{"xhigh": "max"})
	}
	// Claude Opus 4.7
	if strings.Contains(id, "opus-4-7") || strings.Contains(id, "opus-4.7") {
		mergeTLM(m, map[string]interface{}{"xhigh": "xhigh"})
	}
	// deepseek-v4 + openai-completions (initial pass)
	if api == "openai-completions" && strings.Contains(id, "deepseek-v4") {
		mergeTLM(m, deepseekV4TLM)
	}
	// Gemini 3 Pro (google/google-vertex providers, any API)
	if (provider == "google" || provider == "google-vertex" || isGoogleThinkingAPI(api)) && isGemini3ProModel(id) {
		mergeTLM(m, map[string]interface{}{
			"off": nil, "minimal": nil, "low": "LOW", "medium": nil, "high": "HIGH",
		})
	}
	// Gemini 3 Flash (google/google-vertex providers, any API)
	if (provider == "google" || provider == "google-vertex" || isGoogleThinkingAPI(api)) && isGemini3FlashModel(id) {
		mergeTLM(m, map[string]interface{}{"off": nil})
	}
	// Gemma 4 (google/google-vertex providers, any API)
	if (provider == "google" || provider == "google-vertex" || isGoogleThinkingAPI(api)) && isGemma4Model(id) {
		mergeTLM(m, map[string]interface{}{
			"off": nil, "minimal": "MINIMAL", "low": nil, "medium": nil, "high": "HIGH",
		})
	}
	// Groq qwen/qwen3-32b
	if provider == "groq" && id == "qwen/qwen3-32b" {
		mergeTLM(m, map[string]interface{}{
			"minimal": nil, "low": nil, "medium": nil, "high": "default",
		})
	}
	// openai-codex GPT-5
	if provider == "openai-codex" && supportsOpenAiXhigh(id) {
		mergeTLM(m, map[string]interface{}{"minimal": "low"})
	}
	// openai-codex gpt-5.1-codex-mini
	if provider == "openai-codex" && id == "gpt-5.1-codex-mini" {
		mergeTLM(m, map[string]interface{}{
			"minimal": "medium", "low": "medium", "medium": "medium", "high": "high",
		})
	}
}

// ─── OpenRouter types ────────────────────────────────────────────────────────

type OpenRouterModel struct {
	ID                  string          `json:"id"`
	Name                string          `json:"name"`
	ContextLength       int             `json:"context_length"`
	SupportedParameters []string        `json:"supported_parameters"`
	TopProvider         *struct {
		MaxCompletionTokens int `json:"max_completion_tokens"`
	} `json:"top_provider"`
	Architecture *struct {
		Modality json.RawMessage `json:"modality"` // string or []string
	} `json:"architecture"`
	Pricing *struct {
		Prompt            string `json:"prompt"`
		Completion        string `json:"completion"`
		InputCacheRead    string `json:"input_cache_read"`
		InputCacheWrite   string `json:"input_cache_write"`
	} `json:"pricing"`
}

type OpenRouterResponse struct {
	Data []OpenRouterModel `json:"data"`
}

// ─── Vercel AI Gateway types ─────────────────────────────────────────────────

type AiGatewayModel struct {
	ID           string   `json:"id"`
	Name         string   `json:"name"`
	ContextWindow int     `json:"context_window"`
	MaxTokens    int      `json:"max_tokens"`
	Tags         []string `json:"tags"`
	Pricing      *struct {
		Input          interface{} `json:"input"`
		Output         interface{} `json:"output"`
		InputCacheRead  interface{} `json:"input_cache_read"`
		InputCacheWrite interface{} `json:"input_cache_write"`
	} `json:"pricing"`
}

type AiGatewayResponse struct {
	Data []AiGatewayModel `json:"data"`
}

// ─── Provider configuration (mirrors pi's generate-models.ts) ────────────────

type providerConfig struct {
	api     string
	baseURL string
}

func main() {
	var allModels []Model

	// 1. Fetch models.dev API
	fmt.Println("Fetching models from models.dev...")
	modelsDev, err := fetchModelsDev()
	if err != nil {
		fmt.Fprintf(os.Stderr, "Warning: models.dev fetch failed: %v\n", err)
		modelsDev = nil
	}

	// 2. Fetch OpenRouter
	fmt.Println("Fetching models from OpenRouter...")
	openRouterModels, _ := fetchOpenRouter()
	allModels = append(allModels, openRouterModels...)

	// 3. Fetch Vercel AI Gateway
	fmt.Println("Fetching models from Vercel AI Gateway...")
	aiGatewayModels, _ := fetchAiGateway()
	allModels = append(allModels, aiGatewayModels...)

	// 4. Process models.dev providers (exact same list as pi)
	if modelsDev != nil {
		allModels = append(allModels, processModelsDev(modelsDev)...)
	}

	// 5. Mirror OpenAI models as azure-openai-responses
	allModels = append(allModels, mirrorAzureModels(allModels)...)

	// 6. Add opencode / opencode-go models (pi generates these with static IDs)
	allModels = append(allModels, opencodeModels()...)

	// ── Post-processing: compat, thinking level, context fixes (mirrors pi) ────
	// OpenCode variants list Claude Sonnet 4/4.5 with 1M context, actual limit is 200K
	for i := range allModels {
		m := &allModels[i]
		if (m.Provider == "opencode" || m.Provider == "opencode-go") && (m.ID == "claude-sonnet-4-5" || m.ID == "claude-sonnet-4") {
			m.ContextWindow = 200000
		}
		if (m.Provider == "opencode" || m.Provider == "opencode-go") && m.ID == "gpt-5.4" {
			m.ContextWindow = 272000
			m.MaxTokens = 128000
		}
		if m.Provider == "openai" && (m.ID == "gpt-5.4" || m.ID == "gpt-5.5") {
			m.ContextWindow = 272000
			m.MaxTokens = 128000
		}
		// Stabilize OpenRouter model metadata
		if m.Provider == "openrouter" && m.ID == "moonshotai/kimi-k2.5" {
			m.CostInput = 0.41
			m.CostOutput = 2.06
			m.CostCacheRead = 0.07
			m.MaxTokens = 4096
		}
		if m.Provider == "openrouter" && m.ID == "z-ai/glm-5" {
			m.CostInput = 0.6
			m.CostOutput = 1.9
			m.CostCacheRead = 0.119
		}
	}

	// DeepSeek V4 compat + thinking level map for all deepseek-v4 models
	deepseekCompatJSON := `{"requiresReasoningContentOnAssistantMessages":true,"thinkingFormat":"deepseek"}`
	for i := range allModels {
		m := &allModels[i]
		if m.API == "openai-completions" && strings.Contains(m.ID, "deepseek-v4") {
			if m.CompatJSON == "" {
				m.CompatJSON = deepseekCompatJSON
			} else {
				// Merge with existing
				var existing map[string]interface{}
				json.Unmarshal([]byte(m.CompatJSON), &existing)
				existing["requiresReasoningContentOnAssistantMessages"] = true
				existing["thinkingFormat"] = "deepseek"
				m.CompatJSON = tlmJSON(existing)
			}
			mergeTLM(m, deepseekV4TLM)
		}
	}

	// ── Provider-level compat (mirrors pi's per-provider settings) ──────────
	zaiCompat := `{"supportsDeveloperRole":false,"thinkingFormat":"zai","zaiToolStream":true}`
	// ZAI models that don't support tool stream
	zaiNoToolStream := map[string]bool{"glm-4.5": true, "glm-4.5-air": true, "glm-4.5-flash": true, "glm-4.5v": true}
	hfCompat := `{"supportsDeveloperRole":false}`
	moonshotCompat := `{"supportsStore":false,"supportsDeveloperRole":false,"supportsReasoningEffort":false,"maxTokensField":"max_tokens","supportsStrictMode":false}`
	copilotCompat := `{"supportsStore":false,"supportsDeveloperRole":false,"supportsReasoningEffort":false}`
	cfSessionCompat := `{"sendSessionAffinityHeaders":true}`

	for i := range allModels {
		m := &allModels[i]
		switch m.Provider {
		case "zai":
			if zaiNoToolStream[m.ID] {
				m.CompatJSON = `{"supportsDeveloperRole":false,"thinkingFormat":"zai"}`
			} else {
				m.CompatJSON = zaiCompat
			}
		case "huggingface":
			m.CompatJSON = hfCompat
		case "moonshotai", "moonshotai-cn":
			m.CompatJSON = moonshotCompat
		case "github-copilot":
			if m.API == "openai-completions" {
				m.CompatJSON = copilotCompat
			}
			m.HeadersJSON = `{"User-Agent":"GitHubCopilotChat/0.35.0","Editor-Version":"vscode/1.107.0","Editor-Plugin-Version":"copilot-chat/0.35.0","Copilot-Integration-Id":"vscode-chat"}`
		case "cloudflare-ai-gateway":
			if strings.Contains(m.ID, "workers-ai/") {
				m.CompatJSON = cfSessionCompat
			}
		case "cloudflare-workers-ai":
			m.CompatJSON = cfSessionCompat
		case "kimi-coding":
			m.HeadersJSON = `{"User-Agent":"KimiCLI/1.5"}`
		}
	}

	// Apply thinking level metadata to ALL models
	for i := range allModels {
		applyThinkingLevelMetadata(&allModels[i])
	}

	if len(allModels) == 0 {
		fmt.Fprintln(os.Stderr, "Error: no models fetched")
		os.Exit(1)
	}

	byProvider := groupByProvider(allModels)
	generateGoFile(byProvider, len(allModels))
}

func fetchModelsDev() (ModelsDevResponse, error) {
	var data ModelsDevResponse
	err := fetchJSON("https://models.dev/api.json", &data)
	return data, err
}

func fetchOpenRouter() ([]Model, error) {
	var data OpenRouterResponse
	if err := fetchJSON("https://openrouter.ai/api/v1/models", &data); err != nil {
		return nil, err
	}
	var models []Model
	for _, m := range data.Data {
		hasTools := false
		hasReasoning := false
		for _, p := range m.SupportedParameters {
			if p == "tools" {
				hasTools = true
			}
			if p == "reasoning" {
				hasReasoning = true
			}
		}
		if !hasTools {
			continue
		}

		input := []string{"text"}
		if m.Architecture != nil && len(m.Architecture.Modality) > 0 {
			var modalities []string
			if json.Unmarshal(m.Architecture.Modality, &modalities) == nil {
				for _, mod := range modalities {
					if mod == "image" {
						input = append(input, "image")
						break
					}
				}
			} else {
				var s string
				if json.Unmarshal(m.Architecture.Modality, &s) == nil && strings.Contains(s, "image") {
					input = append(input, "image")
				}
			}
		}

		ctx := m.ContextLength
		if ctx <= 0 {
			ctx = 4096
		}
		mt := 4096
		if m.TopProvider != nil && m.TopProvider.MaxCompletionTokens > 0 {
			mt = m.TopProvider.MaxCompletionTokens
		}

		cIn, cOut := 0.0, 0.0
		if m.Pricing != nil {
			fmt.Sscanf(m.Pricing.Prompt, "%f", &cIn)
			fmt.Sscanf(m.Pricing.Completion, "%f", &cOut)
			cIn *= 1_000_000
			cOut *= 1_000_000
		}

		models = append(models, Model{
			ID: m.ID, Name: m.Name, API: "openai-completions",
			Provider: "openrouter", BaseURL: "https://openrouter.ai/api/v1",
			Reasoning: hasReasoning, Input: input,
			ContextWindow: ctx, MaxTokens: mt, CostInput: cIn, CostOutput: cOut,
		})
	}
	return models, nil
}

func fetchAiGateway() ([]Model, error) {
	var data AiGatewayResponse
	if err := fetchJSON("https://ai-gateway.vercel.sh/v1/models", &data); err != nil {
		return nil, err
	}
	var models []Model
	for _, m := range data.Data {
		hasTools, hasVision, hasReasoning := false, false, false
		for _, tag := range m.Tags {
			switch tag {
			case "tool-use":
				hasTools = true
			case "vision":
				hasVision = true
			case "reasoning":
				hasReasoning = true
			}
		}
		if !hasTools {
			continue
		}
		input := []string{"text"}
		if hasVision {
			input = append(input, "image")
		}
		ctx := m.ContextWindow
		if ctx <= 0 {
			ctx = 4096
		}
		mt := m.MaxTokens
		if mt <= 0 {
			mt = 4096
		}
		cIn, cOut := 0.0, 0.0
		if m.Pricing != nil {
			cIn = toFloat(m.Pricing.Input) * 1_000_000
			cOut = toFloat(m.Pricing.Output) * 1_000_000
		}
		models = append(models, Model{
			ID: m.ID, Name: m.Name, API: "anthropic-messages",
			Provider: "vercel-ai-gateway", BaseURL: "https://ai-gateway.vercel.sh",
			Reasoning: hasReasoning, Input: input,
			ContextWindow: ctx, MaxTokens: mt, CostInput: cIn, CostOutput: cOut,
		})
	}
	return models, nil
}

func mirrorAzureModels(all []Model) []Model {
	var azure []Model
	for _, m := range all {
		if m.Provider == "openai" && strings.HasPrefix(m.API, "openai-") {
			// pi mirrors all Azure models even without cost data
			azure = append(azure, Model{
				ID: m.ID, Name: m.Name, API: "azure-openai-responses",
				Provider: "azure-openai-responses",
				BaseURL: "https://YOUR_RESOURCE.openai.azure.com/openai/v1",
				Reasoning: m.Reasoning, Input: m.Input,
				ContextWindow: m.ContextWindow, MaxTokens: m.MaxTokens,
				CostInput: m.CostInput, CostOutput: m.CostOutput,
			})
		}
	}
	return azure
}

// opencodeModels returns opencode / opencode-go models.
// pi fetches these from the OpenCode API, but the model IDs are stable.
func opencodeModels() []Model {
	return []Model{
		// opencode
		{ID: "kimi-k2.6", Provider: "opencode", ContextWindow: 128000, MaxTokens: 65535, Reasoning: false,
			Input: []string{"text"}, CostInput: 0, CostOutput: 0,
			API: "openai-completions", BaseURL: "https://opencode.ai/zen"},
		{ID: "deepseek-v4-pro", Provider: "opencode", ContextWindow: 1000000, MaxTokens: 65535, Reasoning: true,
			Input: []string{"text", "image"}, CostInput: 0, CostOutput: 0,
			API: "openai-completions", BaseURL: "https://opencode.ai/zen"},
		// opencode-go
		{ID: "kimi-k2.6", Provider: "opencode-go", ContextWindow: 128000, MaxTokens: 65535, Reasoning: false,
			Input: []string{"text"}, CostInput: 0, CostOutput: 0,
			API: "openai-completions", BaseURL: "https://opencode.ai/zen/go"},
		{ID: "deepseek-v4-pro", Provider: "opencode-go", ContextWindow: 1000000, MaxTokens: 65535, Reasoning: true,
			Input: []string{"text", "image"}, CostInput: 0, CostOutput: 0,
			API: "openai-completions", BaseURL: "https://opencode.ai/zen/go"},
		// openai-codex (also static)
		{ID: "gpt-5.5", Provider: "openai-codex", ContextWindow: 272000, MaxTokens: 128000, Reasoning: true,
			Input: []string{"text", "image"}, CostInput: 0, CostOutput: 0,
			API: "openai-completions", BaseURL: "https://api.openai.com/v1"},
	}
}

// processModelsDev processes all providers from models.dev.
// Provider list and configurations mirror pi's generate-models.ts exactly.
func processModelsDev(data ModelsDevResponse) []Model {
	var models []Model

	// ── Standard providers (OpenAI-compatible) ────────────────────────────
	standardProviders := map[string]providerConfig{
		"openai":             {"openai-completions", "https://api.openai.com/v1"},
		"google":             {"openai-completions", "https://generativelanguage.googleapis.com/v1beta/openai"},
		"google-vertex":      {"openai-completions", "https://LOCATION-aiplatform.googleapis.com/v1beta1/projects/PROJECT_ID/locations/LOCATION/endpoints/openapi"},
		"deepseek":           {"openai-completions", "https://api.deepseek.com/v1"},
		"groq":               {"openai-completions", "https://api.groq.com/openai/v1"},
		"cerebras":           {"openai-completions", "https://api.cerebras.ai/v1"},
		"mistral":            {"openai-completions", "https://api.mistral.ai/v1"},
		"xai":                {"openai-completions", "https://api.x.ai/v1"},
		"zhipuai":            {"openai-completions", "https://open.bigmodel.cn/api/paas/v4"},
		"zai":                {"openai-completions", "https://api.z.ai/api/paas/v4"},
		"moonshotai":         {"openai-completions", "https://api.moonshot.ai/v1"},
		"moonshotai-cn":      {"openai-completions", "https://api.moonshot.cn/v1"},
		"fireworks":          {"openai-completions", "https://api.fireworks.ai/inference/v1"},
		"together":           {"openai-completions", "https://api.together.xyz/v1"},
		"huggingface":        {"openai-completions", "https://api-inference.huggingface.co/v1"},
		"cloudflare-workers-ai": {"openai-completions", "https://api.cloudflare.com/client/v4/accounts"},
	}
	for provider, cfg := range standardProviders {
		models = append(models, processProvider(data, provider, cfg.api, cfg.baseURL, provider)...)
	}

	// ── Anthropic (native API) ────────────────────────────────────────────
	models = append(models, processProvider(data, "anthropic", "anthropic-messages", "https://api.anthropic.com/v1", "anthropic")...)

	// ── Amazon Bedrock ────────────────────────────────────────────────────
	models = append(models, processBedrock(data)...)

	// ── Anthropic-compatible providers ────────────────────────────────────
	// MiniMax (anthropic-messages API, pi uses same)
	models = append(models, processProvider(data, "minimax", "anthropic-messages", "https://api.minimax.io/anthropic", "minimax")...)
	models = append(models, processProvider(data, "minimax-cn", "anthropic-messages", "https://api.minimaxi.com/anthropic", "minimax-cn")...)

	// Kimi For Coding (anthropic-messages API)
	models = append(models, processKimiCoding(data)...)

	// Xiaomi + token-plan variants (anthropic-messages API)
	xiaomiVariants := []struct{ key, provider, baseURL string }{
		{"xiaomi", "xiaomi", "https://api.xiaomimimo.com/anthropic"},
		{"xiaomi-token-plan-cn", "xiaomi-token-plan-cn", "https://token-plan-cn.xiaomimimo.com/anthropic"},
		{"xiaomi-token-plan-ams", "xiaomi-token-plan-ams", "https://token-plan-ams.xiaomimimo.com/anthropic"},
		{"xiaomi-token-plan-sgp", "xiaomi-token-plan-sgp", "https://token-plan-sgp.xiaomimimo.com/anthropic"},
	}
	for _, v := range xiaomiVariants {
		models = append(models, processProvider(data, v.key, "anthropic-messages", v.baseURL, v.provider)...)
	}

	// ── GitHub Copilot (mixed APIs based on model) ────────────────────────
	models = append(models, processGitHubCopilot(data)...)

	return models
}

func processProvider(data ModelsDevResponse, key, api, baseURL, provider string) []Model {
	providerData, ok := data[key]
	if !ok {
		return nil
	}
	var models []Model
	for id, m := range providerData.Models {
		if !m.toolCall() {
			continue
		}
		// Bedrock exclusions (pi skips these)
		if provider == "amazon-bedrock" {
			if strings.HasPrefix(id, "ai21.jamba") || strings.HasPrefix(id, "mistral.mistral-7b-instruct-v0") {
				continue
			}
		}
		input := []string{"text"}
		if m.hasImageInput() {
			input = append(input, "image")
		}
		models = append(models, Model{
			ID: id, Name: m.name(), API: api, Provider: provider, BaseURL: baseURL,
			Reasoning: m.reasoning(), Input: input,
			ContextWindow: m.contextWindow(), MaxTokens: m.maxTokens(),
			CostInput: m.costInput(), CostOutput: m.costOutput(),
			CostCacheRead: m.costCacheRead(), CostCacheWrite: m.costCacheWrite(),
		})
	}
	return models
}

func processBedrock(data ModelsDevResponse) []Model {
	providerData, ok := data["amazon-bedrock"]
	if !ok {
		return nil
	}
	var models []Model
	for id, m := range providerData.Models {
		if !m.toolCall() {
			continue
		}
		if strings.HasPrefix(id, "ai21.jamba") || strings.HasPrefix(id, "mistral.mistral-7b-instruct-v0") {
			continue
		}
		baseURL := "https://bedrock-runtime.us-east-1.amazonaws.com"
		if strings.HasPrefix(id, "eu.") {
			baseURL = "https://bedrock-runtime.eu-central-1.amazonaws.com"
		}
		input := []string{"text"}
		if m.hasImageInput() {
			input = append(input, "image")
		}
		models = append(models, Model{
			ID: id, Name: m.name(), API: "bedrock-converse-stream", Provider: "amazon-bedrock", BaseURL: baseURL,
			Reasoning: m.reasoning(), Input: input,
			ContextWindow: m.contextWindow(), MaxTokens: m.maxTokens(),
			CostInput: m.costInput(), CostOutput: m.costOutput(),
			CostCacheRead: m.costCacheRead(), CostCacheWrite: m.costCacheWrite(),
		})
	}
	return models
}

func processKimiCoding(data ModelsDevResponse) []Model {
	providerData, ok := data["kimi-for-coding"]
	if !ok {
		return nil
	}
	canonicalExists := false
	if _, exists := providerData.Models["kimi-for-coding"]; exists {
		canonicalExists = true
	}
	aliasSet := map[string]bool{"k2p5": true, "k2p6": true}

	var models []Model
	for id, m := range providerData.Models {
		if !m.toolCall() {
			continue
		}
		// Normalize aliases to canonical, drop duplicates when canonical exists
		if aliasSet[id] && canonicalExists {
			continue
		}
		normalizedID := id
		normalizedName := m.name()
		if aliasSet[id] {
			normalizedID = "kimi-for-coding"
			normalizedName = "Kimi For Coding"
		}
		models = append(models, Model{
			ID: normalizedID, Name: normalizedName, API: "anthropic-messages",
			Provider: "kimi-coding", BaseURL: "https://api.kimi.com/coding",
			Reasoning: m.reasoning(), Input: []string{"text"},
			ContextWindow: m.contextWindow(), MaxTokens: m.maxTokens(),
			CostInput: m.costInput(), CostOutput: m.costOutput(),
			CostCacheRead: m.costCacheRead(), CostCacheWrite: m.costCacheWrite(),
		})
	}
	return models
}

func processGitHubCopilot(data ModelsDevResponse) []Model {
	providerData, ok := data["github-copilot"]
	if !ok {
		return nil
	}
	var models []Model
	for id, m := range providerData.Models {
		if !m.toolCall() {
			continue
		}
		// pi determines API per model: anthropic models use anthropic-messages, others use openai-completions
		api := "openai-completions"
		if strings.Contains(strings.ToLower(id), "claude") {
			api = "anthropic-messages"
		}
		input := []string{"text"}
		if m.hasImageInput() {
			input = append(input, "image")
		}
		models = append(models, Model{
			ID: id, Name: m.name(), API: api, Provider: "github-copilot",
			BaseURL: "https://models.githubcopilot.com/v1",
			Reasoning: m.reasoning(), Input: input,
			ContextWindow: m.contextWindow(), MaxTokens: m.maxTokens(),
			CostInput: m.costInput(), CostOutput: m.costOutput(),
			CostCacheRead: m.costCacheRead(), CostCacheWrite: m.costCacheWrite(),
		})
	}
	return models
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

func fetchJSON(url string, target interface{}) error {
	client := &http.Client{Timeout: 30 * time.Second}
	resp, err := client.Get(url)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("HTTP %d", resp.StatusCode)
	}
	return json.NewDecoder(resp.Body).Decode(target)
}

func toFloat(v interface{}) float64 {
	switch x := v.(type) {
	case float64:
		return x
	case string:
		var f float64
		fmt.Sscanf(x, "%f", &f)
		return f
	default:
		return 0
	}
}

func groupByProvider(all []Model) map[string][]Model {
	byProvider := make(map[string][]Model)
	for _, m := range all {
		byProvider[m.Provider] = append(byProvider[m.Provider], m)
	}
	return byProvider
}

func generateGoFile(byProvider map[string][]Model, total int) {
	var b strings.Builder
	b.WriteString("// Code generated by generate_models.go. DO NOT EDIT.\n")
	b.WriteString(fmt.Sprintf("// Generated at %s\n", time.Now().Format(time.RFC3339)))
	b.WriteString(fmt.Sprintf("// %d models across %d providers\n", total, len(byProvider)))
	b.WriteString("\npackage modelregistry\n\n")
	b.WriteString("import (\n")
	b.WriteString("\t\"encoding/json\"\n\n")
	b.WriteString("\t\"github.com/huichen/xihu/pkg/types\"\n")
	b.WriteString(")\n\n")
	b.WriteString("// InitBuiltinModels returns the complete built-in model catalog.\n")
	b.WriteString("func InitBuiltinModels() []types.Model {\n")
	b.WriteString("\tmk := func(id, name, provider string, ctx, max int, reasoning bool, input []string, inCost, outCost, cacheRead, cacheWrite float64, compatJSON, tlmJSON, headersJSON, api, baseURL string) types.Model {\n")
	b.WriteString("\t\tm := types.Model{\n")
	b.WriteString("\t\t\tID: id, Name: name, Provider: provider, ContextWindow: ctx, MaxTokens: max,\n")
	b.WriteString("\t\t\tReasoning: reasoning, InputTypes: input, API: api, BaseURL: baseURL,\n")
	b.WriteString("\t\t}\n")
	b.WriteString("\t\tm.Cost.Input = inCost\n")
	b.WriteString("\t\tm.Cost.Output = outCost\n")
	b.WriteString("\t\tm.Cost.CacheRead = cacheRead\n")
	b.WriteString("\t\tm.Cost.CacheWrite = cacheWrite\n")
	b.WriteString("\t\tif compatJSON != \"\" {\n")
	b.WriteString("\t\t\tjson.Unmarshal([]byte(compatJSON), &m.Compat)\n")
	b.WriteString("\t\t}\n")
	b.WriteString("\t\tif tlmJSON != \"\" {\n")
	b.WriteString("\t\t\tjson.Unmarshal([]byte(tlmJSON), &m.ThinkingLevelMap)\n")
	b.WriteString("\t\t}\n")
	b.WriteString("\t\tif headersJSON != \"\" {\n")
	b.WriteString("\t\t\tjson.Unmarshal([]byte(headersJSON), &m.Headers)\n")
	b.WriteString("\t\t}\n")
	b.WriteString("\t\treturn m\n")
	b.WriteString("\t}\n")
	b.WriteString("\treturn []types.Model{\n")

	providers := make([]string, 0, len(byProvider))
	for p := range byProvider {
		providers = append(providers, p)
	}
	sort.Strings(providers)

	for _, provider := range providers {
		models := byProvider[provider]
		b.WriteString(fmt.Sprintf("\t\t// ── %s (%d models) ──\n", provider, len(models)))
		sort.Slice(models, func(i, j int) bool { return models[i].ID < models[j].ID })
		for _, m := range models {
			eid := strings.ReplaceAll(strings.ReplaceAll(m.ID, "\\", "\\\\"), "\"", "\\\"")
			ename := strings.ReplaceAll(strings.ReplaceAll(m.Name, "\\", "\\\\"), "\"", "\\\"")
			inputStr := "[]string{" + strings.Join(quoteStrings(m.Input), ", ") + "}"
			cIn := fmt.Sprintf("%.2f", m.CostInput)
			cOut := fmt.Sprintf("%.2f", m.CostOutput)
			cCR := fmt.Sprintf("%.2f", m.CostCacheRead)
			cCW := fmt.Sprintf("%.2f", m.CostCacheWrite)
			eCompat := strings.ReplaceAll(strings.ReplaceAll(m.CompatJSON, "\\", "\\\\"), "\"", "\\\"")
			eTLM := strings.ReplaceAll(strings.ReplaceAll(m.TLMJSON, "\\", "\\\\"), "\"", "\\\"")
			eHeaders := strings.ReplaceAll(strings.ReplaceAll(m.HeadersJSON, "\\", "\\\\"), "\"", "\\\"")
			b.WriteString(fmt.Sprintf("\t\tmk(\"%s\", \"%s\", \"%s\", %d, %d, %t, %s, %s, %s, %s, %s, \"%s\", \"%s\", \"%s\", \"%s\", \"%s\"),\n",
				eid, ename, m.Provider, m.ContextWindow, m.MaxTokens, m.Reasoning, inputStr, cIn, cOut, cCR, cCW, eCompat, eTLM, eHeaders, m.API, m.BaseURL))
		}
	}
	b.WriteString("\t}\n}\n")

	if err := os.WriteFile("models_generated.go", []byte(b.String()), 0644); err != nil {
		fmt.Fprintf(os.Stderr, "Error: %v\n", err)
		os.Exit(1)
	}
	fmt.Printf("Generated models_generated.go: %d models across %d providers\n", total, len(byProvider))
}

func quoteStrings(ss []string) []string {
	qs := make([]string, len(ss))
	for i, s := range ss {
		qs[i] = fmt.Sprintf("\"%s\"", s)
	}
	return qs
}
