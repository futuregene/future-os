package components

import (
	"fmt"
	"strings"
	"sync"

	"github.com/charmbracelet/bubbles/viewport"
	"github.com/charmbracelet/glamour"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

// OSC 133 semantic zone markers for terminal selection (TS pi-mono).
// Terminals like iTerm2 use these to provide semantic selection of message blocks.
const (
	osc133ZoneStart = "\x1b]133;A\x07"
	osc133ZoneEnd   = "\x1b]133;B\x07"
	osc133ZoneFinal = "\x1b]133;C\x07"
)

// ─── ChatViewport ──────────────────────────────────────────────────────────

// ChatEntry represents a single entry in the chat.
type ChatEntry struct {
	Type     string // "text", "thinking", "tool_call", "tool_result", "error", "system", "bash", "custom_message", "user_message"
	CustomType string // for custom_message entries: the custom type label (e.g. "skill", "compaction")
	ID       string
	Content  string
	Expanded bool
	// For tool calls
	ToolName     string
	ToolArgs     string
	IsError      bool
	ToolDuration string // "1.2s" or "Running..."
	ToolPending  bool
	// For bash execution (TS pi-mono style bordered display)
	BashCommand   string
	BashExitCode  int
	BashRunning   bool
	BashExcluded  bool // true for !! (excluded from LLM context), renders dim border
	BashLines     []string
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

	// HiddenThinkingLabel is the label shown when thinking blocks are collapsed.
	HiddenThinkingLabel string

	// Global tool expansion toggle (TS pi-mono: Ctrl+O)
	AllToolsExpanded bool

	// Styling
	assistantStyle  lipgloss.Style
	thinkingStyle   lipgloss.Style
	thinkingBorder  lipgloss.Style
	thinkingDim     lipgloss.Style
	toolStyle       lipgloss.Style
	toolPendingBg   lipgloss.Style
	toolSuccessBg   lipgloss.Style
	toolErrorBg     lipgloss.Style
	toolSuccess     lipgloss.Style
	toolError       lipgloss.Style
	errorStyle      lipgloss.Style
	systemStyle     lipgloss.Style
	warningStyle    lipgloss.Style
	bashBorder      lipgloss.Style
	bashHeader      lipgloss.Style
	bashOutput      lipgloss.Style
	bashStatus      lipgloss.Style
	bashErrorStatus lipgloss.Style
	// Custom message styling (TS pi-mono: CustomMessageComponent)
	customMessageBg    lipgloss.Style
	customMessageLabel lipgloss.Style
	// User message styling (TS pi-mono: UserMessageComponent)
	userMessageBg lipgloss.Style
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
		entries:             make([]ChatEntry, 0),
		vp:                  vp,
		mdRenderer:          renderer,
		HiddenThinkingLabel: "Thinking…",
		assistantStyle: lipgloss.NewStyle().
			Foreground(lipgloss.Color("#abb2bf")),
		thinkingStyle: lipgloss.NewStyle().
			Foreground(lipgloss.Color("#c678dd")),
		thinkingBorder: lipgloss.NewStyle().
			Border(lipgloss.NormalBorder(), false, true).
			BorderForeground(lipgloss.Color("#5c6370")).
			PaddingLeft(1),
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
		warningStyle: lipgloss.NewStyle().
			Foreground(lipgloss.Color("#e5c07b")),
		bashBorder: lipgloss.NewStyle().
			Foreground(lipgloss.Color("#5c6370")),
		bashHeader: lipgloss.NewStyle().
			Foreground(lipgloss.Color("#e5c07b")).Bold(true),
		bashOutput: lipgloss.NewStyle().
			Foreground(lipgloss.Color("#6c7086")),
		bashStatus: lipgloss.NewStyle().
			Foreground(lipgloss.Color("#98c379")),
		bashErrorStatus: lipgloss.NewStyle().
			Foreground(lipgloss.Color("#e06c75")),
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

// SetTheme updates all chat styles from theme colors.
func (c *ChatViewport) SetTheme(accent, muted, dim, warning, success, errColor, thinkingColor, thinkingText string) {
	c.assistantStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(muted))
	c.thinkingStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(thinkingColor))
	c.thinkingDim = lipgloss.NewStyle().Foreground(lipgloss.Color(dim)).Italic(true)
	c.toolStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(warning))
	c.toolPendingBg = lipgloss.NewStyle().Background(lipgloss.Color("#3a3a00")).Foreground(lipgloss.Color(warning))
	c.toolSuccessBg = lipgloss.NewStyle().Background(lipgloss.Color("#1a3a1a")).Foreground(lipgloss.Color(success))
	c.toolErrorBg = lipgloss.NewStyle().Background(lipgloss.Color("#3a1a1a")).Foreground(lipgloss.Color(errColor))
	c.toolSuccess = lipgloss.NewStyle().Foreground(lipgloss.Color(success))
	c.toolError = lipgloss.NewStyle().Foreground(lipgloss.Color(errColor))
	c.errorStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(errColor))
	c.systemStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(dim)).Italic(true)
	c.warningStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(warning))
	c.bashBorder = lipgloss.NewStyle().Foreground(lipgloss.Color(dim))
	c.bashHeader = lipgloss.NewStyle().Foreground(lipgloss.Color(warning)).Bold(true)
	c.bashOutput = lipgloss.NewStyle().Foreground(lipgloss.Color(dim))
	c.bashStatus = lipgloss.NewStyle().Foreground(lipgloss.Color(success))
	c.bashErrorStatus = lipgloss.NewStyle().Foreground(lipgloss.Color(errColor))
}


