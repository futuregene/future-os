package components

import (
	"fmt"
	"strconv"
	"strings"
	"sync"
	"path/filepath"
	"unicode/utf8"

	"github.com/charmbracelet/bubbles/viewport"
	"github.com/charmbracelet/glamour"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

// spinnerChars matches TS pi-mono DEFAULT_FRAMES for animated loader.
var spinnerChars = []string{"⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"}

// OSC 133 semantic zone markers for terminal selection (TS pi-mono).
// Terminals like iTerm2 use these to provide semantic selection of message blocks.
const (
	osc133ZoneStart = "\x1b]133;A\x07"
	osc133ZoneEnd   = "\x1b]133;B\x07"
	osc133ZoneFinal = "\x1b]133;C\x07"
)

// ─── ChatViewport ──────────────────────────────────────────────────────────

// ImageBlock represents an inline image in a tool result (TS pi-mono: ImageBlock).
type ImageBlock struct {
	Base64Data string
	MimeType   string
}

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
	BashLines          []string
	BashTruncated      bool
	BashFullOutputPath string
	// For compaction summary (TS pi-mono: CompactionSummaryMessageComponent)
	TokensBefore int
	// Image blocks for inline image display in tool results (TS pi-mono)
	ImageBlocks []ImageBlock
	// Stop reason for aborted/errored assistant messages (TS pi-mono: stopReason)
	StopReason string // "aborted", "error", or empty
	ErrorMessage string // detailed error for stop reasons (TS pi-mono: errorMessage)
	// CompactReadKind indicates this read result should render compact: "skill", "docs", "resource" (TS pi-mono: CompactReadClassification)
	CompactReadKind string
	// CompactReadLabel is the display label for compact read entries (TS pi-mono: compact label)
	CompactReadLabel string
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

	// ShowImages enables inline image rendering in tool results (TS pi-mono: showImages)
	ShowImages bool

	// imageWidthCells is the max width for inline images in terminal cells (user setting).
	imageWidthCells int

	// ToolToggleKey is the formatted key string for toggling tool outputs.
	// Default: "Ctrl+O". Set via SetToolToggleKey from app keybindings.
	ToolToggleKey string

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
	diffAdd         lipgloss.Style
	diffDel         lipgloss.Style
	diffCtx         lipgloss.Style
	diffHeader      lipgloss.Style
	// Custom message styling (TS pi-mono: CustomMessageComponent)
	customMessageBg    lipgloss.Style
	customMessageLabel lipgloss.Style
	customLabelStyle  lipgloss.Style // compact read label style
	customDimStyle    lipgloss.Style // dim style for compact read hint
	// User message styling (TS pi-mono: UserMessageComponent)
	userMessageBg lipgloss.Style
	// Border style for DynamicBorder separators (TS pi-mono: DynamicBorder)
	borderStyle lipgloss.Style
	// Animated spinner state (TS pi-mono: Loader spinner)
	spinnerFrame int
}

// newGlamourRenderer creates a glamour terminal renderer with consistent settings.
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
	vp.Style = lipgloss.NewStyle().PaddingLeft(2)

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
func (c *ChatViewport) RenderBorder(width int) string {
	if width < 2 {
		width = 2
	}
	return c.borderStyle.Render(strings.Repeat("─", width))
}

// hasToolCalls checks if the text entry at idx has tool calls in the same message block.
// In pi-mono, OSC 133 zones are only applied when the message has no tool calls.
func (c *ChatViewport) hasToolCalls(idx int) bool {
	for j := idx + 1; j < len(c.entries); j++ {
		switch c.entries[j].Type {
		case "user_message":
			return false // reached next message, no tool calls found
		case "tool_call":
			return true
		}
	}
	return false
}

