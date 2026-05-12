// Package tui provides the interactive terminal UI for xihu using Bubble Tea.
package tui

import (

	"sync"
	"os"
	"time"

	tea "github.com/charmbracelet/bubbletea"

	agentsession "github.com/huichen/xihu/internal/agentsession"
	"github.com/huichen/xihu/internal/events"
	"github.com/huichen/xihu/internal/extensions"
	"github.com/huichen/xihu/internal/session"
	"github.com/huichen/xihu/internal/settings"
	"github.com/huichen/xihu/internal/prompt"
	"github.com/huichen/xihu/internal/skills"
	"github.com/huichen/xihu/internal/tui/components"


)

// ─── Message Types ─────────────────────────────────────────────────────────

// StreamTextMsg is a chunk of streamed text from the LLM.

type StreamTextMsg string

// ThinkingMsg is a chunk of thinking/reasoning content.
type ThinkingMsg string

// ToolCallMsg announces a new tool call.
type ToolCallMsg struct {
	ID        string
	Name      string
	Arguments string
}

// ToolCallStartMsg announces a tool call has started streaming (args pending).
type ToolCallStartMsg struct {
	ID   string
	Name string
}

// ToolCallDeltaMsg delivers streaming tool call argument fragments.
type ToolCallDeltaMsg struct {
	ID   string
	Text string
}

// ToolRunningMsg signals that a tool has started executing (not just streaming args).
type ToolRunningMsg struct {
	ID   string
	Name string
}

// ToolResultMsg delivers the result of a tool execution.
type ToolResultMsg struct {
	ID         string
	Output     string
	Error      string
	DurationMs int64
}

// AgentDoneMsg signals the agent has finished processing.
type AgentDoneMsg struct {
	FinalText string
}

// StopReasonMsg carries the model's stop reason for display (TS pi-mono: stopReason).
type StopReasonMsg struct {
	Reason string
}

// AgentErrorMsg signals an error from the agent.
type AgentErrorMsg struct {
	Error error
}

// terminalInputRegistry manages extension terminal input handlers.
type terminalInputRegistry struct {
	mu       sync.Mutex
	handlers map[int]extensions.TerminalInputHandler
	nextID   int
}

type BashExecMsg struct {
	Command          string
	ExcludeFromCtx   bool // true for !! (excluded from LLM context)
}

// BashExecResultMsg carries the result of a direct bash execution (! command).
type BashExecResultMsg struct {
	Command        string
	Output         string
	ExitCode       int
	Cancelled      bool
	Truncated      bool
	FullOutputPath string // path to full output file when Truncated
}

// StatusMsg updates the footer status bar.
type StatusMsg struct {
	TokensIn      int
	TokensOut     int
	TokensCacheR  int
	TokensCacheW  int
	TotalCost     float64
	ContextUsed   float64 // 0.0 ~ 1.0 — context usage ratio (deprecated, use ContextTokens+ContextWin)
	ContextTokens int     // estimated current context size in tokens
	ContextWin    int     // context window size in tokens (denominator for %)
	Streaming     bool
}

// TickMsg advances spinner/loader animations (sent by tea.Tick).
type TickMsg time.Time

// BranchTickMsg triggers a git branch re-check (every 3s).
type BranchTickMsg time.Time

// RetryTickMsg advances the retry countdown (every 1s, sent by tea.Tick).
type RetryTickMsg time.Time

// ResizeMsg indicates terminal size change.
type ResizeMsg struct {
	Width  int
	Height int
}

// WelcomeMsg signals the app to display the startup banner.
type WelcomeMsg struct {
	ThemeAccent          string
	CWD                  string
	Skills               []skills.Skill
	Extensions           []string
	PromptTemplates      []prompt.PromptTemplate
	ContextFiles         []string // file paths of discovered context files
	SkillCollisions      []skills.SkillCollision
	ExtensionDiagnostics []extensions.ExtensionDiagnostic
	KeybindingConflicts  []KeybindingConflict
	SettingsError        string // error from settings/model loading at startup
}

// PromptCollision describes a naming conflict between two prompt templates.
type PromptCollision struct {
	Name        string
	WinnerPath  string
	LoserPath   string
}

// ThemeCollision describes a naming conflict between two themes.
type ThemeCollision struct {
	Name       string
	WinnerPath string
	LoserPath  string
}


// ─── App Model ─────────────────────────────────────────────────────────────