// SetGlamourStyle rebuilds the markdown renderer with the appropriate glamour style.
// "dark" is used for dark themes, "light" for light themes.
func (c *ChatViewport) SetGlamourStyle(style string) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.width < 10 {
		c.width = 80
	}
	renderer, err := glamour.NewTermRenderer(
		glamour.WithStandardStyle(style),
		glamour.WithEmoji(),
		glamour.WithWordWrap(c.width-4),
	)
	if err == nil {
		c.mdRenderer = renderer
	}
}

// SetThinkingBorderColor updates the thinking block border color.
func (c *ChatViewport) SetThinkingBorderColor(color string) {
	c.thinkingBorder = lipgloss.NewStyle().
		Border(lipgloss.NormalBorder(), false, true).
		BorderForeground(lipgloss.Color(color)).
		PaddingLeft(1)
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

// CompleteToolCall finalizes a pending tool_call entry's arguments in-place.
// If no matching pending entry exists, it creates a new one (fallback).
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

// SetLastBashDuration sets the duration on the last bash entry (index-based).
func (c *ChatViewport) SetLastBashDuration(duration string) {
	c.mu.Lock()
	defer c.mu.Unlock()
	for i := len(c.entries) - 1; i >= 0; i-- {
		if c.entries[i].Type == "bash" {
			c.entries[i].ToolDuration = duration
			return
		}
	}
}

// SetToolDuration sets the duration display for a tool result entry.
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

// ToggleAllTools toggles ALL tool results and bash entries globally (TS pi-mono: Ctrl+O).
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
			c.autoScroll = true
			return
		}
	}
}

// AddBashExecution starts a new bash execution entry with bordered display.
// Returns the entry index for later updates.
func (c *ChatViewport) AddBashExecution(command string, excluded bool) int {
	c.mu.Lock()
	defer c.mu.Unlock()
	idx := len(c.entries)
	c.entries = append(c.entries, ChatEntry{
		Type:         "bash",
		BashCommand:  command,
		BashRunning:  true,
		BashExcluded: excluded,
		Expanded:     false,
	})
	c.autoScroll = true
	return idx
}

// AppendBashOutput appends output lines to the last bash execution entry.
func (c *ChatViewport) AppendBashOutput(lines string) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if len(c.entries) == 0 {
		return
	}
	last := &c.entries[len(c.entries)-1]
	if last.Type != "bash" {
		return
	}
	for _, line := range strings.Split(lines, "\n") {
		last.BashLines = append(last.BashLines, line)
	}
	c.autoScroll = true
}

