package components

import (
	"strings"

	"github.com/charmbracelet/bubbles/viewport"
	"github.com/charmbracelet/glamour"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

func newGlamourRenderer(style string, wordWrap int) (*glamour.TermRenderer, error) {
	return glamour.NewTermRenderer(
		glamour.WithStandardStyle(style),
		glamour.WithEmoji(),
		glamour.WithWordWrap(wordWrap),
		glamour.WithChromaFormatter("terminal16m"),
	)
}

// NewChatViewport creates a new chat viewport with a glamour markdown renderer.
func NewChatViewport() ChatViewport {
	vp := viewport.New(80, 20)
	vp.Style = lipgloss.NewStyle().PaddingLeft(1)

	renderer, err := newGlamourRenderer("dark", 80)
	if err != nil {
		renderer, _ = glamour.NewTermRenderer(
			glamour.WithStandardStyle("dark"),
		)
	}

	return ChatViewport{
		entries:             make([]ChatEntry, 0),
		vp:                  vp,
		mdRenderer:          renderer,
		HiddenThinkingLabel: "Thinking...",
		assistantStyle: lipgloss.NewStyle().
			Foreground(lipgloss.Color("#abb2bf")),
		thinkingStyle: lipgloss.NewStyle().
			Foreground(lipgloss.Color("#c678dd")).
			Italic(true),
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
		// Custom message styling (TS pi-mono: customMessageBg / customMessageLabel)
		customMessageBg: lipgloss.NewStyle().
			Background(lipgloss.Color("#2d2838")),
		customMessageLabel: lipgloss.NewStyle().
			Foreground(lipgloss.Color("#9575cd")).
			Bold(true),
		customLabelStyle: lipgloss.NewStyle().
			Foreground(lipgloss.Color("#cba6f7")).
			Bold(true),
		customDimStyle: lipgloss.NewStyle().
			Foreground(lipgloss.Color("#6c7086")),
		// User message styling (TS pi-mono: UserMessageComponent)
		userMessageBg: lipgloss.NewStyle().
			Background(lipgloss.Color("#343541")),
		// DynamicBorder styling (TS pi-mono: DynamicBorder)
		borderStyle: lipgloss.NewStyle().
			Foreground(lipgloss.Color("#5c6370")),
	}
}

// SetSize updates the viewport dimensions and rebuilds the glamour renderer.
func (c *ChatViewport) SetSize(w, h int) {
	c.width = w
	c.height = h
	c.vp.Width = w
	c.vp.Height = h

	// Rebuild glamour renderer with new width (minus padding)
	renderer, err := newGlamourRenderer("dark", w-4)
	if err == nil {
		c.mdRenderer = renderer
	}
}

// syncComponentBase populates the shared component base from the ChatViewport's
// current state. Called at the start of View() so components always see fresh styles,
// toggles, and the markdown renderer (which may be rebuilt on SetSize).
func (c *ChatViewport) syncComponentBase() {
	cb := &c.componentBase
	cb.MdRenderer = c.mdRenderer
	cb.HideAllThinking = &c.HideAllThinking
	cb.HiddenThinkingLabel = &c.HiddenThinkingLabel
	cb.AllToolsExpanded = &c.AllToolsExpanded
	cb.ShowImages = &c.ShowImages
	cb.ImageWidthCells = &c.imageWidthCells
	cb.ToolToggleKey = &c.ToolToggleKey
	cb.SpinnerFrame = &c.spinnerFrame
	cb.AssistantStyle = c.assistantStyle
	cb.ThinkingStyle = c.thinkingStyle
	cb.ThinkingDim = c.thinkingDim
	cb.ToolStyle = c.toolStyle
	cb.ToolPendingBg = c.toolPendingBg
	cb.ToolSuccessBg = c.toolSuccessBg
	cb.ToolErrorBg = c.toolErrorBg
	cb.ToolSuccess = c.toolSuccess
	cb.ToolError = c.toolError
	cb.ErrorStyle = c.errorStyle
	cb.SystemStyle = c.systemStyle
	cb.WarningStyle = c.warningStyle
	cb.BashBorder = c.bashBorder
	cb.BashHeader = c.bashHeader
	cb.BashOutput = c.bashOutput
	cb.BashStatus = c.bashStatus
	cb.BashErrorStatus = c.bashErrorStatus
	cb.DiffAdd = c.diffAdd
	cb.DiffDel = c.diffDel
	cb.DiffCtx = c.diffCtx
	cb.DiffHeader = c.diffHeader
	cb.CustomMessageBg = c.customMessageBg
	cb.CustomMessageLabel = c.customMessageLabel
	cb.CustomLabelStyle = c.customLabelStyle
	cb.CustomDimStyle = c.customDimStyle
	cb.UserMessageBg = c.userMessageBg
	cb.BorderStyle = c.borderStyle
}

// SetTheme updates all chat styles from theme colors.
func (c *ChatViewport) SetTheme(accent, muted, dim, warning, success, errColor, thinkingColor, thinkingText string, toolPendingBgHex, toolSuccessBgHex, toolErrorBgHex string) {
	c.assistantStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(muted))
	c.thinkingStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(thinkingColor)).Italic(true)
	c.thinkingDim = lipgloss.NewStyle().Foreground(lipgloss.Color(dim)).Italic(true)
	c.toolStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(warning))
	c.toolPendingBg = lipgloss.NewStyle().Background(lipgloss.Color(toolPendingBgHex)).Foreground(lipgloss.Color(warning))
	c.toolSuccessBg = lipgloss.NewStyle().Background(lipgloss.Color(toolSuccessBgHex)).Foreground(lipgloss.Color(success))
	c.toolErrorBg = lipgloss.NewStyle().Background(lipgloss.Color(toolErrorBgHex)).Foreground(lipgloss.Color(errColor))
	c.toolSuccess = lipgloss.NewStyle().Foreground(lipgloss.Color(success))
	c.toolError = lipgloss.NewStyle().Foreground(lipgloss.Color(errColor))
	c.errorStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(errColor))
	c.systemStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(dim)).Italic(true)
	c.warningStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(warning))
	c.bashBorder = lipgloss.NewStyle().Foreground(lipgloss.Color(warning))
	c.bashHeader = lipgloss.NewStyle().Foreground(lipgloss.Color(warning)).Bold(true)
	c.bashOutput = lipgloss.NewStyle().Foreground(lipgloss.Color(dim))
	c.bashStatus = lipgloss.NewStyle().Foreground(lipgloss.Color(success))
	c.bashErrorStatus = lipgloss.NewStyle().Foreground(lipgloss.Color(errColor))
	c.diffAdd = lipgloss.NewStyle().Foreground(lipgloss.Color(success))
	c.diffDel = lipgloss.NewStyle().Foreground(lipgloss.Color(errColor))
	c.diffCtx = lipgloss.NewStyle().Foreground(lipgloss.Color(dim))
	c.diffHeader = lipgloss.NewStyle().Foreground(lipgloss.Color(accent)).Bold(true)
}


