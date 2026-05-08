package components

import (
	"strings"
	"sync"

	"github.com/charmbracelet/bubbles/viewport"
	"github.com/charmbracelet/glamour"
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
	ToolName     string
	ToolArgs     string
	IsError      bool
	ToolDuration string // "1.2s" or "Running..."
	ToolPending  bool
}

// ChatViewport manages the scrolling message list using bubbles/viewport
// for proper line-based scrolling, mouse wheel, pgup/pgdown support.
type ChatViewport struct {
	mu      sync.Mutex
	entries []ChatEntry
	width   int
	height  int

	// bubbles viewport handles all scrolling natively
	vp viewport.Model

	// Auto-scroll to bottom when new content arrives (streaming mode)
	autoScroll bool

	// Glamour markdown renderer (rebuilt on SetSize for correct word-wrap width)
	mdRenderer *glamour.TermRenderer

	// Global thinking toggle (TS pi-mono: hideThinkingBlock)
	HideAllThinking bool

	// Styling
	assistantStyle  lipgloss.Style
	thinkingStyle   lipgloss.Style
	thinkingDim     lipgloss.Style
	toolStyle       lipgloss.Style
	toolPendingBg   lipgloss.Style
	toolSuccessBg   lipgloss.Style
	toolErrorBg     lipgloss.Style
	toolSuccess     lipgloss.Style
	toolError       lipgloss.Style
	errorStyle      lipgloss.Style
	systemStyle     lipgloss.Style
}

// NewChatViewport creates a new chat viewport with a glamour markdown renderer.
func NewChatViewport() ChatViewport {
	vp := viewport.New(80, 20)
	vp.Style = lipgloss.NewStyle().PaddingLeft(2)

	renderer, err := glamour.NewTermRenderer(
		glamour.WithStandardStyle("dark"),
		glamour.WithEmoji(),
		glamour.WithWordWrap(80),
	)
	if err != nil {
		renderer, _ = glamour.NewTermRenderer(
			glamour.WithStandardStyle("dark"),
		)
	}

	return ChatViewport{
		entries:    make([]ChatEntry, 0),
		vp:         vp,
		mdRenderer: renderer,
		assistantStyle: lipgloss.NewStyle().
			Foreground(lipgloss.Color("#abb2bf")),
		thinkingStyle: lipgloss.NewStyle().
			Foreground(lipgloss.Color("#c678dd")),
		thinkingDim: lipgloss.NewStyle().
			Foreground(lipgloss.Color("#5c6370")).
			Italic(true),
		toolStyle: lipgloss.NewStyle().
			Foreground(lipgloss.Color("#e5c07b")),
		toolPendingBg: lipgloss.NewStyle().
			Background(lipgloss.Color("#3a3a00")).
			Foreground(lipgloss.Color("#e5c07b")),
		toolSuccessBg: lipgloss.NewStyle().
			Background(lipgloss.Color("#1a3a1a")).
			Foreground(lipgloss.Color("#98c379")),
		toolErrorBg: lipgloss.NewStyle().
			Background(lipgloss.Color("#3a1a1a")).
			Foreground(lipgloss.Color("#e06c75")),
		toolSuccess: lipgloss.NewStyle().
			Foreground(lipgloss.Color("#98c379")),
		toolError: lipgloss.NewStyle().
			Foreground(lipgloss.Color("#e06c75")),
		errorStyle: lipgloss.NewStyle().
			Foreground(lipgloss.Color("#e06c75")),
		systemStyle: lipgloss.NewStyle().
			Foreground(lipgloss.Color("#5c6370")).
			Italic(true),
	}
}

// SetSize updates the viewport dimensions and rebuilds the glamour renderer.
func (c *ChatViewport) SetSize(w, h int) {
	c.width = w
	c.height = h
	c.vp.Width = w
	c.vp.Height = h

	// Rebuild glamour renderer with new width (minus padding)
	renderer, err := glamour.NewTermRenderer(
		glamour.WithStandardStyle("dark"),
		glamour.WithEmoji(),
		glamour.WithWordWrap(w - 4),
	)
	if err == nil {
		c.mdRenderer = renderer
	}
}

// AppendText adds a text chunk (or appends to the last text entry).
func (c *ChatViewport) AppendText(text string) {
	c.mu.Lock()
	defer c.mu.Unlock()

	if len(c.entries) > 0 && c.entries[len(c.entries)-1].Type == "text" {
		c.entries[len(c.entries)-1].Content += text
		c.autoScroll = true
		return
	}
	c.entries = append(c.entries, ChatEntry{
		Type:    "text",
		Content: text,
	})
	c.autoScroll = true
}

