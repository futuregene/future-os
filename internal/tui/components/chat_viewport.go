package components

import (
	"strings"
	"sync"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

// ─── ChatViewport ──────────────────────────────────────────────────────────

// ChatEntry represents a single entry in the chat.
type ChatEntry struct {
	Type     string // "text", "thinking", "tool_call", "tool_result", "error", "system"
	ID       string
	Content  string
	Expanded bool
	// For tool calls
	ToolName string
	ToolArgs string
	IsError  bool
}

// ChatViewport manages the scrolling message list.
type ChatViewport struct {
	mu       sync.Mutex
	entries  []ChatEntry
	width    int
	height   int
	scroll   int
	followTail bool

	// Styling
	userStyle      lipgloss.Style
	assistantStyle lipgloss.Style
	thinkingStyle  lipgloss.Style
	toolStyle      lipgloss.Style
	errorStyle     lipgloss.Style
	systemStyle    lipgloss.Style
}

// NewChatViewport creates a new chat viewport.
func NewChatViewport() ChatViewport {
	return ChatViewport{
		entries:    make([]ChatEntry, 0),
		followTail: true,
		userStyle: lipgloss.NewStyle().
			Foreground(lipgloss.Color("#61afef")).
			PaddingLeft(2),
		assistantStyle: lipgloss.NewStyle().
			Foreground(lipgloss.Color("#abb2bf")).
			PaddingLeft(2),
		thinkingStyle: lipgloss.NewStyle().
			Foreground(lipgloss.Color("#5c6370")).
			Italic(true).
			PaddingLeft(4),
		toolStyle: lipgloss.NewStyle().
			Foreground(lipgloss.Color("#e5c07b")).
			PaddingLeft(2),
		errorStyle: lipgloss.NewStyle().
			Foreground(lipgloss.Color("#e06c75")).
			PaddingLeft(2),
		systemStyle: lipgloss.NewStyle().
			Foreground(lipgloss.Color("#98c379")).
			PaddingLeft(2),
	}
}

// SetSize updates the viewport dimensions.
func (c *ChatViewport) SetSize(w, h int) {
	c.width = w
	c.height = h
}

// AppendText adds a text chunk (or appends to the last text entry).
func (c *ChatViewport) AppendText(text string) {
	c.mu.Lock()
	defer c.mu.Unlock()

	if len(c.entries) > 0 && c.entries[len(c.entries)-1].Type == "text" {
		c.entries[len(c.entries)-1].Content += text
		return
	}
	c.entries = append(c.entries, ChatEntry{
		Type:    "text",
		Content: text,
	})
}

// AppendThinking adds a thinking chunk.
func (c *ChatViewport) AppendThinking(text string) {
	c.mu.Lock()
	defer c.mu.Unlock()

	if len(c.entries) > 0 && c.entries[len(c.entries)-1].Type == "thinking" {
		c.entries[len(c.entries)-1].Content += text
		return
	}
	c.entries = append(c.entries, ChatEntry{
		Type:     "thinking",
		Content:  text,
		Expanded: false, // default collapsed
	})
}

// AddToolCall records a new tool call.
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

// AppendError adds an error message.
func (c *ChatViewport) AppendError(msg string) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.entries = append(c.entries, ChatEntry{
		Type:    "error",
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

// ToggleExpand toggles the expand state of an entry by index.
func (c *ChatViewport) ToggleExpand(idx int) {
	if idx >= 0 && idx < len(c.entries) {
		c.entries[idx].Expanded = !c.entries[idx].Expanded
	}
}

// ScrollUp scrolls up.
func (c *ChatViewport) ScrollUp(n int) {
	c.followTail = false
	c.scroll += n
	if c.scroll > len(c.entries) {
		c.scroll = len(c.entries)
	}
}

// ScrollDown scrolls down.
func (c *ChatViewport) ScrollDown(n int) {
	c.scroll -= n
	if c.scroll < 0 {
		c.scroll = 0
		c.followTail = true
	}
}

// Update handles Bubble Tea messages.
func (c *ChatViewport) Update(msg tea.Msg) (ChatViewport, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.KeyMsg:
		switch msg.String() {
		case "pgup", "ctrl+u":
			c.ScrollUp(c.height / 2)
		case "pgdown", "ctrl+d":
			c.ScrollDown(c.height / 2)
		case "g":
			c.scroll = 0
		case "G":
			c.followTail = true
			c.scroll = 0
		}
	}
	return *c, nil
}

// View renders the chat viewport.
func (c *ChatViewport) View() string {
	c.mu.Lock()
	defer c.mu.Unlock()

	var sb strings.Builder
	visible := c.computeVisible()

	for _, e := range visible {
		switch e.Type {
		case "text":
			sb.WriteString(c.assistantStyle.Render(wordWrap(e.Content, c.width-10)))
		case "thinking":
			if e.Expanded {
				sb.WriteString(c.thinkingStyle.Render("💭 " + wordWrap(e.Content, c.width-10)))
			} else {
				sb.WriteString(c.thinkingStyle.Render("💭 Thinking… (expand)"))
			}
		case "tool_call":
			sb.WriteString(c.toolStyle.Render("🔧 " + e.ToolName))
			if e.Expanded && e.Content != "" {
				sb.WriteString(c.toolStyle.Render("\n" + wordWrap(e.Content, c.width-10)))
			}
		case "tool_result":
			prefix := "  ✓ "
			style := c.toolStyle
			if e.IsError {
				prefix = "  ✗ "
				style = c.errorStyle
			}
			sb.WriteString(style.Render(prefix + e.ToolName))
			if e.Expanded {
				sb.WriteString(style.Render("\n" + wordWrap(e.Content, c.width-10)))
			}
		case "error":
			sb.WriteString(c.errorStyle.Render("⚠ " + wordWrap(e.Content, c.width-10)))
		case "system":
			sb.WriteString(c.systemStyle.Render("ℹ " + wordWrap(e.Content, c.width-10)))
		}
		sb.WriteByte('\n')
	}

	return sb.String()
}

func (c *ChatViewport) computeVisible() []ChatEntry {
	if len(c.entries) == 0 {
		return nil
	}
	if c.followTail {
		// Show last N entries that fit in the viewport
		start := len(c.entries) - c.height
		if start < 0 {
			start = 0
		}
		return c.entries[start:]
	}
	// Show from scroll position
	start := c.scroll
	if start > len(c.entries)-c.height {
		start = len(c.entries) - c.height
	}
	if start < 0 {
		start = 0
	}
	end := start + c.height
	if end > len(c.entries) {
		end = len(c.entries)
	}
	return c.entries[start:end]
}

func wordWrap(s string, width int) string {
	if width <= 0 {
		return s
	}
	var result strings.Builder
	lineLen := 0
	for _, r := range s {
		if r == '\n' {
			result.WriteRune('\n')
			lineLen = 0
			continue
		}
		if lineLen >= width {
			result.WriteByte('\n')
			lineLen = 0
		}
		result.WriteRune(r)
		lineLen++
	}
	return result.String()
}
