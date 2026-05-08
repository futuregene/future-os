package tui

import (
	"encoding/json"
	"strings"
)

// StreamParser incrementally parses LLM streaming output into structured messages.
// It detects thinking blocks (<thinking>...</thinking>), tool calls (JSON function_call),
// and plain text, emitting the appropriate Bubble Tea messages.
type StreamParser struct {
	buffer       strings.Builder
	inThinking   bool
	thinkingOpen string // "<thinking>" or similar
}

// NewStreamParser creates a new stream parser.
func NewStreamParser() *StreamParser {
	return &StreamParser{
		thinkingOpen: "<thinking>",
	}
}

// Feed processes a chunk of text and returns any new messages to send.
func (p *StreamParser) Feed(chunk string) []interface{} {
	var msgs []interface{}

	for i := 0; i < len(chunk); i++ {
		ch := chunk[i]

		if p.inThinking {
			p.buffer.WriteByte(ch)
			// Check for closing tag
			content := p.buffer.String()
			if idx := strings.Index(content, "</thinking>"); idx >= 0 {
				// Found end of thinking block
				thinking := content[:idx]
				if thinking != "" {
					msgs = append(msgs, ThinkingMsg(thinking))
				}
				rest := content[idx+len("</thinking>"):]
				p.buffer.Reset()
				p.inThinking = false
				if rest != "" {
					msgs = append(msgs, StreamTextMsg(rest))
				}
			}
		} else {
			p.buffer.WriteByte(ch)
			content := p.buffer.String()

			// Check for thinking tag start
			if idx := strings.Index(content, p.thinkingOpen); idx >= 0 {
				// Flush text before the tag
				if idx > 0 {
					msgs = append(msgs, StreamTextMsg(content[:idx]))
				}
				rest := content[idx+len(p.thinkingOpen):]
				p.buffer.Reset()
				p.buffer.WriteString(rest)
				p.inThinking = true
				continue
			}

			// Check for tool calls (JSON with "tool_calls")
			if strings.Contains(content, `"tool_calls"`) {
				// Try to parse as a complete JSON object
				if tc := p.tryParseToolCall(content); tc != nil {
					msgs = append(msgs, tc)
					p.buffer.Reset()
					continue
				}
			}

			// Flush accumulated text periodically (every 80 chars or on newline)
			if len(content) >= 80 || ch == '\n' {
				if content != "" {
					msgs = append(msgs, StreamTextMsg(string(content)))
					p.buffer.Reset()
				}
			}
		}
	}

	return msgs
}

// Flush emits any remaining buffered content.
func (p *StreamParser) Flush() []interface{} {
	var msgs []interface{}
	if p.buffer.Len() > 0 {
		content := p.buffer.String()
		if p.inThinking {
			msgs = append(msgs, ThinkingMsg(content))
		} else {
			msgs = append(msgs, StreamTextMsg(content))
		}
		p.buffer.Reset()
	}
	p.inThinking = false
	return msgs
}

// tryParseToolCall attempts to parse a JSON tool_calls object from the buffer.
func (p *StreamParser) tryParseToolCall(text string) interface{} {
	// Look for the delta format: {"choices":[{"delta":{"tool_calls":[...]}}]}
	type Delta struct {
		ToolCalls []struct {
			Index    int    `json:"index"`
			ID       string `json:"id"`
			Type     string `json:"type"`
			Function struct {
				Name      string `json:"name"`
				Arguments string `json:"arguments"`
			} `json:"function"`
		} `json:"tool_calls"`
	}
	type Choice struct {
		Delta Delta `json:"delta"`
	}
	type Payload struct {
		Choices []Choice `json:"choices"`
	}

	var payload Payload
	if err := json.Unmarshal([]byte(text), &payload); err != nil {
		return nil
	}

	for _, choice := range payload.Choices {
		for _, tc := range choice.Delta.ToolCalls {
			if tc.Function.Name != "" {
				return ToolCallMsg{
					ID:        tc.ID,
					Name:      tc.Function.Name,
					Arguments: tc.Function.Arguments,
				}
			}
		}
	}
	return nil
}

var _ = strings.Index
