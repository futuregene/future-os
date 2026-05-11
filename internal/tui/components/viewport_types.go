package components

import (
	"sync"

	"github.com/charmbracelet/bubbles/viewport"
	"github.com/charmbracelet/glamour"
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

	// Message components: each message type has its own independent component class,
	// matching TS pi-mono's pattern (AssistantMessageComponent, ToolExecutionComponent, etc.)
	componentBase MessageComponentBase
	assistantComp AssistantMessageComponent
	thinkingComp  ThinkingMessageComponent
	toolCallComp  ToolCallComponent
	toolResultComp ToolResultComponent
	bashComp      BashExecutionComponent
	errorComp     ErrorMessageComponent
	systemComp    SystemMessageComponent
	customMsgComp CustomMessageComponent
	userMsgComp   UserMessageComponent
}

// newGlamourRenderer creates a glamour terminal renderer with consistent settings.
