package compaction

import (
	"encoding/json"
	"fmt"
	"math"
	"strings"

	"github.com/huichen/xihu/pkg/types"
)

// DefaultCompactionSettings are the same defaults as TypeScript pi.
var DefaultCompactionSettings = CompactionSettings{
	Enabled:         true,
	ReserveTokens:   16384,
	KeepRecentTokens: 20000,
}

type CompactionSettings struct {
	Enabled         bool
	ReserveTokens   int
	KeepRecentTokens int
}

type CompactOptions struct {
	ReserveTokens   int
	KeepRecentTokens int
	Summarizer      func(messages []types.Message) (string, error)
}

type CompactionResult struct {
	Summary          string
	FirstKeptEntryID string
	TokensBefore     int
	ReadFiles        []string
	ModifiedFiles    []string
}

type CutPointResult struct {
	FirstKeptEntryIndex int
	TurnStartIndex      int
	IsSplitTurn         bool
}

// ShouldCompact checks if compaction should trigger.
func ShouldCompact(contextTokens int, contextWindow int, settings CompactionSettings) bool {
	if !settings.Enabled {
		return false
	}
	return contextTokens > contextWindow-settings.ReserveTokens
}

// EstimateTokens estimates tokens for a single message using per-role heuristics.
func EstimateTokens(msg types.Message) int {
	var chars int
	switch msg.Role {
	case "user":
		chars = countContentChars(msg.Content)
	case "assistant":
		chars = countContentChars(msg.Content)
		for _, tc := range msg.ToolCalls {
			chars += len(tc.Function.Name) + len(tc.Function.Arguments)
		}
	case "tool":
		chars = countContentChars(msg.Content)
	default:
		chars = countContentChars(msg.Content)
	}
	return int(math.Ceil(float64(chars) / 4.0))
}

// EstimateContextTokens estimates total tokens from messages using last assistant usage.
func EstimateContextTokens(messages []types.Message) int {
	total := 0
	for _, msg := range messages {
		total += EstimateTokens(msg)
	}
	return total
}

// findValidCutPoints returns indices where it's safe to cut (user/assistant messages, not tool results).
func findValidCutPoints(messages []types.Message, start, end int) []int {
	var points []int
	for i := start; i < end; i++ {
		msg := messages[i]
		switch msg.Role {
		case "user", "assistant":
			if len(msg.ToolCalls) == 0 {
				points = append(points, i)
			}
		case "system":
			// system messages are valid cut points
			points = append(points, i)
		}
		// Don't cut at tool messages
	}
	return points
}

// findTurnStartIndex finds the user message that starts the turn containing the given index.
func findTurnStartIndex(messages []types.Message, msgIndex, startIndex int) int {
	for i := msgIndex; i >= startIndex; i-- {
		if messages[i].Role == "user" {
			return i
		}
	}
	return -1
}

// FindCutPoint finds the cut point that keeps approximately keepRecentTokens.
func FindCutPoint(messages []types.Message, startIndex, endIndex int, keepRecentTokens int) CutPointResult {
	cutPoints := findValidCutPoints(messages, startIndex, endIndex)
	if len(cutPoints) == 0 {
		return CutPointResult{FirstKeptEntryIndex: startIndex, TurnStartIndex: -1, IsSplitTurn: false}
	}

	accumulated := 0
	cutIndex := cutPoints[0]

	for i := endIndex - 1; i >= startIndex; i-- {
		tokens := EstimateTokens(messages[i])
		accumulated += tokens
		if accumulated >= keepRecentTokens {
			for _, cp := range cutPoints {
				if cp >= i {
					cutIndex = cp
					break
				}
			}
			break
		}
	}

	// Check if split turn
	cutMsg := messages[cutIndex]
	isUser := cutMsg.Role == "user"
	turnStart := -1
	if !isUser {
		turnStart = findTurnStartIndex(messages, cutIndex, startIndex)
	}

	return CutPointResult{
		FirstKeptEntryIndex: cutIndex,
		TurnStartIndex:      turnStart,
		IsSplitTurn:         !isUser && turnStart != -1,
	}
}

