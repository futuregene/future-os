package llm

import (
	"strings"
	"time"
)

// StreamOptions holds optional configuration for streaming requests.
// Both Client and AnthropicClient embed this for per-request overrides.
type StreamOptions struct {
	// Timeout is the HTTP client timeout for each attempt.
	// Default: 120s.
	Timeout time.Duration

	// MaxRetries is the maximum number of retries on transient failures.
	// 0 or negative means no retries. Default: 3.
	MaxRetries int

	// RetryDelay is the initial delay before the first retry. Subsequent
	// retries use exponential backoff (delay * 2^attempt).
	// Default: 2s.
	RetryDelay time.Duration

	// APIKey overrides the client-level API key for this request.
	// If empty, the client-level key is used.
	APIKey string

	// Headers are custom HTTP headers added to the request.
	Headers map[string]string
}

// DefaultStreamOptions returns sensible defaults for streaming.
func DefaultStreamOptions() *StreamOptions {
	return &StreamOptions{
		Timeout:    120 * time.Second,
		MaxRetries: 3,
		RetryDelay: 2 * time.Second,
	}
}

// merge returns effective options by merging with defaults.
// Nil receiver is treated as defaults.
func (o *StreamOptions) merge() *StreamOptions {
	if o == nil {
		return DefaultStreamOptions()
	}
	merged := *o
	if merged.Timeout <= 0 {
		merged.Timeout = 120 * time.Second
	}
	if merged.MaxRetries < 0 {
		merged.MaxRetries = 0
	}
	if merged.RetryDelay <= 0 {
		merged.RetryDelay = 2 * time.Second
	}
	return &merged
}

// EstimateCost estimates the cost of an LLM API call in USD.
//
// Pricing (per 1M tokens):
//   - gpt-4o:           $2.50 input / $10.00 output
//   - gpt-4o-mini:      $0.15 input / $0.60  output
//   - claude-sonnet:    $3.00 input / $15.00 output
//   - claude-haiku:     $0.25 input / $1.25  output
//   - deepseek-chat:    $0.14 input / $0.28  output
//   - deepseek-reasoner:$0.55 input / $2.19  output
//
// Returns 0 if the model is not recognized.
func EstimateCost(model string, inputTokens, outputTokens int) float64 {
	model = strings.ToLower(model)

	type price struct{ input, output float64 }
	prices := map[string]price{
		"gpt-4o":              {2.50, 10.00},
		"gpt-4o-mini":         {0.15, 0.60},
		"claude-sonnet":       {3.00, 15.00},
		"claude-sonnet-4":     {3.00, 15.00},
		"claude-3-5-sonnet":   {3.00, 15.00},
		"claude-3.5-sonnet":   {3.00, 15.00},
		"claude-haiku":        {0.25, 1.25},
		"claude-3-5-haiku":    {0.25, 1.25},
		"claude-3.5-haiku":    {0.25, 1.25},
		"deepseek-chat":       {0.14, 0.28},
		"deepseek-reasoner":   {0.55, 2.19},
		"deepseek-v3":         {0.14, 0.28},
	}

	// Try exact match first
	if p, ok := prices[model]; ok {
		return (float64(inputTokens)/1_000_000)*p.input + (float64(outputTokens)/1_000_000)*p.output
	}

	// Try prefix match
	for prefix, p := range prices {
		if strings.HasPrefix(model, prefix) {
			return (float64(inputTokens)/1_000_000)*p.input + (float64(outputTokens)/1_000_000)*p.output
		}
	}

	return 0
}