// AppModel is the root Bubble Tea model for the xihu TUI.
type AppModel struct {
	width  int
	height int

	// Sub-components
	chat         *components.ChatViewport
	footer       *components.Footer
	header       *components.Header
	input        *components.Editor
	overlay      *components.Overlay
	autocomplete *components.Autocomplete

	// Custom components provided by extensions (nil = use built-in)
	customFooter       FooterComponent
	customHeader       HeaderComponent
	customEditor       EditorComponent
	customEditorNeedsInit bool // true when factory set but instance not yet created

	// Agent state
	agent   *agentsession.AgentSession
	session *session.Session
	sessMgr *session.Manager

	// EventBus bridges agent streaming events to Bubble Tea messages
	eventBus *events.EventBus

	// Program reference for sending messages from goroutines
	program *tea.Program

	// Extension UI bridge for extensions to show dialogs
	extensionBridge *tuiExtensionBridge

	// Terminal input registry for extension key interception
	inputRegistry *terminalInputRegistry

	// Extension runner for command dispatch
	extRunner *extensions.ExtensionRunner

	// Extension statuses shown in the footer
	extensionStatuses map[string]string

	// Extension widgets rendered above/below the editor
	widgetsAbove map[string]string
	widgetsBelow map[string]string

	// Working indicator customization
	workingMessage    string   // default "Working..."
	workingVisible    bool
	workingFrames     []string // custom spinner frames
	workingIntervalMs int      // custom spinner interval

	// Loaded resources
	Skills          []skills.Skill
	SkillCollisions []skills.SkillCollision // diagnostics for dropped skills
	Extensions      []string
	settingsLoadErr string // error from settings/model loading at startup
	thinkingLevel                string
	currentModelSupportsThinking bool

	// Theme
	theme *Theme

	// Keybindings manager (user-configurable key mappings)
	keybindings *KeybindingsManager

	// Available models for cycling (TS: app.model.cycleForward)
	availableModels []string
	modelIndex      int

	// Scoped models: if non-empty, only these models are cycled via Ctrl+P / Ctrl+Shift+P.
	// Maps model string (as stored in availableModels) to enabled state.
	scopedModels map[string]bool
	modelOrder   []string // preferred cycling order (TS pi-mono: scoped model ordering)

	// Spinner animation
	spinnerFrame int

	// Derived state
	streaming  bool
	compacting        bool
	compactionQueue   []string // messages queued during compaction, flushed on end
	retryTicking      bool     // true while retry countdown is active
	retryDelaySec     int      // remaining seconds in retry countdown
	retryAttempt      int      // current retry attempt number
	retryMaxAttempts  int      // max retry attempts
	quitting          bool

	// Accumulated stats across agent runs
	lastStatus StatusMsg

	// streamID is an atomic counter that changes on each new submission / interrupt.
	// The EventBus forwarding goroutine checks it to discard events from stale streams.
	streamID int32

	// Help overlay state
	welcomeExpanded bool
	lastWelcomeMsg  *WelcomeMsg // stored for rebuild on toggle

	// Tree selector transient state
	treeFoldedNodes    map[string]bool
	treeFilterMode     string // "default", "no-tools", "user-only", "labeled-only", "all"
	treeSearchQuery    string
	treeShowTimestamps bool     // Shift+T toggle
	treeItemIndents    []int    // indent levels for tree items (branch navigation)

	// Runtime settings (TS pi-mono: SettingsConfig)
	settingsObj       *settings.Settings // reference for persistence
	autoCompact        bool
	doubleEscapeAction string // "tree", "fork", "none"
	defaultTreeFilter  string // "default", "no-tools", "user-only", "labeled-only", "all"
	quietStartup       bool   // suppress welcome message on startup
	clearOnShrink      bool   // clear editor when terminal shrinks
	steeringMode       string // "one-at-a-time" or "all"
	followUpMode       string // "one-at-a-time" or "all"
	transport          string // "sse", "websocket", "websocket-cached", "auto"
	showHardwareCursor bool   // show terminal block cursor
	terminalProgress   bool   // show terminal progress messages
	progressCancel     chan struct{} // cancel OSC 9;4 keepalive goroutine
	writeLogFile       *os.File // TUI write log for debugging (XIHU_TUI_WRITE_LOG)
	skillCommands      bool   // enable slash-command skill invocation
	showImages         bool   // show images in terminal
	imageWidthCells    int    // image width in cells (60, 80, 120)
	autoResizeImages   bool   // auto-resize images on terminal resize
	blockImages        bool   // block image rendering
	promptTemplates    []prompt.PromptTemplate // loaded prompt templates (/:name)
	contextFiles       []string // discovered context file paths
	installTelemetry   bool   // opt-in installation telemetry
	collapseChangelog  bool   // show condensed changelog
	editorPadding      int    // 0, 1, 2, 3
	autocompleteMax    int    // 3, 5, 7, 10, 15, 20
	anthropicExtraUsage bool  // warn on anthropic extra usage pricing
	lastChangelogVersion   string // tracks last viewed changelog version

	// Double-escape tracking
	lastEscapeTime time.Time

	// gg (double-g) jump-to-top tracking
	lastGTime time.Time

	// Ctrl+C double-press guard (TS pi-mono: exit on second press within 500ms)
	lastCtrlCTime time.Time

	// Bash cancel channel for Esc/Ctrl+C during direct bash execution
	bashCancelCh chan struct{}

	// Recent pending message texts for display (TS pi-mono: pendingMessagesContainer)
		pendingSteeringMsgs  []string
		pendingFollowUpMsgs []string

	// Live git branch tracking
	gitBranch string
}

// NewAppModel creates a new AppModel.
type refreshWarningsMsg struct{}

// showThinkingSelector opens a thinking level submenu (TS pi-mono: settings submenu).
type refreshScopedSelectorMsg struct{}

// refreshSettingsMsg is an internal message to refresh the settings selector.
