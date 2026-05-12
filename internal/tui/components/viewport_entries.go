package components

import (

)

func (c *ChatViewport) AppendText(text string) {
	c.mu.Lock()
	defer c.mu.Unlock()

	if len(c.entries) > 0 && c.entries[len(c.entries)-1].Type == "text" {
		c.entries[len(c.entries)-1].Content += text
		if c.vp.AtBottom() {
			c.autoScroll = true
		}
		return
	}
	c.entries = append(c.entries, ChatEntry{
		Type:    "text",
		Content: text,
	})
	if c.vp.AtBottom() {
		c.autoScroll = true
	}
}

// AppendThinking adds a thinking chunk.
// Searches backwards for an existing thinking entry within the current turn,
// stopping at a user_message boundary. This consolidates interleaved
// thinking/text chunks while keeping separate turns distinct.
func (c *ChatViewport) AppendThinking(text string) {
	c.mu.Lock()
	defer c.mu.Unlock()

	// Search backwards for the last thinking entry, stop at turn boundary
	for i := len(c.entries) - 1; i >= 0; i-- {
		if c.entries[i].Type == "user_message" || c.entries[i].Type == "tool_result" {
			break // New turn boundary, stop searching
		}
		if c.entries[i].Type == "thinking" {
			c.entries[i].Content += text
			if c.vp.AtBottom() {
				c.autoScroll = true
			}
			return
		}
	}
	// No existing thinking entry in this turn — create one
	c.entries = append(c.entries, ChatEntry{
		Type:     "thinking",
		Content:  text,
		Expanded: true,
	})
	if c.vp.AtBottom() {
		c.autoScroll = true
	}
}

// AddToolCall records a new tool call.
func (c *ChatViewport) AppendImageBlock(base64Data, mimeType string) {
	c.mu.Lock()
	defer c.mu.Unlock()
	for i := len(c.entries) - 1; i >= 0; i-- {
		if c.entries[i].Type == "tool_result" {
			c.entries[i].ImageBlocks = append(c.entries[i].ImageBlocks, ImageBlock{
				Base64Data: base64Data,
				MimeType:   mimeType,
			})
			return
		}
	}
}

// SetLastBashDuration sets the duration on the last bash entry (index-based).
func (c *ChatViewport) AppendError(msg string) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.entries = append(c.entries, ChatEntry{
		Type:    "error",
		Content: msg,
	})
}

// AppendWarning adds a warning message (yellow/orange, distinct from errors).
func (c *ChatViewport) AppendWarning(msg string) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.entries = append(c.entries, ChatEntry{
		Type:    "warning",
		Content: msg,
	})
}

// AppendSystem adds a system message.
func (c *ChatViewport) AppendSystem(msg string) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.entries = append(c.entries, ChatEntry{
		Type:    "system",
		Content: msg,
	})
}

// ReplaceLastSystem replaces the last system message's content if the last entry
// is a system message. Returns true if replaced, false otherwise.
// Used for deduping consecutive status messages (e.g. rapid model cycling).
func (c *ChatViewport) ReplaceLastSystem(msg string) bool {
	c.mu.Lock()
	defer c.mu.Unlock()
	if len(c.entries) == 0 {
		return false
	}
	last := &c.entries[len(c.entries)-1]
	if last.Type != "system" {
		return false
	}
	last.Content = msg
	return true
}

// Clear resets all entries and viewport content.
func (c *ChatViewport) AppendUserMessage(contentText string) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.entries = append(c.entries, ChatEntry{
		Type:    "user_message",
		Content: contentText,
	})
	if c.vp.AtBottom() {
		c.autoScroll = true
	}
}

// AppendCustomMessage adds a custom message entry (TS pi-mono: CustomMessageComponent).
func (c *ChatViewport) AppendCustomMessage(customType, contentText string) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.entries = append(c.entries, ChatEntry{
		Type:       "custom_message",
		CustomType: customType,
		Content:    contentText,
		Expanded:   false,
	})
	if c.vp.AtBottom() {
		c.autoScroll = true
	}
}

// visibleWidth returns the visual column width of a string, ignoring ANSI
// escape sequences. CJK/wide characters count as 2, all others as 1.
func (c *ChatViewport) AppendCompactionSummary(summary string, tokensBefore int) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.entries = append(c.entries, ChatEntry{
		Type:         "custom_message",
		CustomType:   "compaction",
		Content:      summary,
		TokensBefore: tokensBefore,
		Expanded:     false,
	})
	if c.vp.AtBottom() {
		c.autoScroll = true
	}
}
