package components

import (

)

func (c *ChatViewport) AddToolCall(id, name, args string) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.entries = append(c.entries, ChatEntry{
		Type:     "tool_call",
		ID:       id,
		ToolName: name,
		ToolArgs: args,
		Expanded: false,
	})
	if c.vp.AtBottom() {
		c.autoScroll = true
	}

}

// compactReadFileNames are files that get compact rendering when read (TS pi-mono: COMPACT_RESOURCE_FILE_NAMES).
var compactReadFileNames = map[string]bool{
	"AGENTS.md":  true,
	"AGENTS.MD":  true,
	"CLAUDE.md":  true,
	"CLAUDE.MD":  true,
}

// classifyCompactRead determines if a read tool target should get compact rendering.
// Returns (kind, label). Empty kind means no compact rendering (TS pi-mono: getCompactReadClassification).
func (c *ChatViewport) CompleteToolCall(id, args string) {
	c.mu.Lock()
	defer c.mu.Unlock()
	for i := len(c.entries) - 1; i >= 0; i-- {
		if c.entries[i].ID == id && c.entries[i].Type == "tool_call" {
			c.entries[i].ToolArgs = args
			return
		}
	}
	// Fallback: create new entry
	c.entries = append(c.entries, ChatEntry{
		Type:     "tool_call",
		ID:       id,
		ToolArgs: args,
		Expanded: false,
	})
}

// RemovePendingToolCall removes the pending tool_call entry by ID (used when bash replaces it).
func (c *ChatViewport) RemovePendingToolCall(id string) {
	c.mu.Lock()
	defer c.mu.Unlock()
	for i := len(c.entries) - 1; i >= 0; i-- {
		if c.entries[i].ID == id && c.entries[i].Type == "tool_call" {
			c.entries = append(c.entries[:i], c.entries[i+1:]...)
			return
		}
	}
}

// UpdateToolResult attaches the result to an existing tool call entry.
func (c *ChatViewport) UpdateToolResult(id, output string, isError bool) {
	c.mu.Lock()
	defer c.mu.Unlock()
	for i := range c.entries {
		if c.entries[i].ID == id && c.entries[i].Type == "tool_call" {
			c.entries[i].Content = output
			c.entries[i].IsError = isError
			c.entries[i].Type = "tool_result"
			return
		}
	}
}

// MarkLastStopReason sets the stop reason on the most recent text entry (TS pi-mono: stopReason).
func (c *ChatViewport) MarkLastStopReason(reason string, errorMessage ...string) {
	c.mu.Lock()
	defer c.mu.Unlock()
	for i := len(c.entries) - 1; i >= 0; i-- {
		if c.entries[i].Type == "text" {
			c.entries[i].StopReason = reason
			if len(errorMessage) > 0 {
				c.entries[i].ErrorMessage = errorMessage[0]
			}
			return
		}
	}
}

// AppendImageBlock adds an image to the most recent tool_result entry (TS pi-mono).
func (c *ChatViewport) MarkToolRunning(id string) {
	c.mu.Lock()
	defer c.mu.Unlock()
	for i := range c.entries {
		if c.entries[i].ID == id && c.entries[i].Type == "tool_call" {
			c.entries[i].ToolPending = true
			return
		}
	}
}

// SetToolToggleKey sets the key string shown in tool expand/collapse hints.
func (c *ChatViewport) SetToolDuration(id, duration string) {
	c.mu.Lock()
	defer c.mu.Unlock()
	for i := range c.entries {
		if c.entries[i].ID == id {
			c.entries[i].ToolDuration = duration
			return
		}
	}
}

// MarkToolRunning marks a pending tool call as actively executing.
func (c *ChatViewport) ToggleAllTools() {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.AllToolsExpanded = !c.AllToolsExpanded
	for i := range c.entries {
		e := &c.entries[i]
		if e.Type == "tool_result" || e.Type == "bash" || e.Type == "tool_call" || e.Type == "custom_message" {
			e.Expanded = c.AllToolsExpanded
		}
	}
}

// ToggleExpandLastTool toggles the Expanded state on the last tool_result or bash entry.
func (c *ChatViewport) ToggleExpandLastTool() {
	c.mu.Lock()
	defer c.mu.Unlock()
	for i := len(c.entries) - 1; i >= 0; i-- {
		e := &c.entries[i]
		if e.Type == "tool_result" || e.Type == "bash" || e.Type == "tool_call" || e.Type == "custom_message" {
			e.Expanded = !e.Expanded
			return
		}
	}
}

// AppendToolCallDelta appends streaming arguments to the last tool call entry.
func (c *ChatViewport) AppendToolCallDelta(text string) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if len(c.entries) > 0 {
		last := &c.entries[len(c.entries)-1]
		if last.Type == "tool_call" {
			last.ToolArgs += text
			if c.vp.AtBottom() {
				c.autoScroll = true
			}
			return
		}
	}
}

// AddBashExecution starts a new bash execution entry with bordered display.
// Returns the entry index for later updates.
func (c *ChatViewport) HasRunningTools() bool {
	c.mu.Lock()
	defer c.mu.Unlock()
	for i := len(c.entries) - 1; i >= 0; i-- {
		if c.entries[i].ToolPending || c.entries[i].BashRunning {
			return true
		}
	}
	return false
}

// View renders the chat viewport by delegating to per-message-type components,
// matching TS pi-mono's pattern of independent component classes.