// Compact performs message compaction. Returns compacted messages and metadata.
func Compact(messages []types.Message, opts CompactOptions) ([]types.Message, *CompactionResult, error) {
	tokensBefore := EstimateContextTokens(messages)
	if opts.ReserveTokens == 0 {
		opts.ReserveTokens = DefaultCompactionSettings.ReserveTokens
	}
	if opts.KeepRecentTokens == 0 {
		opts.KeepRecentTokens = DefaultCompactionSettings.KeepRecentTokens
	}

	// Use token budget from end
	budget := opts.KeepRecentTokens
	if budget > tokensBefore {
		return messages, nil, nil
	}

	cut := FindCutPoint(messages, 0, len(messages), budget)
	if cut.FirstKeptEntryIndex <= 0 {
		return messages, nil, nil
	}

	oldMessages := messages[:cut.FirstKeptEntryIndex]
	recentMessages := messages[cut.FirstKeptEntryIndex:]

	// Extract file operations
	reads, writes := ExtractFileOperations(messages)

	// Summarize old messages
	summary := buildFallbackSummary(reads, writes)
	if opts.Summarizer != nil {
		if s, err := opts.Summarizer(oldMessages); err == nil && s != "" {
			summary = s
		}
	}

	// Build compaction message
	compactionMsg := types.Message{
		Role: "user",
		Content: json.RawMessage(fmt.Sprintf(
			`[{"type":"text","text":%q}]`,
			fmt.Sprintf("[Context compaction: previous conversation summarized. Key files read: %s. Modified: %s]\n\n%s",
				strings.Join(reads, ", "), strings.Join(writes, ", "), summary),
		)),
	}

	result := make([]types.Message, 0, len(recentMessages)+1)
	result = append(result, compactionMsg)
	result = append(result, recentMessages...)

	return result, &CompactionResult{
		Summary:      summary,
		TokensBefore: tokensBefore,
		ReadFiles:    reads,
		ModifiedFiles: writes,
	}, nil
}

// ExtractFileOperations scans messages for file operations from tool calls.
func ExtractFileOperations(messages []types.Message) (reads []string, writes []string) {
	readSet := make(map[string]bool)
	writeSet := make(map[string]bool)

	for _, msg := range messages {
		if msg.Role != "assistant" {
			continue
		}
		for _, tc := range msg.ToolCalls {
			var args struct {
				FilePath string `json:"file_path"`
				Path     string `json:"path"`
			}
			json.Unmarshal(tc.Function.Arguments, &args)
			fp := args.FilePath
			if fp == "" {
				fp = args.Path
			}
			if fp == "" {
				continue
			}

			switch tc.Function.Name {
			case "read", "read_file":
				readSet[fp] = true
			case "write", "write_file", "edit", "patch":
				writeSet[fp] = true
			}
		}
	}

	for f := range readSet {
		reads = append(reads, f)
	}
	for f := range writeSet {
		writes = append(writes, f)
	}
	return
}

// FormatFileOpsForSummary formats file operations as bullet list.
func FormatFileOpsForSummary(reads, writes []string) string {
	var sb strings.Builder
	if len(reads) > 0 {
		sb.WriteString("Files read:\n")
		for _, f := range reads {
			sb.WriteString(fmt.Sprintf("  - %s\n", f))
		}
	}
	if len(writes) > 0 {
		sb.WriteString("Files modified:\n")
		for _, f := range writes {
			sb.WriteString(fmt.Sprintf("  - %s\n", f))
		}
	}
	return sb.String()
}

// SummarizationPrompt returns the structured summarization prompt (matching TypeScript).
const SummarizationPrompt = `The messages above are a conversation to summarize. Create a structured context checkpoint summary that another LLM will use to continue the work.

Use this EXACT format:

## Goal
[What is the user trying to accomplish?]

## Constraints & Preferences
- [constraints or "(none)"]

## Progress
### Done
- [x] [Completed tasks]

### In Progress
- [ ] [Current work]

### Blocked
- [Issues, if any]

## Key Decisions
- **[Decision]**: [Brief rationale]

## Next Steps
1. [Ordered list]

## Critical Context
- [Data, paths, errors to preserve]
- [Or "(none)"]`

const UpdateSummarizationPrompt = `The messages above are NEW conversation messages to incorporate into the existing summary provided in <previous-summary> tags.

Update the existing summary. RULES:
- PRESERVE all existing information
- ADD new progress, decisions, context
- UPDATE Progress section
- UPDATE Next Steps
- PRESERVE exact file paths, function names, error messages`

func buildFallbackSummary(reads, writes []string) string {
	return fmt.Sprintf("Previous conversation summarized. %s", FormatFileOpsForSummary(reads, writes))
}

func countContentChars(content json.RawMessage) int {
	var blocks []types.TextContent
	if err := json.Unmarshal(content, &blocks); err == nil {
		chars := 0
		for _, b := range blocks {
			chars += len(b.Text)
		}
		return chars
	}
	var s string
	if err := json.Unmarshal(content, &s); err == nil {
		return len(s)
	}
	return len(content)
}