// CompleteBash marks the last bash execution as complete with an exit code.
func (c *ChatViewport) CompleteBash(exitCode int, cancelled bool) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if len(c.entries) == 0 {
		return
	}
	last := &c.entries[len(c.entries)-1]
	if last.Type != "bash" {
		return
	}
	last.BashRunning = false
	last.BashExitCode = exitCode
	if cancelled {
		last.BashExitCode = -1
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
func (c *ChatViewport) Clear() {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.entries = make([]ChatEntry, 0)
	c.vp.SetContent("")
}

// AppendChatEntry appends a pre-built ChatEntry and updates the viewport.
func (c *ChatViewport) AppendChatEntry(e ChatEntry) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.entries = append(c.entries, e)
}

// ScrollToTop jumps to the top of the chat (gg).
func (c *ChatViewport) ScrollToTop() {
	c.vp.GotoTop()
	c.autoScroll = false
}

// ScrollToBottom jumps to the bottom of the chat (G, follow mode).
func (c *ChatViewport) ScrollToBottom() {
	c.vp.GotoBottom()
	c.autoScroll = true
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
			sb.WriteString(osc133ZoneStart)
			rendered, err := c.mdRenderer.Render(e.Content)
			if err != nil {
				sb.WriteString(c.assistantStyle.Render(wordWrap(e.Content, c.width-10)))
			} else {
				rendered = strings.TrimSuffix(rendered, "\n")
				sb.WriteString(rendered)
			}
			sb.WriteString(osc133ZoneEnd)
			sb.WriteString(osc133ZoneFinal)
		case "thinking":
			if c.HideAllThinking {
				sb.WriteString(c.thinkingDim.Render("💭 " + c.HiddenThinkingLabel))
			} else if e.Expanded {
				rendered, err := c.mdRenderer.Render(e.Content)
				if err != nil {
					sb.WriteString(c.thinkingStyle.Render("💭 " + wordWrap(e.Content, c.width-10)))
				} else {
					rendered = strings.TrimSuffix(rendered, "\n")
					sb.WriteString(c.thinkingStyle.Render("💭 " + rendered))
				}
			} else {
				sb.WriteString(c.thinkingDim.Render("💭 " + c.HiddenThinkingLabel))
			}
		case "tool_call":
			// Tool call: pending (yellow) or running (with spinner)
			line := c.toolStyle.Render(toolIcon(e.ToolName) + e.ToolName)
			argsPreview := toolArgsPreview(e.ToolName, e.ToolArgs)
			if argsPreview != "" {
				line += " " + argsPreview
			}
			if e.ToolPending {
				line += c.toolStyle.Render("  ⠋ Running...")
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
				durPart = "  Took " + e.ToolDuration
			}
			detail := toolResultDetail(e)
			header := icon + toolIcon(e.ToolName) + e.ToolName + detail + durPart
			if !e.Expanded && e.Content != "" {
				header += "  (Ctrl+O to expand)"
			}
			sb.WriteString(bgStyle.Render(header))
			if e.Expanded && e.Content != "" {
				sb.WriteByte('\n')
				if isDiffContent(e.Content) {
					diffStyle := DefaultDiffStyle()
					rendered := RenderDiff(e.Content, diffStyle)
					for _, line := range strings.Split(rendered, "\n") {
						sb.WriteString(bgStyle.Render("  " + line))
						sb.WriteByte('\n')
					}
				} else {
					sb.WriteString(bgStyle.Render("  " + wordWrap(e.Content, c.width-12)))
				}
			}
		case "bash":
			sb.WriteString(c.renderBashEntry(e))
		case "error":
			sb.WriteString(c.errorStyle.Render("⚠ " + wordWrap(e.Content, c.width-10)))
		case "warning":
			sb.WriteString(c.warningStyle.Render("▲ " + wordWrap(e.Content, c.width-10)))
		case "system":
			sb.WriteString(c.systemStyle.Render("ℹ  " + wordWrap(e.Content, c.width-10)))
		case "custom_message":
			sb.WriteString(c.renderCustomMessageEntry(e))
		case "user_message":
			sb.WriteString(osc133ZoneStart)
			rendered, err := c.mdRenderer.Render(e.Content)
			if err != nil {
				sb.WriteString(c.userMessageBg.Render("  " + wordWrap(e.Content, c.width-12)))
			} else {
				rendered = strings.TrimSuffix(rendered, "\n")
				sb.WriteString(c.userMessageBg.Render("  " + rendered))
			}
			sb.WriteString(osc133ZoneEnd)
			sb.WriteString(osc133ZoneFinal)
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

// AppendUserMessage adds a user message entry with distinct background (TS pi-mono: UserMessageComponent).
func (c *ChatViewport) AppendUserMessage(contentText string) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.entries = append(c.entries, ChatEntry{
		Type:    "user_message",
		Content: contentText,
	})
	c.autoScroll = true
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
	c.autoScroll = true
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

// renderBashEntry renders a bash execution entry with bordered display
// matching TS pi-mono BashExecutionComponent style.
func (c *ChatViewport) renderBashEntry(e ChatEntry) string {
	boxWidth := c.width - 6
	if boxWidth < 20 {
		boxWidth = 20
	}
	if boxWidth > 120 {
		boxWidth = 120
	}

	// Dim all styles for excluded bash (TS pi-mono: !! uses dim border)
	borderStyle := c.bashBorder
	headerStyle := c.bashHeader
	outputStyle := c.bashOutput
	if e.BashExcluded {
		borderStyle = borderStyle.Copy().Faint(true)
		headerStyle = headerStyle.Copy().Faint(true)
		outputStyle = outputStyle.Copy().Faint(true)
	}

	var sb strings.Builder
	prefix := borderStyle.Render("│ ")
	headerPrefix := borderStyle.Render("┌─")
	botPrefix := borderStyle.Render("└")

	// Top border with command header
	cmdDisplay := e.BashCommand
	if len(cmdDisplay) > boxWidth-5 {
		cmdDisplay = cmdDisplay[:boxWidth-8] + "..."
	}
	header := headerStyle.Render("$ " + cmdDisplay)
	headerLine := headerPrefix + header +
		strings.Repeat(borderStyle.Render("─"), boxWidth-lipgloss.Width("┌─"+cmdDisplay)-1) +
		borderStyle.Render("┐")
	sb.WriteString(headerLine)
	sb.WriteByte('\n')

	// Output lines
	previewLines := 20
	outputLines := e.BashLines
	hiddenCount := 0
	if !e.Expanded && len(outputLines) > previewLines {
		hiddenCount = len(outputLines) - previewLines
		outputLines = outputLines[len(outputLines)-previewLines:]
	}

	for _, line := range outputLines {
		// Truncate line to fit within box
		displayLine := line
		if lipgloss.Width(line) > boxWidth-2 {
			displayLine = line[:boxWidth-5] + "..."
		}
		sb.WriteString(prefix)
		sb.WriteString(outputStyle.Render(displayLine))
		sb.WriteString(strings.Repeat(" ", boxWidth-lipgloss.Width(displayLine)-1))
		sb.WriteString(borderStyle.Render("│"))
		sb.WriteByte('\n')
	}

	// Status line
	if e.BashRunning {
		status := outputStyle.Render("⠋ Running...")
		sb.WriteString(prefix)
		sb.WriteString(status)
		sb.WriteString(strings.Repeat(" ", boxWidth-lipgloss.Width("Running...")-1))
	} else {
		cancelled := e.BashExitCode == -1
		statusStyle := c.bashStatus
		statusIcon := "✓"
		statusText := "exit 0"
		if cancelled {
			statusStyle = c.bashErrorStatus
			statusIcon = "✗"
			statusText = "cancelled"
		} else if e.BashExitCode != 0 {
			statusStyle = c.bashErrorStatus
			statusIcon = "✗"
			statusText = fmt.Sprintf("exit %d", e.BashExitCode)
		}
		if e.BashExcluded {
			statusStyle = statusStyle.Copy().Faint(true)
		}
		status := statusStyle.Render(statusIcon + " " + statusText)

		if hiddenCount > 0 {
			if e.Expanded {
				status += c.bashOutput.Render("  (Ctrl+O to collapse)")
			} else {
				status += c.bashOutput.Render(fmt.Sprintf("  ... %d more lines (Ctrl+O to expand)", hiddenCount))
			}
		}

		sb.WriteString(prefix)
		sb.WriteString(status)
		// Pad remaining space
		 padLen := boxWidth - lipgloss.Width(statusIcon+" "+statusText) - 1
		 if padLen < 0 {
		 	padLen = 0
		 }
		 sb.WriteString(strings.Repeat(" ", padLen))
	}
	sb.WriteString(borderStyle.Render("│"))
	sb.WriteByte('\n')

	// Bottom border
	sb.WriteString(botPrefix)
	sb.WriteString(borderStyle.Render(strings.Repeat("─", boxWidth-1) + "┘"))

	return sb.String()
}

// renderCustomMessageEntry renders a custom_message entry with purple background
// matching TS pi-mono CustomMessageComponent style.
func (c *ChatViewport) renderCustomMessageEntry(e ChatEntry) string {
	label := c.customMessageLabel.Render("[" + e.CustomType + "]")
	if e.Expanded && e.Content != "" {
		rendered, err := c.mdRenderer.Render(e.Content)
		if err != nil {
			return c.customMessageBg.Render(label + " " + wordWrap(e.Content, c.width-12))
		}
		rendered = strings.TrimSuffix(rendered, "\n")
		return c.customMessageBg.Render(label) + "\n" + c.customMessageBg.Render("  " + rendered)
	}
	// Collapsed: show first line of content as preview (TS pi-mono style)
	if e.Content != "" {
		firstLine := e.Content
		if idx := strings.IndexByte(firstLine, '\n'); idx != -1 {
			firstLine = firstLine[:idx]
		}
		if len(firstLine) > 80 {
			firstLine = firstLine[:77] + "..."
		}
		hint := " (Ctrl+O to expand)"
		return c.customMessageBg.Render(label + " " + firstLine + c.thinkingDim.Render(hint))
	}
	return c.customMessageBg.Render(label)
}

// toolArgsPreview returns a human-readable preview of tool arguments.

// toolIcon returns a per-tool icon matching TS pi-mono tool icons.
func toolIcon(name string) string {
	switch name {
	case "read", "read_file":
		return "📖 "
	case "edit", "patch":
		return "✏️ "
	case "write", "write_file":
		return "📝 "
	case "bash":
		return "💻 "
	case "grep":
		return "🔍 "
	case "ls":
		return "📂 "
	case "find":
		return "🔎 "
	case "web_search", "websearch":
		return "🌐 "
	case "web_fetch", "webfetch":
		return "📥 "
	case "notebook_edit", "notebookedit":
		return "📓 "
	default:
		return "🔧 "
	}
}

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

// toolResultDetail returns a tool-specific detail string for the collapsed view.
func toolResultDetail(e ChatEntry) string {
	switch e.ToolName {
	case "read", "edit", "write":
		if path := extractJSONField(e.ToolArgs, "file_path"); path != "" {
			detail := " " + path
			if lc := lineCount(e.Content); lc > 0 {
				detail += fmt.Sprintf(" (%d lines)", lc)
			}
			return detail
		}
	case "bash":
		if cmd := extractJSONField(e.ToolArgs, "command"); cmd != "" {
			if len(cmd) > 60 {
				cmd = cmd[:60] + "..."
			}
			detail := " " + cmd
			if lc := lineCount(e.Content); lc > 0 {
				detail += fmt.Sprintf(" (%d lines)", lc)
			}
			return detail
		}
	case "grep":
		if pat := extractJSONField(e.ToolArgs, "pattern"); pat != "" {
			return " " + pat
		}
	case "ls", "find", "glob":
		if dir := extractJSONField(e.ToolArgs, "path"); dir != "" {
			return " " + dir
		}
	case "web_search":
		if query := extractJSONField(e.ToolArgs, "query"); query != "" {
			if len(query) > 60 {
				query = query[:60] + "..."
			}
			return " " + query
		}
	case "web_fetch":
		if url := extractJSONField(e.ToolArgs, "url"); url != "" {
			if len(url) > 60 {
				url = url[:60] + "..."
			}
			return " " + url
		}
	}
	return ""
}

// lineCount returns the number of lines in a string.
func lineCount(s string) int {
	if s == "" {
		return 0
	}
	n := strings.Count(s, "\n") + 1
	// Don't count trailing empty line
	if strings.HasSuffix(s, "\n") {
		n--
	}
	return n
}

// isDiffContent detects whether content looks like a unified diff.
func isDiffContent(content string) bool {
	return strings.HasPrefix(content, "diff ") ||
		strings.Contains(content, "\n--- ") ||
		strings.Contains(content, "\n+++ ") ||
		strings.Contains(content, "\n@@ ")
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