// SetGlamourStyle rebuilds the markdown renderer with the appropriate glamour style.
// "dark" is used for dark themes, "light" for light themes.
func (c *ChatViewport) SetGlamourStyle(style string) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.width < 10 {
		c.width = 80
	}
	renderer, err := newGlamourRenderer(style, c.width-4)
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

// SetBorderColor updates the DynamicBorder separator color (TS pi-mono).
func (c *ChatViewport) SetBorderColor(color string) {
	c.borderStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(color))
}

// RenderBorder returns a full-width horizontal separator line (TS pi-mono: DynamicBorder).
func (c *ChatViewport) SetToolToggleKey(key string) {
	c.ToolToggleKey = key
}

// SetShowImages enables or disables inline image rendering in tool results.
func (c *ChatViewport) SetShowImages(enabled bool) {
	c.ShowImages = enabled
}

// SetImageWidth sets the max width in cells for inline images.
func (c *ChatViewport) SetImageWidth(width int) {
	c.imageWidthCells = width
}

// ToggleAllTools toggles ALL tool results and bash entries globally (TS pi-mono: Ctrl+O).
func (c *ChatViewport) Update(msg tea.Msg) (*ChatViewport, tea.Cmd) {
	var cmd tea.Cmd
	c.vp, cmd = c.vp.Update(msg)
	return c, cmd
}

// SetSpinnerFrame sets the current spinner animation frame (TS pi-mono: Loader frame index).
func (c *ChatViewport) SetSpinnerFrame(frame int) {
	c.spinnerFrame = frame % len(spinnerChars)
}