// AppendThinking adds a thinking chunk.
func (c *ChatViewport) AppendThinking(text string) {
	c.mu.Lock()
	defer c.mu.Unlock()

	if len(c.entries) > 0 && c.entries[len(c.entries)-1].Type == "thinking" {
		c.entries[len(c.entries)-1].Content += text
		c.autoScroll = true
		return
	}
	c.entries = append(c.entries, ChatEntry{
		Type:     "thinking",
		Content:  text,
		Expanded: true,
	})
	c.autoScroll = true
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
	c.autoScroll = true
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

// ScrollUp scrolls up by N lines (line-based, using viewport).
func (c *ChatViewport) ScrollUp(n int) {
	c.vp.LineUp(n)
	c.autoScroll = false
}

// ScrollDown scrolls down by N lines.
func (c *ChatViewport) ScrollDown(n int) {
	c.vp.LineDown(n)
	// Re-engage auto-scroll if scrolled to bottom
	if c.vp.AtBottom() {
		c.autoScroll = true
	}
}

// Update handles Bubble Tea messages. The bubbles viewport handles
// pgup/pgdown/ctrl+u/ctrl+d/home/end/mouse wheel natively.
func (c *ChatViewport) Update(msg tea.Msg) (*ChatViewport, tea.Cmd) {
	var cmd tea.Cmd
	c.vp, cmd = c.vp.Update(msg)
	return c, cmd
}

// View renders the chat viewport. All entries are rendered to a single
// string, and the viewport handles the scrolling window.
func (c *ChatViewport) View() string {
	c.mu.Lock()
	defer c.mu.Unlock()

	var sb strings.Builder

	for _, e := range c.entries {
		switch e.Type {
		case "text":
			rendered, err := c.mdRenderer.Render(e.Content)
			if err != nil {
				sb.WriteString(c.assistantStyle.Render(wordWrap(e.Content, c.width-10)))
			} else {
				rendered = strings.TrimSuffix(rendered, "\n")
				sb.WriteString(rendered)
			}
		case "thinking":
			if c.HideAllThinking {
				sb.WriteString(c.thinkingDim.Render("💭 Thinking…"))
			} else if e.Expanded {
				sb.WriteString(c.thinkingStyle.Render("💭 " + wordWrap(e.Content, c.width-10)))
			} else {
				sb.WriteString(c.thinkingDim.Render("💭 Thinking…"))
			}
		case "tool_call":
			// Tool call pending: yellow background
			line := c.toolStyle.Render("🔧 " + e.ToolName)
			argsPreview := toolArgsPreview(e.ToolName, e.ToolArgs)
			if argsPreview != "" {
				line += " " + argsPreview
			}
			sb.WriteString(c.toolPendingBg.Render(line))
			if e.Expanded && e.ToolArgs != "" {
				sb.WriteByte('\n')
				sb.WriteString(c.toolPendingBg.Render("  args: " + wordWrap(e.ToolArgs, c.width-12)))
			}
		case "tool_result":
			// Tool result: green (success) or red (error) background
			var bgStyle lipgloss.Style
			var icon string
			if e.IsError {
				bgStyle = c.toolErrorBg
				icon = "✗ "
			} else {
				bgStyle = c.toolSuccessBg
				icon = "✓ "
			}
			// Duration display (TS pi-mono: "Took X.Xs")
			durPart := ""
			if e.ToolDuration != "" {
				durPart = "  " + e.ToolDuration
			}
			sb.WriteString(bgStyle.Render(icon + e.ToolName + durPart))
			if e.Expanded && e.Content != "" {
				sb.WriteByte('\n')
				sb.WriteString(bgStyle.Render("  " + wordWrap(e.Content, c.width-12)))
			}
		case "error":
			sb.WriteString(c.errorStyle.Render("⚠ " + wordWrap(e.Content, c.width-10)))
		case "system":
			sb.WriteString(c.systemStyle.Render("ℹ  " + wordWrap(e.Content, c.width-10)))
		}
		sb.WriteByte('\n')
	}

	// Set the full rendered content into the viewport, then let it handle scrolling
	c.vp.SetContent(sb.String())

	// Auto-scroll to bottom during streaming
	if c.autoScroll {
		c.vp.GotoBottom()
	}

	return c.vp.View()
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

// toolArgsPreview returns a human-readable preview of tool arguments.
func toolArgsPreview(name, args string) string {
	switch name {
	case "read", "edit", "write":
		if path := extractJSONField(args, "file_path"); path != "" {
			return path
		}
	case "bash":
		if cmd := extractJSONField(args, "command"); cmd != "" {
			if len(cmd) > 60 {
				cmd = cmd[:60] + "..."
			}
			return cmd
		}
	case "grep":
		if pat := extractJSONField(args, "pattern"); pat != "" {
			return pat
		}
	case "ls":
		if dir := extractJSONField(args, "path"); dir != "" {
			return dir
		}
	}
	return ""
}

// extractJSONField extracts a string field from a JSON object string.
func extractJSONField(jsonStr, field string) string {
	needle := `"` + field + `": "`
	idx := strings.Index(jsonStr, needle)
	if idx < 0 {
		return ""
	}
	start := idx + len(needle)
	end := strings.IndexByte(jsonStr[start:], '"')
	if end < 0 {
		return ""
	}
	return jsonStr[start : start+end]
}