// AppendText adds a text chunk (or appends to the last text entry).
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
func (c *ChatViewport) AppendThinking(text string) {
	c.mu.Lock()
	defer c.mu.Unlock()

	if len(c.entries) > 0 && c.entries[len(c.entries)-1].Type == "thinking" {
		c.entries[len(c.entries)-1].Content += text
		if c.vp.AtBottom() {
			c.autoScroll = true
		}
		return
	}
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
func classifyCompactRead(args string) (kind, label string) {
	path := extractJSONField(args, "file_path")
	if path == "" {
		path = extractJSONField(args, "path")
	}
	if path == "" {
		return "", ""
	}

	base := filepath.Base(path)
	if base == "SKILL.md" {
		parent := filepath.Base(filepath.Dir(path))
		if parent == "" || parent == "." {
			parent = base
		}
		return "skill", parent
	}

	if compactReadFileNames[base] {
		return "resource", base
	}

	// Check for pi docs: README.md, docs/*, examples/*
	slashPath := filepath.ToSlash(path)
	if base == "README.md" || strings.HasPrefix(slashPath, "docs/") || strings.HasPrefix(slashPath, "examples/") {
		return "docs", slashPath
	}

	return "", ""
}

// markCompactRead marks a tool_call entry for compact rendering if applicable (TS pi-mono: compact read call).
func (c *ChatViewport) markCompactRead(idx int) {
	if idx < 0 || idx >= len(c.entries) {
		return
	}
	e := &c.entries[idx]
	if e.ToolName != "read" {
		return
	}
	kind, label := classifyCompactRead(e.ToolArgs)
	if kind != "" {
		e.CompactReadKind = kind
		e.CompactReadLabel = label
		e.Expanded = false // collapsed by default for system files
	}
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

// SetToolToggleKey sets the key string shown in tool expand/collapse hints.
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

// toggleKey returns the tool toggle key string with fallback.
func (c *ChatViewport) toggleKey() string {
	if c.ToolToggleKey != "" {
		return c.ToolToggleKey
	}
	return "Ctrl+O"
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
			if c.vp.AtBottom() {
				c.autoScroll = true
			}
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
	if c.vp.AtBottom() {
		c.autoScroll = true
	}
	return idx
}

// AppendBashOutput appends output lines to the last bash execution entry.
// Strips ANSI escape sequences from output matching TS pi-mono stripAnsi behavior.
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
		// Strip ANSI codes and normalize line endings (TS pi-mono: stripAnsi)
		clean := stripAnsiCodes(lines)
		clean = strings.ReplaceAll(clean, "\r\n", "\n")
		clean = strings.ReplaceAll(clean, "\r", "\n")
		for _, line := range strings.Split(clean, "\n") {
		last.BashLines = append(last.BashLines, line)
	}
	if c.vp.AtBottom() {
		c.autoScroll = true
	}
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

// SetBashTruncation sets truncation info on the last bash entry (TS pi-mono: truncation warning inline in border).
func (c *ChatViewport) SetBashTruncation(truncated bool, fullOutputPath string) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if len(c.entries) == 0 {
		return
	}
	last := &c.entries[len(c.entries)-1]
	if last.Type != "bash" {
		return
	}
	last.BashTruncated = truncated
	last.BashFullOutputPath = fullOutputPath
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

// SetSpinnerFrame sets the current spinner animation frame (TS pi-mono: Loader frame index).
func (c *ChatViewport) SetSpinnerFrame(frame int) {
	c.spinnerFrame = frame % len(spinnerChars)
}

// HasRunningTools returns true if any tool call or bash entry is currently executing.
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

// spinnerChar returns the current spinner animation frame (TS pi-mono: Loader frame).
func (c *ChatViewport) spinnerChar() string {
	return spinnerChars[c.spinnerFrame%len(spinnerChars)]
}

// View renders the chat viewport. All entries are rendered to a single
// string, and the viewport handles the scrolling window.
func (c *ChatViewport) View() string {
	c.mu.Lock()
	defer c.mu.Unlock()

	var sb strings.Builder
	sb.WriteByte('\n')

	for i, e := range c.entries {
		switch e.Type {
		case "text":
			// OSC 133 zones only when no tool calls in this message (TS pi-mono)
			hasToolCalls := c.hasToolCalls(i)
			if !hasToolCalls {
				sb.WriteString(osc133ZoneStart)
			}
			rendered, err := c.mdRenderer.Render(e.Content)
			if err != nil {
				sb.WriteString(c.assistantStyle.Render(wordWrap(e.Content, c.width-10)))
			} else {
				rendered = strings.TrimSuffix(rendered, "\n")
				sb.WriteString(wrapURLsOSC8(rendered))
			}
			if !hasToolCalls {
				sb.WriteString(osc133ZoneEnd)
				sb.WriteString(osc133ZoneFinal)
			}
			// Stop reason rendering (TS pi-mono: stopReason)
			if e.StopReason == "aborted" {
				sb.WriteByte('\n')
				abortMsg := "Operation aborted"
				if e.ErrorMessage != "" && e.ErrorMessage != "Request was aborted" {
					abortMsg = e.ErrorMessage
				}
				sb.WriteString(c.errorStyle.Render(abortMsg))
			} else if e.StopReason == "error" {
				sb.WriteByte('\n')
				errMsg := "Unknown error"
				if e.ErrorMessage != "" {
					errMsg = e.ErrorMessage
				}
				sb.WriteString(c.errorStyle.Render("Error: " + errMsg))
			} else if e.StopReason == "length" {
				sb.WriteByte('\n')
				sb.WriteString(c.warningStyle.Render("[Output truncated — model reached token limit]"))
			}
		case "thinking":
			if c.HideAllThinking {
				sb.WriteString(c.thinkingDim.Render("💭 " + c.HiddenThinkingLabel))
			} else if e.Expanded {
				rendered, err := c.mdRenderer.Render(e.Content)
				if err != nil {
					sb.WriteString(c.thinkingStyle.Render("💭 " + wordWrap(e.Content, c.width-10)))
				} else {
					rendered = strings.TrimSuffix(rendered, "\n")
					sb.WriteString(c.thinkingStyle.Render("💭 " + wrapURLsOSC8(rendered)))
				}
			} else {
				sb.WriteString(c.thinkingDim.Render("💭 " + c.HiddenThinkingLabel))
			}
		case "tool_call":
			// Compact read rendering (TS pi-mono: compact read for system files)
			// Classify on the fly since args may have been completed after AddToolCall
			if e.CompactReadKind == "" && e.ToolName == "read" {
				kind, label := classifyCompactRead(e.ToolArgs)
				if kind != "" {
					e.CompactReadKind = kind
					e.CompactReadLabel = label
					e.Expanded = false
				}
			}
			if e.CompactReadKind != "" {
				var compactLine string
				switch e.CompactReadKind {
				case "skill":
					compactLine = c.customLabelStyle.Render("[skill] ") + e.CompactReadLabel
				case "docs":
					compactLine = c.toolStyle.Render("read docs ") + e.CompactReadLabel
				case "resource":
					compactLine = c.toolStyle.Render("read resource ") + e.CompactReadLabel
				}
				if lr := formatLineRange(e.ToolArgs); lr != "" {
					compactLine += c.warningStyle.Render(lr)
				}
				if !e.Expanded {
					compactLine += c.customDimStyle.Render(" (" + c.toggleKey() + " to expand)")
				}
				sb.WriteString(applyLineBg(compactLine, c.width, c.toolPendingBg))
				if e.Expanded && e.ToolArgs != "" {
					sb.WriteByte('\n')
					sb.WriteString(prefixedLineBg("  args: ", wordWrap(e.ToolArgs, c.width-14), c.width, c.toolPendingBg))
				}
				break
			}
			// Tool call: pending (yellow) or running (with spinner)
			line := c.toolStyle.Render(toolIcon(e.ToolName) + e.ToolName)
			argsPreview := toolArgsPreview(e.ToolName, e.ToolArgs)
			if argsPreview != "" {
				line += " " + argsPreview
			}
			if e.ToolPending {
				line += c.toolStyle.Render("  " + c.spinnerChar() + " Running... (Esc to cancel)")
			}
			sb.WriteString(applyLineBg(line, c.width, c.toolPendingBg))
			if e.Expanded && e.ToolArgs != "" {
				sb.WriteByte('\n')
				sb.WriteString(prefixedLineBg("  args: ", wordWrap(e.ToolArgs, c.width-14), c.width, c.toolPendingBg))
			}
		case "tool_result":
			// Compact read result: show compact one-liner when collapsed (TS pi-mono hides result)
			if e.CompactReadKind != "" {
				bgStyle := c.toolSuccessBg
				icon := "✓ "
				if e.IsError {
					bgStyle = c.toolErrorBg
					icon = "✗ "
				}
				var compactLine string
				switch e.CompactReadKind {
				case "skill":
					compactLine = c.customLabelStyle.Render("[skill] ") + e.CompactReadLabel
				case "docs":
					compactLine = c.toolStyle.Render("read docs ") + e.CompactReadLabel
				case "resource":
					compactLine = c.toolStyle.Render("read resource ") + e.CompactReadLabel
				}
				if lr := formatLineRange(e.ToolArgs); lr != "" {
					compactLine += c.warningStyle.Render(lr)
				}
				if !e.Expanded {
					compactLine += c.customDimStyle.Render(" (" + c.toggleKey() + " to expand)")
				}
				sb.WriteString(applyLineBg(icon + compactLine, c.width, bgStyle))
				if e.Expanded && e.Content != "" {
					sb.WriteByte(0x0a)
					if isDiffContent(e.Content) {
						diffStyle := DiffStyle{
							Add:     c.diffAdd,
							Del:     c.diffDel,
							Context: c.diffCtx,
							Header:  c.diffHeader,
							Inverse: lipgloss.NewStyle().Reverse(true),
						}
						rendered := RenderDiff(e.Content, diffStyle)
						sb.WriteString(prefixedLineBg(" ", rendered, c.width, bgStyle))
					} else {
						sb.WriteString(prefixedLineBg(" ", wordWrap(e.Content, c.width-14), c.width, bgStyle))
					}
				}
				break
			}
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
				header += "  (" + c.toggleKey() + " to expand)"
			}
			sb.WriteString(applyLineBg(header, c.width, bgStyle))
			if e.Expanded && e.Content != "" {
				sb.WriteByte('\n')
				if isDiffContent(e.Content) {
					diffStyle := DiffStyle{
						Add:     c.diffAdd,
						Del:     c.diffDel,
						Context: c.diffCtx,
						Header:  c.diffHeader,
						Inverse: lipgloss.NewStyle().Reverse(true),
					}
					rendered := RenderDiff(e.Content, diffStyle)
					sb.WriteString(prefixedLineBg(" ", rendered, c.width, bgStyle))
				} else {
					sb.WriteString(prefixedLineBg(" ", wordWrap(e.Content, c.width-14), c.width, bgStyle))
				}
			// Inline images (TS pi-mono: ImageBlock rendering in tool results)
			if e.Expanded && c.ShowImages && len(e.ImageBlocks) > 0 {
				imageTheme := ImageTheme{
					FallbackColor: func(s string) string {
						return lipgloss.NewStyle().Foreground(lipgloss.Color("#6c7086")).Render(s)
					},
				}
				for _, ib := range e.ImageBlocks {
					maxImgWidth := c.width - 4
					if c.imageWidthCells > 0 && c.imageWidthCells < maxImgWidth {
						maxImgWidth = c.imageWidthCells
					}
					img := NewImage(ib.Base64Data, ib.MimeType, imageTheme, ImageOptions{
						MaxWidthCells: maxImgWidth,
					})
					imageLines := img.Render(c.width)
					for _, line := range imageLines {
						sb.WriteByte('\n')
						sb.WriteString(applyLineBg("  "+line, c.width, bgStyle))
					}
				}
			}
			}
		case "bash":
			sb.WriteString(c.renderBashEntry(e))
		case "error":
			sb.WriteString(c.errorStyle.Render("Error: " + wordWrap(e.Content, c.width-10)))
		case "warning":
			sb.WriteString(c.warningStyle.Render("Warning: " + wordWrap(e.Content, c.width-10)))
		case "system":
			sb.WriteString(c.systemStyle.Render("  " + wordWrap(e.Content, c.width-10)))
		case "custom_message":
			sb.WriteString(c.renderCustomMessageEntry(e))
		case "user_message":
			sb.WriteString(osc133ZoneStart)
			rendered, err := c.mdRenderer.Render(e.Content)
			if err != nil {
				sb.WriteString(prefixedLineBg(" ", wordWrap(e.Content, c.width-14), c.width, c.userMessageBg))
			} else {
				rendered = strings.TrimSuffix(rendered, "\n")
				sb.WriteString(prefixedLineBg(" ", wrapURLsOSC8(rendered), c.width, c.userMessageBg))
			}
			sb.WriteString(osc133ZoneEnd)
			sb.WriteString(osc133ZoneFinal)
		}
		// Inter-entry spacing (TS pi-mono: Spacer(1) between message blocks)
		if i < len(c.entries)-1 {
			next := c.entries[i+1]
			needBlank := needsBlankLine(e, next)
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
func visibleWidth(s string) int {
	w := 0
	inAnsi := false
	for i := 0; i < len(s); {
		if inAnsi {
			if s[i] >= '@' && s[i] <= '~' {
				inAnsi = false
			}
			i++
			continue
		}
		if s[i] == '\x1b' && i+1 < len(s) && s[i+1] == '[' {
			inAnsi = true
			i += 2
			continue
		}
		r, size := utf8.DecodeRuneInString(s[i:])
		if isWideRune(r) {
			w += 2
		} else {
			w += 1
		}
		i += size
	}
	return w
}

// isWideRune returns true for CJK ideographs, hangul, kana, and emoji that
// occupy two terminal columns.
func isWideRune(r rune) bool {
	if r >= 0x1100 && r <= 0x115f ||
		r == 0x2329 || r == 0x232a ||
		r >= 0x2e80 && r <= 0xa4cf ||
		r >= 0xac00 && r <= 0xd7a3 ||
		r >= 0xf900 && r <= 0xfaff ||
		r >= 0xfe10 && r <= 0xfe19 ||
		r >= 0xfe30 && r <= 0xfe6f ||
		r >= 0xff00 && r <= 0xff60 ||
		r >= 0xffe0 && r <= 0xffe6 ||
		r >= 0x1f300 && r <= 0x1f64f ||
		r >= 0x1f680 && r <= 0x1f6ff ||
		r >= 0x1f900 && r <= 0x1f9ff {
		return true
	}
	return false
}

// needsBlankLine returns true if a blank line should separate two consecutive entries (TS pi-mono: Spacer(1)).
func needsBlankLine(current, next ChatEntry) bool {
	// Blank line between distinct message blocks: user→assistant, assistant→next message,
	// tool results→next, bash→next, compaction→next, etc.
	// Blank line between: user_msg→anything, anything→next distinct block
	if current.Type == "user_message" {
		return true
	}
	if current.Type == "text" || current.Type == "thinking" {
		return next.Type != "thinking" // no blank between consecutive thinking blocks
	}
	if current.Type == "tool_result" || current.Type == "tool_call" {
		return next.Type != "tool_result" && next.Type != "tool_call"
	}
	if current.Type == "bash" {
		return true
	}
	if current.Type == "custom_message" || current.Type == "system" ||
		current.Type == "error" || current.Type == "warning" {
		return true
	}
	return false
}

// normalizeTerminalOutput decomposes Thai/Lao AM vowels to avoid stale-cell
// artifacts in terminal renderers during differential repaint (TS pi-mono).
func normalizeTerminalOutput(s string) string {
	if !strings.ContainsRune(s, 'ำ') && !strings.ContainsRune(s, 'ຳ') {
		return s
	}
	s = strings.ReplaceAll(s, "ำ", "ํา")
	s = strings.ReplaceAll(s, "ຳ", "ໍາ")
	return s
}

// wordWrap wraps text at word boundaries (spaces), never breaking mid-word
// unless a single word exceeds the width. ANSI escape sequences are skipped
// when measuring width.
func wordWrap(s string, width int) string {
	if width <= 0 {
		return s
	}
	var result strings.Builder

	writeBreak := func(cur *int) {
		result.WriteByte('\n')
		*cur = 0
	}

	lines := strings.Split(s, "\n")
	for li, line := range lines {
		if li > 0 {
			result.WriteByte('\n')
		}
		vw := visibleWidth(line)
		if vw <= width {
			result.WriteString(line)
			continue
		}
		// Tokenise into (word, whitespace) pairs so we can wrap at spaces.
		cur := 0
		var tok strings.Builder
		flush := func() {
			if tok.Len() == 0 {
				return
			}
			t := tok.String()
			tw := visibleWidth(t)
			tok.Reset()
			if tw > width {
				// Word longer than line — force-break by character.
				breakLongWord(&result, t, width, &cur)
				return
			}
			if cur+tw > width && cur > 0 {
				writeBreak(&cur)
			}
			result.WriteString(t)
			cur += tw
		}
		for i := 0; i < len(line); {
			if line[i] == ' ' {
				flush()
				if cur >= width {
					writeBreak(&cur)
					// skip leading space on fresh line
					i++
					continue
				}
				result.WriteByte(' ')
				cur++
				i++
				continue
			}
			// Collect ANSI codes (attach to current token)
			if line[i] == '\x1b' && i+1 < len(line) && line[i+1] == '[' {
				end := i + 2
				for end < len(line) && !(line[end] >= '@' && line[end] <= '~') {
					end++
				}
				if end < len(line) {
					end++
				}
				tok.WriteString(line[i:end])
				i = end
				continue
			}
			r, size := utf8.DecodeRuneInString(line[i:])
			tok.WriteRune(r)
			i += size
		}
		flush()
	}
	return result.String()
}

// breakLongWord breaks a single token that exceeds width by inserting newlines,
// preserving any embedded ANSI codes.
func breakLongWord(rb *strings.Builder, token string, width int, cur *int) {
	col := *cur
	for i := 0; i < len(token); {
		// Collect any ANSI prefix
		ansiPrefix := ""
		for i < len(token) && token[i] == '\x1b' && i+1 < len(token) && token[i+1] == '[' {
			end := i + 2
			for end < len(token) && !(token[end] >= '@' && token[end] <= '~') {
				end++
			}
			if end < len(token) {
				end++
			}
			ansiPrefix += token[i:end]
			i = end
		}
		if i >= len(token) {
			break
		}
		r, size := utf8.DecodeRuneInString(token[i:])
		rw := 1
		if isWideRune(r) {
			rw = 2
		}
		if col+rw > width && col > 0 {
			rb.WriteByte('\n')
			col = 0
			if ansiPrefix != "" {
				rb.WriteString(ansiPrefix)
			}
		}
		if ansiPrefix != "" {
			rb.WriteString(ansiPrefix)
		}
		rb.WriteRune(r)
		col += rw
		i += size
	}
	*cur = col
}

// applyLineBg pads each line to the given width with spaces then wraps it in a
// background style, matching TS pi-mono applyBackgroundToLine. Multi-line input
// (separated by \n) is processed line-by-line.
func applyLineBg(s string, width int, style lipgloss.Style) string {
	var sb strings.Builder
	for _, line := range strings.Split(s, "\n") {
		vw := visibleWidth(line)
		if vw < width {
			line += strings.Repeat(" ", width-vw)
		}
		sb.WriteString(style.Render(line))
		sb.WriteByte('\n')
	}
	return strings.TrimSuffix(sb.String(), "\n")
}

// prefixedLineBg applies a prefix to every line before calling applyLineBg.
// Useful for content margins inside background-colored blocks.
func prefixedLineBg(prefix, content string, width int, style lipgloss.Style) string {
	lines := strings.Split(content, "\n")
	for i, l := range lines {
		lines[i] = prefix + l
	}
	return applyLineBg(strings.Join(lines, "\n"), width, style)
}

// renderBashEntry renders a bash execution entry with bordered display
// matching TS pi-mono BashExecutionComponent style.

// wrapURLsOSC8 wraps bare http/https URLs in OSC 8 hyperlink sequences (TS pi-mono).
// Handles ANSI escape codes that may be interleaved within URLs.
func wrapURLsOSC8(s string) string {
	if !strings.Contains(s, "http://") && !strings.Contains(s, "https://") {
		return s
	}

	var result strings.Builder
	i := 0
	for i < len(s) {
		// Look for next URL start
		rem := s[i:]
		httpIdx := strings.Index(rem, "http://")
		httpsIdx := strings.Index(rem, "https://")
		urlStart := -1
		if httpIdx >= 0 && (httpsIdx < 0 || httpIdx < httpsIdx) {
			urlStart = httpIdx
		} else if httpsIdx >= 0 {
			urlStart = httpsIdx
		}
		if urlStart < 0 {
			result.WriteString(rem)
			break
		}

		// Write everything before the URL
		result.WriteString(rem[:urlStart])
		urlRem := rem[urlStart:]

		// Extract URL characters (stop at whitespace, ANSI end, or special chars)
		urlEnd := 0
		inAnsi := false
		cleanURL := ""
		for j := 0; j < len(urlRem); j++ {
			ch := urlRem[j]
			if inAnsi {
				cleanURL += string(ch)
				if ch >= '@' && ch <= '~' {
					inAnsi = false
				}
				urlEnd = j + 1
				continue
			}
			if ch == '\x1b' && j+1 < len(urlRem) && urlRem[j+1] == '[' {
				inAnsi = true
				urlEnd = j + 1
				continue
			}
			if ch == ' ' || ch == '\t' || ch == '\n' || ch == '\r' ||
				ch == '"' || ch == '\'' || ch == '<' || ch == '>' ||
				ch == ')' || ch == ']' || ch == '}' {
				break
			}
			urlEnd = j + 1
		}

		if urlEnd > 0 {
			rawURL := urlRem[:urlEnd]
			// Strip ANSI codes to get the clean URL text
			clean := stripAnsiCodes(rawURL)
			// Wrap in OSC 8
			result.WriteString(fmt.Sprintf("\x1b]8;;%s\x1b\\%s\x1b]8;;\x1b\\", clean, rawURL))
		}
		i += urlStart + urlEnd
	}
	return result.String()
}

// stripAnsiCodes removes ANSI escape sequences from a string.
func stripAnsiCodes(s string) string {
	var b strings.Builder
	inAnsi := false
	for i := 0; i < len(s); i++ {
		if inAnsi {
			if s[i] >= '@' && s[i] <= '~' {
				inAnsi = false
			}
			continue
		}
		if s[i] == '\x1b' && i+1 < len(s) && s[i+1] == '[' {
			inAnsi = true
			i++ // skip '['
			continue
		}
		b.WriteByte(s[i])
	}
	return b.String()
}

// TruncateByWidth truncates a string to fit within a visual width,
// preserving ANSI escape sequences. Returns the truncated string.
func TruncateByWidth(s string, maxWidth int) string {
	if maxWidth <= 0 {
		return ""
	}
	visualWidth := 0
	inAnsi := false
	for i := 0; i < len(s); {
		if inAnsi {
			if s[i] >= '@' && s[i] <= '~' {
				inAnsi = false
			}
			i++
			continue
		}
		if s[i] == '\x1b' && i+1 < len(s) && s[i+1] == '[' {
			inAnsi = true
			i += 2
			continue
		}
		r, size := utf8.DecodeRuneInString(s[i:])
		w := 1
		if isWideRune(r) {
			w = 2
		}
		if visualWidth+w > maxWidth {
			return s[:i]
		}
		visualWidth += w
		i += size
	}
	return s
}
func (c *ChatViewport) renderBashEntry(e ChatEntry) string {
	width := c.width - 6
	if width < 20 {
		width = 20
	}

	// TS pi-mono: border dim for !! (excluded); header in bashMode per updateDisplay.
	// pi-mono constructor initially dims both for !! but updateDisplay (called on
	// first output) re-creates the header in bashMode regardless of !! status.
	borderColor := c.bashBorder
	headerStyle := c.bashHeader
	outputStyle := c.bashOutput
	if e.BashExcluded {
		borderColor = c.bashOutput.Copy()
	}

	var sb strings.Builder

	// DynamicBorder top (TS pi-mono: DynamicBorder)
	sb.WriteString(borderColor.Render(strings.Repeat("─", width)))
	sb.WriteByte('\n')

	// Command header (TS pi-mono: "$ command")
	cmdDisplay := e.BashCommand
	if lipgloss.Width(cmdDisplay) > width-4 {
		cmdDisplay = TruncateByWidth(cmdDisplay, width-7) + "..."
	}
	sb.WriteString("  " + headerStyle.Render("$ "+cmdDisplay))
	sb.WriteByte('\n')

	// Build visual (word-wrapped) lines for width-aware truncation (TS pi-mono: visual-truncate)
	previewLines := 20
	contentWidth := width - 2
	if contentWidth < 10 {
		contentWidth = 10
	}
	var visualLines []string
	for _, line := range e.BashLines {
		if lipgloss.Width(line) <= contentWidth {
			visualLines = append(visualLines, line)
		} else {
			visualLines = append(visualLines, strings.Split(wordWrap(line, contentWidth), "\n")...)
		}
	}

	hiddenCount := 0
	visibleLines := visualLines
	if !e.Expanded && len(visualLines) > previewLines {
		hiddenCount = len(visualLines) - previewLines
		visibleLines = visualLines[len(visualLines)-previewLines:]
	}

	// Output lines (TS pi-mono: 1-space indent, muted color)
	for _, line := range visibleLines {
		displayLine := line
		if lipgloss.Width(displayLine) > contentWidth {
			displayLine = TruncateByWidth(displayLine, contentWidth-3) + "..."
		}
		sb.WriteString("  " + outputStyle.Render(displayLine))
		sb.WriteByte('\n')
	}

	// Status line
	if e.BashRunning {
		runningText := "  " + c.spinnerChar() + " Running... (Esc to cancel)"
		sb.WriteString(outputStyle.Render(runningText))
	} else {
		cancelled := e.BashExitCode == -1

		var statusParts []string

		// Hidden lines info (shown regardless of exit code)
		if hiddenCount > 0 {
			if e.Expanded {
				statusParts = append(statusParts, c.bashOutput.Render("("+c.toggleKey()+" to collapse)"))
			} else {
				statusParts = append(statusParts, c.bashOutput.Render(fmt.Sprintf("... %d more lines ("+c.toggleKey()+" to expand)", hiddenCount)))
			}
		}

		// Exit status: suppressed for exit 0 (TS pi-mono: no status for success)
		if cancelled {
			statusParts = append(statusParts, c.warningStyle.Render("(cancelled)"))
		} else if e.BashExitCode != 0 {
			statusParts = append(statusParts, c.bashErrorStatus.Render(fmt.Sprintf("(exit %d)", e.BashExitCode)))
		}

		// Truncation warning inline in border (TS pi-mono: Output truncated. Full output: {path})
		if e.BashTruncated && e.BashFullOutputPath != "" {
			statusParts = append(statusParts, c.warningStyle.Render("Output truncated. Full output: "+e.BashFullOutputPath))
		}

		if len(statusParts) > 0 {
			sb.WriteString("  " + strings.Join(statusParts, "\n  "))
		}
	}
	sb.WriteByte('\n')

	// DynamicBorder bottom (TS pi-mono: DynamicBorder)
	sb.WriteString(borderColor.Render(strings.Repeat("─", width)))

	return sb.String()
}

// renderCustomMessageEntry renders a custom_message entry matching TS pi-mono components:
// CompactionSummaryMessageComponent, BranchSummaryMessageComponent, and generic CustomMessageComponent.
func (c *ChatViewport) renderCustomMessageEntry(e ChatEntry) string {
	labelOnBg := c.customMessageBg.Copy().Inherit(c.customMessageLabel)
	textOnBg := c.customMessageBg.Copy()
	dimOnBg := c.customMessageBg.Copy().Foreground(lipgloss.Color("#5c6370"))

	label := labelOnBg.Render("[" + e.CustomType + "]")
	toggleKey := c.toggleKey()

	switch e.CustomType {
	case "compaction":
		tokenStr := formatTokenCount(e.TokensBefore)
		if e.Expanded && e.Content != "" {
			labelLine := padLineToWidth(label, c.width, c.customMessageBg)
			spacerLine := padLineToWidth(textOnBg.Render(""), c.width, c.customMessageBg)
			mdContent := "**Compacted from " + tokenStr + " tokens**\n\n" + e.Content
			rendered, err := c.mdRenderer.Render(mdContent)
			if err != nil {
				rendered = wordWrap(mdContent, c.width-10)
			}
			rendered = strings.TrimSuffix(rendered, "\n")
			return labelLine + "\n" + spacerLine + "\n" + prefixedLineBg(" ", wrapURLsOSC8(rendered), c.width, c.customMessageBg)
		}
		msg := textOnBg.Render("Compacted from " + tokenStr + " tokens (")
		keyHint := dimOnBg.Render(toggleKey)
		suffix := textOnBg.Render(" to expand)")
		labelLine := padLineToWidth(label, c.width, c.customMessageBg)
		contentLine := padLineToWidth(textOnBg.Render(" "+msg+keyHint+suffix), c.width, c.customMessageBg)
		return labelLine + "\n" + contentLine

	case "branch":
		if e.Expanded && e.Content != "" {
			labelLine := padLineToWidth(label, c.width, c.customMessageBg)
			spacerLine := padLineToWidth(textOnBg.Render(""), c.width, c.customMessageBg)
			mdContent := "**Branch Summary**\n\n" + e.Content
			rendered, err := c.mdRenderer.Render(mdContent)
			if err != nil {
				rendered = wordWrap(mdContent, c.width-10)
			}
			rendered = strings.TrimSuffix(rendered, "\n")
			return labelLine + "\n" + spacerLine + "\n" + prefixedLineBg(" ", wrapURLsOSC8(rendered), c.width, c.customMessageBg)
		}
		msg := textOnBg.Render("Branch summary (")
		keyHint := dimOnBg.Render(toggleKey)
		suffix := textOnBg.Render(" to expand)")
		labelLine := padLineToWidth(label, c.width, c.customMessageBg)
		contentLine := padLineToWidth(textOnBg.Render(" "+msg+keyHint+suffix), c.width, c.customMessageBg)
		return labelLine + "\n" + contentLine

	case "skill":
		// TS pi-mono: SkillInvocationMessageComponent
		// Content format: "name\n\nfullSkillContent"
		name := e.Content
		fullContent := ""
		if idx := strings.Index(e.Content, "\n\n"); idx != -1 {
			name = e.Content[:idx]
			fullContent = e.Content[idx+2:]
		}
		if e.Expanded && fullContent != "" {
			labelLine := padLineToWidth(label, c.width, c.customMessageBg)
			spacerLine := padLineToWidth(textOnBg.Render(""), c.width, c.customMessageBg)
			mdContent := "**" + name + "**\n\n" + fullContent
			rendered, err := c.mdRenderer.Render(mdContent)
			if err != nil {
				return labelLine + "\n" + spacerLine + "\n" +
					prefixedLineBg(" ", wordWrap(mdContent, c.width-14), c.width, c.customMessageBg)
			}
			rendered = strings.TrimSuffix(rendered, "\n")
			return labelLine + "\n" + spacerLine + "\n" +
				prefixedLineBg(" ", wrapURLsOSC8(rendered), c.width, c.customMessageBg)
		}
		hint := dimOnBg.Render(" (" + toggleKey + " to expand)")
		return padLineToWidth(label+" "+textOnBg.Render(name)+hint, c.width, c.customMessageBg)

	default:
		// Generic handling for prompt, etc. (TS pi-mono: CustomMessageComponent)
		if e.Expanded && e.Content != "" {
			labelLine := padLineToWidth(label, c.width, c.customMessageBg)
			spacerLine := padLineToWidth(textOnBg.Render(""), c.width, c.customMessageBg)
			rendered, err := c.mdRenderer.Render(e.Content)
			if err != nil {
				return labelLine + "\n" + spacerLine + "\n" + prefixedLineBg(" ", wordWrap(e.Content, c.width-14), c.width, c.customMessageBg)
			}
			rendered = strings.TrimSuffix(rendered, "\n")
			return labelLine + "\n" + spacerLine + "\n" +
				prefixedLineBg(" ", wrapURLsOSC8(rendered), c.width, c.customMessageBg)
		}
		if e.Content != "" {
			firstLine := e.Content
			if idx := strings.IndexByte(firstLine, '\n'); idx != -1 {
				firstLine = firstLine[:idx]
			}
			if lipgloss.Width(firstLine) > 80 {
				firstLine = TruncateByWidth(firstLine, 77) + "..."
			}
			hint := dimOnBg.Render(" (" + toggleKey + " to expand)")
			return padLineToWidth(label+" "+textOnBg.Render(firstLine)+hint, c.width, c.customMessageBg)
		}
		return padLineToWidth(label, c.width, c.customMessageBg)
	}
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
			return path + formatLineRange(args)
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
			detail := " " + path + formatLineRange(e.ToolArgs)
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

// extractJSONIntField extracts an integer field from a JSON object string (TS pi-mono: line range parsing).
func extractJSONIntField(jsonStr, field string) (int, bool) {
	needle := `"` + field + `": `
	idx := strings.Index(jsonStr, needle)
	if idx < 0 {
		return 0, false
	}
	start := idx + len(needle)
	// Read until comma, whitespace, or closing brace
	end := start
	for end < len(jsonStr) && jsonStr[end] != ',' && jsonStr[end] != ' ' && jsonStr[end] != '\n' && jsonStr[end] != '}' {
		end++
	}
	val, err := strconv.Atoi(strings.TrimSpace(jsonStr[start:end]))
	if err != nil {
		return 0, false
	}
	return val, true
}

// formatLineRange returns a line range string like ":10-20" or ":51" matching TS pi-mono formatReadLineRange.
func formatLineRange(args string) string {
	offset, hasOffset := extractJSONIntField(args, "offset")
	limit, hasLimit := extractJSONIntField(args, "limit")
	if !hasOffset && !hasLimit {
		return ""
	}
	if !hasOffset {
		offset = 1
	}
	if hasLimit {
		return fmt.Sprintf(":%d-%d", offset, offset+limit-1)
	}
	return fmt.Sprintf(":%d", offset)
}

// padLineToWidth pads a line (which may contain ANSI codes) to the given visual width
// using background-colored spaces so the entire line has a uniform background.
func padLineToWidth(line string, width int, bg lipgloss.Style) string {
	vw := lipgloss.Width(line)
	if vw < width {
		line += bg.Render(strings.Repeat(" ", width-vw))
	}
	return line
}

// formatTokenCount formats a token count with comma separators (matching TS pi-mono toLocaleString).
func formatTokenCount(n int) string {
	if n <= 0 {
		return "?"
	}
	s := fmt.Sprintf("%d", n)
	var result []byte
	for i := len(s) - 1; i >= 0; i-- {
		result = append([]byte{s[i]}, result...)
		if (len(s)-i)%3 == 0 && i > 0 {
			result = append([]byte{','}, result...)
		}
	}
	return string(result)
}

// AppendCompactionSummary adds a compaction summary custom message with token count
// matching TS pi-mono CompactionSummaryMessageComponent.
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