// HasRunningTools returns true if any tool call or bash entry is currently executing.
func (c *ChatViewport) View() string {
	c.mu.Lock()
	defer c.mu.Unlock()

	// Sync shared state to component base (styles, toggles, renderer)
	c.syncComponentBase()

	// Wire component instances once (they embed base, which we just synced)
	c.assistantComp.base = &c.componentBase
	c.thinkingComp.base = &c.componentBase
	c.toolCallComp.base = &c.componentBase
	c.toolResultComp.base = &c.componentBase
	c.bashComp.base = &c.componentBase
	c.errorComp.base = &c.componentBase
	c.systemComp.base = &c.componentBase
	c.customMsgComp.base = &c.componentBase
	c.userMsgComp.base = &c.componentBase

	var sb strings.Builder

	for i, e := range c.entries {
		switch e.Type {
		case "text":
			c.assistantComp.HasFollowingToolCalls = c.hasToolCalls(i)
			sb.WriteString(c.assistantComp.Render(e, c.width))
		case "thinking":
			sb.WriteString(c.thinkingComp.Render(e, c.width))
		case "tool_call":
			// Classify compact read on the fly (args may have been completed after AddToolCall)
			if e.CompactReadKind == "" && e.ToolName == "read" {
				kind, label := classifyCompactRead(e.ToolArgs)
				if kind != "" {
					e.CompactReadKind = kind
					e.CompactReadLabel = label
					e.Expanded = false
					c.entries[i] = e
				}
			}
			sb.WriteString(c.toolCallComp.Render(e, c.width))
		case "tool_result":
			sb.WriteString(c.toolResultComp.Render(e, c.width))
		case "bash":
			sb.WriteString(c.bashComp.Render(e, c.width))
		case "error":
			sb.WriteString(c.errorComp.Render(e, c.width))
		case "warning":
			sb.WriteString(c.errorComp.Render(e, c.width))
		case "system":
			sb.WriteString(c.systemComp.Render(e, c.width))
		case "custom_message":
			sb.WriteString(c.customMsgComp.Render(e, c.width))
		case "user_message":
			sb.WriteString(c.userMsgComp.Render(e, c.width))
		}
		// Inter-entry spacing (TS pi-mono: Spacer(1) between message blocks)
		if i < len(c.entries)-1 {
			needBlank := needsBlankLine(e, c.entries[i+1])
			sb.WriteByte('\n')
			if needBlank {
				sb.WriteByte('\n')
			}
		}
	}

	// Set the full rendered content into the viewport, then let it handle scrolling
	c.vp.SetContent(sb.String())

	// Auto-scroll to bottom during streaming
	if c.autoScroll {
		c.vp.GotoBottom()
	}

	// Normalize Thai/Lao AM vowels to avoid stale-cell artifacts (TS pi-mono)
	return normalizeTerminalOutput(c.vp.View())
}

// AppendUserMessage adds a user message entry with distinct background (TS pi-mono: UserMessageComponent).
func (c *ChatViewport) Clear() {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.entries = make([]ChatEntry, 0)
	c.vp.SetContent("")
}

// GetEntries returns a copy of all chat entries (thread-safe).
func (c *ChatViewport) GetEntries() []ChatEntry {
	c.mu.Lock()
	defer c.mu.Unlock()
	result := make([]ChatEntry, len(c.entries))
	copy(result, c.entries)
	return result
}

// KeepEntries keeps only entries at the given indices, discarding the rest.
func (c *ChatViewport) KeepEntries(indices []int) {
	c.mu.Lock()
	defer c.mu.Unlock()
	idxSet := make(map[int]bool, len(indices))
	for _, i := range indices {
		idxSet[i] = true
	}
	kept := make([]ChatEntry, 0, len(indices))
	for i, e := range c.entries {
		if idxSet[i] {
			kept = append(kept, e)
		}
	}
	c.entries = kept
	c.vp.SetContent("") // force rebuild on next View()
}

// AppendChatEntry appends a pre-built ChatEntry and updates the viewport.
func (c *ChatViewport) ScrollToTop() {
	c.vp.GotoTop()
	c.autoScroll = false
}

// ScrollToBottom jumps to the bottom of the chat (G, follow mode).
func (c *ChatViewport) DisableAutoScroll() {
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
func (c *ChatViewport) AppendChatEntry(e ChatEntry) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.entries = append(c.entries, e)
}

// ScrollToTop jumps to the top of the chat (gg).
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
