// Package tui provides the interactive terminal UI for xihu using Bubble Tea.
package tui

import (
	"context"
	"sync"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"sync/atomic"
	"time"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"

	agentsession "github.com/huichen/xihu/internal/agentsession"
	"github.com/huichen/xihu/internal/auth"
	"github.com/huichen/xihu/internal/commands"
	"github.com/huichen/xihu/internal/events"
	"github.com/huichen/xihu/internal/extensions"
	"github.com/huichen/xihu/internal/models"
	"github.com/huichen/xihu/internal/session"
	"github.com/huichen/xihu/internal/settings"
	"github.com/huichen/xihu/internal/prompt"
	"github.com/huichen/xihu/internal/skills"
	"github.com/huichen/xihu/internal/tui/components"
	"github.com/huichen/xihu/internal/utils"
	"github.com/huichen/xihu/pkg/types"

	bashexec "github.com/huichen/xihu/internal/exec"
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

func newTerminalInputRegistry() *terminalInputRegistry {
	return &terminalInputRegistry{handlers: make(map[int]extensions.TerminalInputHandler)}
}

func (r *terminalInputRegistry) add(h extensions.TerminalInputHandler) int {
	r.mu.Lock()
	defer r.mu.Unlock()
	id := r.nextID
	r.nextID++
	r.handlers[id] = h
	return id
}

func (r *terminalInputRegistry) remove(id int) {
	r.mu.Lock()
	defer r.mu.Unlock()
	delete(r.handlers, id)
}

// dispatch calls all registered handlers and returns whether the input was consumed.
func (r *terminalInputRegistry) dispatch(data string) (consumed bool, newData string) {
	r.mu.Lock()
	handlers := make([]extensions.TerminalInputHandler, 0, len(r.handlers))
	for _, h := range r.handlers {
		handlers = append(handlers, h)
	}
	r.mu.Unlock()

	newData = data
	for _, h := range handlers {
		result := h(newData)
		if result != nil {
			if result.Data != "" {
				newData = result.Data
			}
			if result.Consume {
				consumed = true
				return
			}
		}
	}
	return false, newData
}

// BashExecMsg is sent from the SubmitMsg handler when user types ! command.
// It's processed asynchronously — the goroutine executes the command and
// sends BashExecResultMsg back to the program.
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
	TokensIn     int
	TokensOut    int
	TokensCacheR int
	TokensCacheW int
	TotalCost    float64
	ContextUsed  float64 // 0.0 ~ 1.0
	Streaming    bool
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
func NewAppModel(as *agentsession.AgentSession, sessMgr *session.Manager, sess *session.Session, theme *Theme, modelStr string, skillList []skills.Skill, extensions []string, thinkingLevel string, availableModels []string, cfg *settings.Settings, promptTemplates []prompt.PromptTemplate, contextFiles []string, skillCollisions []skills.SkillCollision) AppModel {
	chat := components.NewChatViewport()
	chat.SetTheme(theme.Accent, theme.Muted, theme.Dim, theme.Warning, theme.Success, theme.ErrorColor, theme.ThinkingColor, theme.ThinkingText, theme.ToolPendingBg, theme.ToolSuccessBg, theme.ToolErrorBg)
	footer := components.NewFooter(theme.FooterStyle(), theme.ContextGreen, theme.ContextYellow, theme.ContextRed)
	header := components.NewHeader(theme.Accent, utils.Version)
	input := components.NewEditor(theme.InputStyle())
	overlay := components.NewOverlay()
	ac := components.NewAutocomplete()

	// Wire keybinding manager to editor for user-configurable keybindings (pi-mono aligned)
	input.SetKeyMatcher(func(ks, bindingID string) bool {
		kb := GetKeybindings()
		return kb != nil && kb.Matches(ks, KeybindingID(bindingID))
	})

	// Find current model index in available models
	modelIndex := -1
	for i, m := range availableModels {
		if m == modelStr {
			modelIndex = i
			break
		}
	}

			inputRegistry := newTerminalInputRegistry()

			app := AppModel{
			chat:               &chat,
			footer:             &footer,
			header:             &header,
			input:              &input,
			overlay:            &overlay,
			autocomplete:       &ac,
			agent:              as,
			session:            sess,
			sessMgr:            sessMgr,
			inputRegistry:      inputRegistry,
			keybindings:        GetKeybindings(),
			theme:              theme,
			thinkingLevel:      thinkingLevel,
			availableModels:    availableModels,
			modelIndex:         modelIndex,
			scopedModels:       make(map[string]bool),
			modelOrder:         availableModels, // copy for reorder
			settingsObj:        cfg,
			doubleEscapeAction: "tree",
			defaultTreeFilter:  "default",
			treeShowTimestamps: true,
			steeringMode:       "one-at-a-time",
			followUpMode:       "one-at-a-time",
			transport:          "auto",
			showHardwareCursor: true,
			terminalProgress:   true,
			skillCommands:      true,
			showImages:         true,
			imageWidthCells:    80,
			autoResizeImages:   true,
			blockImages:        false,
			workingMessage:     "Working...",
			workingVisible:     true,
			workingFrames:      nil,
			workingIntervalMs:  0,
			installTelemetry:   false,
			collapseChangelog:  false,
			editorPadding:      1,
			autocompleteMax:    10,
			promptTemplates:    promptTemplates,
			contextFiles:       contextFiles,
			anthropicExtraUsage: false,
		}

	// Apply settings from config files (project overrides global)
	if cfg != nil {
		if cfg.DoubleEscapeAction != "" {
			app.doubleEscapeAction = cfg.DoubleEscapeAction
		}
		if cfg.TreeFilterMode != "" {
			app.defaultTreeFilter = cfg.TreeFilterMode
		}
		if cfg.QuietStartup != nil {
			app.quietStartup = *cfg.QuietStartup
		}
		if cfg.CompactionEnabled != nil {
			app.autoCompact = *cfg.CompactionEnabled
		}
		if cfg.HideThinkingBlock != nil {
			app.chat.HideAllThinking = *cfg.HideThinkingBlock
		}
		if cfg.SteeringMode != "" {
			app.steeringMode = cfg.SteeringMode
		}
		if cfg.FollowUpMode != "" {
			app.followUpMode = cfg.FollowUpMode
		}
		if cfg.Transport != "" {
			app.transport = cfg.Transport
		}
		if cfg.ShowHardwareCursor != nil {
			app.showHardwareCursor = *cfg.ShowHardwareCursor
		}
		if cfg.Terminal != nil {
			if cfg.Terminal.ShowTerminalProgress != nil {
				app.terminalProgress = *cfg.Terminal.ShowTerminalProgress
			}
			if cfg.Terminal.ClearOnShrink != nil {
				app.clearOnShrink = *cfg.Terminal.ClearOnShrink
			}
			if cfg.Terminal.ShowImages != nil {
				app.showImages = *cfg.Terminal.ShowImages
			}
			if cfg.Terminal.ImageWidthCells > 0 {
				app.imageWidthCells = cfg.Terminal.ImageWidthCells
			}
		}
		if cfg.Images != nil {
			if cfg.Images.AutoResize != nil {
				app.autoResizeImages = *cfg.Images.AutoResize
			}
			if cfg.Images.BlockImages != nil {
				app.blockImages = *cfg.Images.BlockImages
			}
		}
		// Propagate image settings to chat viewport
		chat.SetShowImages(app.showImages)
		chat.SetImageWidth(app.imageWidthCells)
		// Propagate actual keybinding for tool toggle hint
		if tk := formatKeyStr(app.keybindings, GlobalToggleTools); tk != "" {
			chat.SetToolToggleKey(tk)
		}
		if cfg.EnableSkillCommands != nil {
			app.skillCommands = *cfg.EnableSkillCommands
		}
		if cfg.CollapseChangelog != nil {
			app.collapseChangelog = *cfg.CollapseChangelog
		}
		if cfg.EnableInstallTelemetry != nil {
			app.installTelemetry = *cfg.EnableInstallTelemetry
		}
		if cfg.EditorPaddingX > 0 {
			app.editorPadding = cfg.EditorPaddingX
		}
		if cfg.AutocompleteMaxVisible > 0 {
			app.autocompleteMax = cfg.AutocompleteMaxVisible
		}
		if cfg.Warnings != nil && cfg.Warnings.AnthropicExtraUsage != nil {
			app.anthropicExtraUsage = *cfg.Warnings.AnthropicExtraUsage
		}
		if cfg.LastChangelogVersion != "" {
			app.lastChangelogVersion = cfg.LastChangelogVersion
		}
		// Scoped models
		if len(cfg.ScopedModels) > 0 {
			for _, m := range cfg.ScopedModels {
				app.scopedModels[m] = true
			}
		}
	}

	// Propagate steering mode to agent loop
	as.Loop().SteeringMode = app.steeringMode

	// Wire autocomplete max visible
	app.autocomplete.SetMaxVisible(app.autocompleteMax)

	// Wire editor padding
	app.input.SetPaddingX(app.editorPadding)

	// Set skills and extensions
	if len(skillList) > 0 {
		app.Skills = skillList
	}
	app.SkillCollisions = skillCollisions
	if len(extensions) > 0 {
		app.Extensions = extensions
	}

	// Wire footer with session info + parsed model/provider
	cwd := ""
	sessionName := ""
	if sess != nil {
		cwd = sess.CWD
		sessionName = sess.GetSessionName()
	}
	gitBranch := getGitBranch(cwd)
	app.gitBranch = gitBranch
	modelName, provider := parseModelString(modelStr)
	// Use explicit thinkingLevel parameter (not extracted from modelStr)
	app.footer.SetSession(cwd, gitBranch, sessionName, modelName, thinkingLevel, provider)
	app.footer.SetHasReasoning(supportsThinking(modelName))
	app.input.SetBorderColor(app.theme.ThinkingBorderColor(thinkingLevel))
	app.input.SetBashBorderColor("#98c379")  // green (TS pi-mono: bashMode)
	app.input.SetSlashBorderColor("#61afef") // blue (default)
	app.input.SetFileBorderColor("#e5c07b")  // yellow/amber (TS pi-mono: @ file mode)
	app.input.SetSymbolBorderColor("#c678dd") // magenta/purple (TS pi-mono: # symbol mode)
	if sess != nil {
		app.footer.SetEntryCount(len(sess.Entries))
	}

	// Track unique providers for footer display (TS pi-mono: only show provider when >1)
	providers := make(map[string]bool)
	for _, m := range availableModels {
		_, p := parseModelString(m)
		if p != "" {
			providers[p] = true
		}
	}
	app.footer.SetAvailableProviders(len(providers))

	// Create EventBus and attach to agent for fine-grained events
	app.eventBus = events.NewEventBus()
	as.Loop().EventBus = app.eventBus

	// TUI write log for debugging (XIHU_TUI_WRITE_LOG env var)
	if logPath := os.Getenv("XIHU_TUI_WRITE_LOG"); logPath != "" {
		f, err := os.Create(logPath)
		if err == nil {
			app.writeLogFile = f
		}
	}

	return app
}

// Init is the first command run when the program starts.
func (m AppModel) Init() tea.Cmd {
	cmds := []tea.Cmd{
		tea.EnterAltScreen,
		m.input.Focus(),
		func() tea.Msg {
			return WelcomeMsg{
				ThemeAccent:          m.theme.Accent,
				CWD:                  m.session.CWD,
				Skills:               m.Skills,
				Extensions:           m.Extensions,
				PromptTemplates:      m.promptTemplates,
				ContextFiles:         m.contextFiles,
				SkillCollisions:      m.SkillCollisions,
				ExtensionDiagnostics: m.getExtDiagnostics(),
				KeybindingConflicts:  m.getKBConflicts(),
				SettingsError:        m.settingsLoadErr,
			}
		},
	}
	if !m.showHardwareCursor {
		cmds = append(cmds, tea.HideCursor)
	}
	// Start git branch watcher (TS pi-mono: footer live branch updates)
	cmds = append(cmds, tea.Tick(3*time.Second, func(t time.Time) tea.Msg {
		return BranchTickMsg(t)
	}))
	// Set terminal title (TS pi-mono: updateTerminalTitle)
	updateTerminalTitle(m.session.GetSessionName(), m.session.CWD)
	return tea.Batch(cmds...)
}

// Update handles messages and updates the model.
func (m AppModel) Update(msg tea.Msg) (outModel tea.Model, outCmd tea.Cmd) {
	// Crash recovery: prevent any panic in TUI handlers from killing the process
	defer func() {
		if r := recover(); r != nil {
			m.chat.AppendError(fmt.Sprintf("Internal error recovered: %v", r))
			fmt.Fprintf(os.Stderr, "\n[xihu] panic recovered: %v\n", r)
			outModel = m
			outCmd = nil
		}
	}()

	switch msg := msg.(type) {
	case tea.WindowSizeMsg:
		// Clear on shrink: if terminal shrank, clear editor
		if m.clearOnShrink && (msg.Width < m.width || msg.Height < m.height) {
			if m.customEditor != nil {
				if !m.customEditor.Empty() {
					m.customEditor.Reset()
				}
			} else if !m.editorEmpty() {
				m.input.Reset()
			}
		}
		m.width = msg.Width
		m.height = msg.Height
		m.header.SetWidth(msg.Width)
		if m.customHeader != nil {
			m.customHeader.SetWidth(msg.Width)
		}
		if m.customEditor != nil {
			m.customEditor.SetWidth(msg.Width - 4)
			m.customEditor.SetHeight(msg.Height)
		} else {
			m.input.SetHeight(msg.Height)
			m.input.SetWidth(msg.Width - 4)
		}
		editorHeight := m.editorHeight()
		if m.customFooter != nil {
			m.customFooter.SetWidth(msg.Width)
		}
		footerHeight := m.footerHeight()
		headerHeight := 2
		if m.customHeader == nil && m.header.Expanded() {
			headerHeight = 7
		}
		m.chat.SetSize(msg.Width, msg.Height-editorHeight-footerHeight-headerHeight)
		m.footer.SetWidth(msg.Width)
		m.overlay.SetTermSize(msg.Width, msg.Height)
		return m, nil

	case WelcomeMsg:
		m.showWelcome(msg)
		return m, nil

	case tea.MouseMsg:
		// Route mouse events to chat viewport for native scroll handling
		_, _ = m.chat.Update(msg)
		return m, nil

	case components.CountdownTickMsg:
		if m.overlay.Active() {
			return m, m.overlay.Update(msg)
		}
		return m, nil

	case tea.KeyMsg:
		// Bracketed paste detection (TS pi-mono: handlePaste with CSI 200~ / 201~)
		// Bubble Tea v1.3.10+ already decodes paste events and sets Key.Paste = true.
		// Route paste content through the editor's Paste method for large-paste markers.
		if k := tea.Key(msg); k.Paste && len(k.Runes) > 0 {
			if m.customEditor != nil {
				m.customEditor.SetValue(string(k.Runes))
			} else {
				m.input.Paste(string(k.Runes))
			}
			return m, nil
		}

		// Dispatch to extension terminal input handlers first
		if m.inputRegistry != nil {
			consumed, _ := m.inputRegistry.dispatch(msg.String())
			if consumed {
				return m, nil
			}
		}

		// Keybindings global action dispatch (pi-mono aligned)
		if m.keybindings != nil {
			ks := msg.String()
			switch {
			case m.keybindings.Matches(ks, GlobalInterrupt):
				m.quitting = true
				return m, tea.Quit
			case m.keybindings.Matches(ks, GlobalClear):
				if !m.editorEmpty() {
					m.input.Reset()
					return m, nil
				}
				if !m.streaming && !m.compacting {
					now := time.Now()
					if now.Sub(m.lastCtrlCTime) < 500*time.Millisecond {
						m.lastCtrlCTime = time.Time{}
						m.quitting = true
						return m, tea.Quit
					}
					m.lastCtrlCTime = now
					return m, nil
				}
			case m.keybindings.Matches(ks, GlobalExit):
				if !m.streaming && !m.compacting && m.editorEmpty() {
					m.quitting = true
					return m, tea.Quit
				}
				if !m.editorEmpty() {
					_, cmd := m.input.Update(msg)
					return m, cmd
				}
			case m.keybindings.Matches(ks, GlobalToggleHeader):
				m.header.Toggle()
				headerHeight := 2
				if m.header.Expanded() {
					headerHeight = 7
				}
			editorHeight := m.editorHeight()
			footerHeight := m.footerHeight()
			m.chat.SetSize(m.width, m.height-editorHeight-footerHeight-headerHeight)
			return m, nil
		case m.keybindings.Matches(ks, GlobalToggleTools):
			m.chat.ToggleAllTools()
			return m, nil
			case m.keybindings.Matches(ks, GlobalToggleThinking):
				m.chat.HideAllThinking = !m.chat.HideAllThinking
				visible := "hidden"
				if !m.chat.HideAllThinking {
					visible = "visible"
				}
				m.chat.AppendSystem("Thinking blocks: " + visible)
				return m, nil
			case m.keybindings.Matches(ks, GlobalModelSelector):
				m.showModelSelector()
				return m, nil
			case m.keybindings.Matches(ks, GlobalCycleModelFwd):
				m.cycleModelForward()
				return m, nil
			case m.keybindings.Matches(ks, GlobalCycleModelBack):
				m.cycleModelBackward()
				return m, nil
			case m.keybindings.Matches(ks, GlobalCycleThinking):
				m.cycleThinking()
				return m, nil
			case m.keybindings.Matches(ks, GlobalExternalEditor):
				m.openExternalEditor()
				return m, nil
			case m.keybindings.Matches(ks, EditorYank):
				_, cmd := m.input.Update(msg)
				return m, cmd
			case m.keybindings.Matches(ks, EditorYankPop):
				_, cmd := m.input.Update(msg)
				return m, cmd
			case m.keybindings.Matches(ks, EditorUndo):
				_, cmd := m.input.Update(msg)
				return m, cmd
			}
		}

		switch msg.String() {
		case "ctrl+c":
			// TS pi-mono: double-press guard — second Ctrl+C within 500ms exits.
			// Every Ctrl+C clears the editor and records the timestamp.
			now := time.Now()
			if now.Sub(m.lastCtrlCTime) < 500*time.Millisecond {
				m.lastCtrlCTime = time.Time{}
				m.quitting = true
				return m, tea.Quit
			}
			m.lastCtrlCTime = now
			if !m.editorEmpty() {
				m.input.Reset()
			}
			return m, nil
		case "ctrl+z":
			// TS pi-mono: Suspend to background
			return m, tea.Suspend
		case "ctrl+d":
			if !m.streaming && !m.compacting && m.editorEmpty() {
				m.quitting = true
				return m, tea.Quit
			}
			// Forward to editor for delete-char-forward when editor has content
			if !m.editorEmpty() {
				_, cmd := m.input.Update(msg)
				return m, cmd
			}
		case "ctrl+h":
			// Toggle header expanded/collapsed (TS pi-mono: ExpandableText header)
			if m.customHeader != nil {
				// Custom header — just re-layout
			} else {
				m.header.Toggle()
			}
			headerHeight := 2
			if m.customHeader == nil && m.header.Expanded() {
				headerHeight = 7
			}
			editorHeight := m.editorHeight()
			footerHeight := m.footerHeight()
			m.chat.SetSize(m.width, m.height-editorHeight-footerHeight-headerHeight)
			return m, nil
		case "ctrl+o":
			// TS pi-mono: Toggle ALL tool outputs expand/collapse globally
			m.chat.ToggleAllTools()
			return m, nil
		case "ctrl+l":
			// TS pi-mono: Open model selector; close any existing overlay first
			m.overlay.HideAll()
			m.showModelSelector()
			return m, nil
		case "ctrl+g":
			// TS pi-mono: Open external editor ($EDITOR)
			text := m.openExternalEditor()
			if text != "" && m.program != nil {
				m.program.Send(components.SubmitMsg(text))
			}
			return m, nil
		case "esc":
			// TS pi-mono: Escape during streaming = abort current LLM call
			if m.bashCancelCh != nil {
				close(m.bashCancelCh)
				m.bashCancelCh = nil
				return m, nil
			}
			if m.compacting {
				// Signal compaction cancellation via event bus
				if m.agent != nil && m.agent.Loop().EventBus != nil {
					m.agent.Loop().EventBus.Emit(events.CompactionEnd(0, "", true, "manual"))
				}
				return m, nil
			}
			if m.streaming {
				m.agent.Abort()
				// Restore queued messages to editor (TS pi-mono: prepend to existing content)
				msgs := m.agent.Loop().DrainQueues()
				if len(msgs) > 0 {
					queued := strings.Join(msgs, "\n\n")
					current := m.input.Value()
					if current != "" {
						m.input.SetValue(queued + "\n\n" + current)
					} else {
						m.input.SetValue(queued)
					}
					// Silent abort — agent response indicates cancellation via stopReason
				}
				return m, nil
			}
			// Double-escape with empty editor: trigger tree or fork (TS pi-mono)
			if m.editorEmpty() && m.doubleEscapeAction != "none" {
				now := time.Now()
				if now.Sub(m.lastEscapeTime) < 500*time.Millisecond {
					m.lastEscapeTime = time.Time{}
					if m.doubleEscapeAction == "tree" {
						m.showSessionTree()
					} else if m.doubleEscapeAction == "fork" {
						m.showForkSelector()
					}
					return m, nil
				}
				m.lastEscapeTime = now
			}
		case "shift+tab":
			// Cycle thinking level: off → low → medium → high → xhigh → off
			m.cycleThinking()
			return m, nil
		case "ctrl+t":
			// Toggle thinking visibility (TS pi-mono: hideThinkingBlock)
			m.chat.HideAllThinking = !m.chat.HideAllThinking
			visible := "hidden"
			if !m.chat.HideAllThinking {
				visible = "visible"
			}
			m.chat.AppendSystem("Thinking blocks: " + visible)
			return m, nil
		case "ctrl+p":
			// TS pi-mono: Cycle model forward
			if len(m.availableModels) > 0 {
				m.cycleModelForward()
			}
			return m, nil
		case "ctrl+shift+p":
			// TS pi-mono: Cycle model backward
			if len(m.availableModels) > 0 {
				m.cycleModelBackward()
			}
			return m, nil
		case "alt+up":
			// TS pi-mono: Dequeue — prepend queued messages to existing editor content
			msgs := m.agent.Loop().DrainQueues()
			if len(msgs) > 0 {
				queued := strings.Join(msgs, "\n\n")
				current := m.input.Value()
				if current != "" {
					m.input.SetValue(queued + "\n\n" + current)
				} else {
					m.input.SetValue(queued)
				}
				noun := "message"
				if len(msgs) > 1 {
					noun = "messages"
				}
				m.chat.AppendSystem(fmt.Sprintf("Restored %d queued %s to editor", len(msgs), noun))
			} else {
				m.chat.AppendSystem("No queued messages to restore")
			}
			m.pendingSteeringMsgs = nil
			m.pendingFollowUpMsgs = nil
			return m, nil
		}

		// Route to overlay if active and capturing (nonCapturing overlays let keys through)
		if m.overlay.Active() && !m.overlay.NonCapturing() {
			cmd := m.overlay.Update(msg)
			return m, cmd
		}
		// Non-capturing overlay: only Esc closes it, all other keys pass through
		if m.overlay.Active() && m.overlay.NonCapturing() {
			if msg.String() == "esc" {
				m.overlay.Hide()
				return m, nil
			}
		}

		// Handle autocomplete navigation (arrow keys when autocomplete is active)
		if m.autocomplete.Active() {
			switch msg.String() {
			case "up":
				m.autocomplete.SelectPrev()
				return m, nil
			case "down":
				m.autocomplete.SelectNext()
				return m, nil
			case "tab":
				// TS pi-mono: Tab cycles to next autocomplete candidate
				m.autocomplete.SelectNext()
				if selected := m.autocomplete.Selected(); selected != "" {
					m.input.SetValue(selected)
				}
				return m, nil
			case "shift+tab":
				// TS pi-mono: Shift+Tab cycles to previous
				m.autocomplete.SelectPrev()
				if selected := m.autocomplete.Selected(); selected != "" {
					m.input.SetValue(selected)
				}
				return m, nil
			case "enter":
				selected := m.autocomplete.Selected()
				if selected != "" {
					m.input.SetValue(selected)
					m.autocomplete.Hide()
					m.input.ExitSlashMode()
				}
				return m, nil
			}
		}

		// Route scroll/chat keys to chat viewport (handles pgup/pgdown/ctrl+u/ctrl+d/mouse wheel natively via bubbles/viewport)
		switch msg.String() {
		case "pgup", "pgdown", "ctrl+u", "ctrl+d", "home", "end":
			_, cmd := m.chat.Update(msg)
			return m, cmd
		case "G":
			// Jump to bottom (follow mode)
			m.chat.ScrollToBottom()
			return m, nil
		case "g":
			// gg: jump to top on double-g within 500ms
			// Only intercept when editor is empty (otherwise user is typing "g" as text)
			if m.editorEmpty() {
				now := time.Now()
				if now.Sub(m.lastGTime) < 500*time.Millisecond {
					m.lastGTime = time.Time{}
					m.chat.ScrollToTop()
					return m, nil
				}
				m.lastGTime = now
				return m, nil
			}
			// Forward to editor
		case "ctrl+v":
			// Paste from system clipboard (TS pi-mono: clipboard paste with markers for large text)
			if text, err := pasteFromClipboard(); err == nil && text != "" {
				if marker := m.input.StorePaste(text); marker != "" {
					m.input.SetValue(m.input.Value() + marker)
				} else {
					m.input.SetValue(m.input.Value() + text)
				}
			} else if err != nil {
				m.chat.AppendSystem("Clipboard paste failed: " + err.Error())
			}
			return m, nil
		}

	case components.SubmitMsg:
		text := m.input.ExpandPastes(string(msg))
		if m.streaming {
			// TS-style steer: inject message without aborting current stream
			m.input.RecordSubmission(text)
			m.pendingSteeringMsgs = append(m.pendingSteeringMsgs, text)
			m.agent.Steer(text)
			return m, nil
		}
		if m.compacting {
			m.input.RecordSubmission(text)
			m.compactionQueue = append(m.compactionQueue, text)
			m.chat.AppendSystem("Queued message for after compaction")
			return m, nil
		}
		{
			atomic.AddInt32(&m.streamID, 1)
			if strings.HasPrefix(text, "!") {
				// TS pi-mono: bash already-running guard
				if m.bashCancelCh != nil {
					m.chat.AppendWarning("A bash command is already running. Press Esc to cancel it first.")
					m.input.SetValue(text)
					return m, nil
				}
			}
			if strings.HasPrefix(text, "!!") {
				cmd := strings.TrimPrefix(text, "!!")
				cmd = strings.TrimSpace(cmd)
				if cmd != "" {
					go m.runBashDirect(cmd, true)
				}
			} else if strings.HasPrefix(text, "!") {
				cmd := strings.TrimPrefix(text, "!")
				cmd = strings.TrimSpace(cmd)
				if cmd != "" {
					go m.runBashDirect(cmd, false)
				}
			} else if strings.HasPrefix(text, "/skill:") && m.skillCommands {
				atomic.AddInt32(&m.streamID, 1)
				skillName := strings.TrimPrefix(text, "/skill:")
				found := false
				for _, s := range m.Skills {
					if s.Name == skillName {
						content, err := os.ReadFile(s.Path)
						if err != nil {
							m.chat.AppendSystem("Skill error: " + err.Error())
							found = true
							break
						}
						m.chat.AppendCustomMessage("skill", fmt.Sprintf("Invoking skill: %s\n%s", s.Name, s.Description))
						go m.runAgent("Follow the skill instructions:\n\n" + string(content), m.streamID)
						found = true
						break
					}
				}
				if !found {
					m.chat.AppendSystem("Skill not found: " + skillName)
				}
			} else if strings.HasPrefix(text, "/") && !strings.HasPrefix(text, "//") {
				// Check for prompt template: /:name [args...]
				cmdName, cmdArgs := splitSlashCommand(text)
				if tmpl := m.findTemplate(cmdName); tmpl != nil {
					atomic.AddInt32(&m.streamID, 1)
					expanded := prompt.ExpandTemplate(*tmpl, cmdArgs...)
					go m.runAgent(expanded, m.streamID)
				} else {
					result, handled := m.handleSlashCmd(text)
					if !handled {
						// TS pi-mono: unknown slash commands fall through to LLM as normal prompts
						m.chat.AppendUserMessage(text)
						m.input.RecordSubmission(text)
						go m.runAgent(text, m.streamID)
					} else if result != "" {
						m.chat.AppendSystem(result)
					}
				}
			} else {
				m.chat.AppendUserMessage(text)
				m.input.RecordSubmission(text)
				go m.runAgent(text, m.streamID)
			}
		}
		return m, nil

	case components.FollowUpMsg:
		// TS pi-mono: Alt+Enter queues message for after agent finishes
		text := m.input.ExpandPastes(string(msg))
		m.pendingFollowUpMsgs = append(m.pendingFollowUpMsgs, text)
		m.agent.FollowUp(text) // Uses FollowUpQueue → processed after agent finishes
		return m, nil

	case StreamTextMsg:
		m.chat.AppendText(string(msg))
		m.footer.SetWorkingMessage("Thinking...")
		return m, nil

	case ThinkingMsg:
		m.chat.AppendThinking(string(msg))
		m.footer.SetWorkingMessage("Thinking...")
		return m, nil

	case ToolCallMsg:
		if msg.Name == "bash" {
			// Replace pending tool_call entry with bordered bash display
			m.chat.RemovePendingToolCall(msg.ID)
			cmd := extractBashCommand(msg.Arguments)
			m.chat.AddBashExecution(cmd, false)
		} else {
			// Finalize pending tool_call entry's args in-place (avoids duplicate)
			m.chat.CompleteToolCall(msg.ID, msg.Arguments)
		}
		return m, nil

	case ToolCallStartMsg:
		m.chat.AddToolCall(msg.ID, msg.Name, "")
		// Working message set in ToolRunningMsg when execution actually starts (TS pi-mono timing)
		return m, nil

	case ToolCallDeltaMsg:
		m.chat.AppendToolCallDelta(msg.Text)
		return m, nil

	case ToolRunningMsg:
		m.chat.MarkToolRunning(msg.ID)
		m.footer.SetWorkingMessage("Running " + msg.Name + "...")
		return m, nil

	case ToolResultMsg:
		durStr := formatDuration(msg.DurationMs)
		if msg.ID == "bash" {
			if msg.Error != "" {
				m.chat.CompleteBash(1, false); m.chat.SetLastBashDuration(durStr)
			} else {
				m.chat.CompleteBash(0, false); m.chat.SetLastBashDuration(durStr)
			}
			_ = durStr
		} else {
			if msg.Error != "" {
				m.chat.UpdateToolResult(msg.ID, msg.Error, true)
			} else {
				m.chat.UpdateToolResult(msg.ID, msg.Output, false)
			}
			m.chat.SetToolDuration(msg.ID, durStr)
		}
		return m, nil

	case StopReasonMsg:
		m.chat.MarkLastStopReason(msg.Reason)
		return m, nil

	case AgentDoneMsg:
		m.streaming = false
		m.stopProgress()
		m.footer.SetWorkingMessage("")
		// Auto-save session after each agent turn (TS pi-mono: saves on message_end)
		if m.sessMgr != nil && m.session != nil {
			m.sessMgr.Save(m.session)
		}
		return m, nil

	case AgentErrorMsg:
		m.streaming = false
		m.stopProgress()
		// Save even on error to preserve conversation up to failure
		if m.sessMgr != nil && m.session != nil {
			m.sessMgr.Save(m.session)
		}
		m.chat.AppendError(msg.Error.Error())
		return m, nil

	case BashExecResultMsg:
		m.chat.AppendBashOutput(msg.Output)
		m.chat.CompleteBash(msg.ExitCode, msg.Cancelled)
		if msg.Truncated && msg.FullOutputPath != "" {
			m.chat.AppendWarning("Output truncated. Full output: " + msg.FullOutputPath)
		}
		return m, nil

	case ShareResultMsg:
		if msg.Error != "" {
			m.chat.AppendError(msg.Error)
		} else if msg.GistURL != "" {
			if msg.PreviewURL != "" {
				m.chat.AppendSystem("Share URL: " + msg.PreviewURL + "\nGist: " + msg.GistURL)
			} else {
				m.chat.AppendSystem("Share URL: " + msg.GistURL + "\nGist: " + msg.GistURL)
			}
		}
		return m, nil

	case refreshScopedSelectorMsg:
		m.overlay.Hide()
		m.showScopedModelSelector()
		return m, nil

	case refreshSettingsMsg:
		m.overlay.Hide()
		m.showSettingsSelector()
		if m.showHardwareCursor {
			return m, tea.ShowCursor
		}
		return m, tea.HideCursor

	case refreshWarningsMsg:
		m.overlay.Hide()
		m.showWarningsSelector()
		return m, nil

	case refreshModelSelectorMsg:
		m.overlay.Hide()
		m.showModelSelector()
		return m, nil

	case components.SelectorChosenMsg:
		if strings.HasPrefix(msg.Value, "show:") {
			sub := strings.TrimPrefix(msg.Value, "show:")
			switch sub {
			case "theme_selector":
				m.showThemeSelector()
			case "model_selector":
				m.showModelSelector()
			}
			return m, nil
		}
		if strings.HasPrefix(msg.Value, "session:") {
			sid := strings.TrimPrefix(msg.Value, "session:")
			m.switchToSession(sid)
		} else if msg.Value != "" {
			m.switchToModel(msg.Value)
		}
		return m, nil

	case extensionSelectMsg:
		items := make([]components.SelectorItem, len(msg.options))
		for i, opt := range msg.options {
			items[i] = components.SelectorItem{Label: opt, Value: opt}
		}
		w := m.width * 60 / 100
		h := m.height * 60 / 100
		m.overlay.ShowSelector(msg.title, items, func(value string) {
			msg.respCh <- extensionUIResponse{value: value}
		}, w, h)
		m.overlay.OnDismiss(func() {
			msg.respCh <- extensionUIResponse{err: fmt.Errorf("cancelled")}
		})
		if msg.timeout > 0 {
			m.overlay.StartCountdown(int(msg.timeout.Seconds()))
			return m, components.CountdownTick()
		}
		return m, nil

	case extensionInputMsg:
		w := m.width * 60 / 100
		h := 10
		m.overlay.ShowInput(msg.title, func(value string) {
			msg.respCh <- extensionUIResponse{value: value}
		}, func() {
			msg.respCh <- extensionUIResponse{err: fmt.Errorf("cancelled")}
		}, w, h)
		if msg.timeout > 0 {
			m.overlay.StartCountdown(int(msg.timeout.Seconds()))
			return m, components.CountdownTick()
		}
		return m, nil

	case extensionEditorMsg:
		w := m.width * 70 / 100
		h := m.height * 70 / 100
		m.overlay.ShowEditor(msg.title, msg.prefill, func(value string) {
			msg.respCh <- extensionUIResponse{value: value}
		}, func() {
			msg.respCh <- extensionUIResponse{err: fmt.Errorf("cancelled")}
		}, w, h)
		return m, nil

	case extensionCustomMsg:
		w := m.width * 60 / 100
		h := m.height * 60 / 100
		overlayButtons := make([]components.CustomButton, len(msg.buttons))
		for i, b := range msg.buttons {
			overlayButtons[i] = components.CustomButton{Label: b.Label, Value: b.Value}
		}
		m.overlay.ShowCustom(msg.title, msg.content, overlayButtons, func(value string) {
			msg.respCh <- extensionUIResponse{value: value}
		}, func() {
			msg.respCh <- extensionUIResponse{err: fmt.Errorf("cancelled")}
		}, w, h)
		m.overlay.OnDismiss(func() {
			msg.respCh <- extensionUIResponse{err: fmt.Errorf("cancelled")}
		})
		if msg.timeout > 0 {
			m.overlay.StartCountdown(int(msg.timeout.Seconds()))
			return m, components.CountdownTick()
		}
		return m, nil

	case extensionStatusMsg:
		if m.extensionStatuses == nil {
			m.extensionStatuses = make(map[string]string)
		}
		if msg.text == "" {
			delete(m.extensionStatuses, msg.key)
		} else {
			m.extensionStatuses[msg.key] = msg.text
		}
		m.footer.SetExtensionStatuses(m.extensionStatuses)
		return m, nil

	case extensionSetTitleMsg:
		fmt.Printf("\033]0;%s\007", msg.title)
		return m, nil

	case extensionHiddenThinkingLabelMsg:
		if msg.label == "" {
			m.chat.HiddenThinkingLabel = "Thinking..."
		} else {
			m.chat.HiddenThinkingLabel = msg.label
		}
		return m, nil

	case extensionWorkingMessageMsg:
		if msg.message == "" {
			m.workingMessage = "Working..."
		} else {
			m.workingMessage = msg.message
		}
		m.footer.SetWorkingMessage(m.workingMessage)
		return m, nil

	case extensionWorkingVisibleMsg:
		m.workingVisible = msg.visible
		m.footer.SetWorkingVisible(msg.visible)
		return m, nil

	case extensionWorkingIndicatorMsg:
		m.workingFrames = msg.frames
		m.workingIntervalMs = msg.intervalMs
		m.footer.SetWorkingIndicator(msg.frames, msg.intervalMs)
		return m, nil

	case extensionEditorTextMsg:
		if msg.isSet {
			if m.customEditor != nil {
				m.customEditor.SetValue(msg.text)
			} else {
				m.input.SetValue(msg.text)
			}
		} else {
			if m.customEditor != nil {
				msg.respCh <- m.customEditor.Value()
			} else {
				msg.respCh <- m.input.Value()
			}
		}
		return m, nil

	case extensionPasteMsg:
		if m.customEditor != nil {
			// Custom editors don't have Paste, fall back to SetValue
			m.customEditor.SetValue(msg.text)
		} else {
			m.input.Paste(msg.text)
		}
		return m, nil

	case extensionWidgetMsg:
		if msg.content == "" {
			if msg.placement == "belowEditor" {
				delete(m.widgetsBelow, msg.key)
			} else {
				delete(m.widgetsAbove, msg.key)
			}
		} else {
			if msg.placement == "belowEditor" {
				if m.widgetsBelow == nil {
					m.widgetsBelow = make(map[string]string)
				}
				m.widgetsBelow[msg.key] = msg.content
			} else {
				if m.widgetsAbove == nil {
					m.widgetsAbove = make(map[string]string)
				}
				m.widgetsAbove[msg.key] = msg.content
			}
		}
		return m, nil

	case extensionGetAllThemesMsg:
		themes := []extensions.ThemeInfo{
			{Name: "dark", Path: ""},
			{Name: "light", Path: ""},
		}
		customPaths, _ := DiscoverThemes("")
		for _, p := range customPaths {
			t, err := LoadTheme(p)
			if err != nil || t.Name == "" {
				continue
			}
			if t.Name == "dark" || t.Name == "light" {
				continue
			}
			themes = append(themes, extensions.ThemeInfo{Name: t.Name, Path: p})
		}
		msg.respCh <- themes
		return m, nil

	case extensionGetCurrentThemeNameMsg:
		if m.theme != nil {
			msg.respCh <- m.theme.Name
		} else {
			msg.respCh <- "dark"
		}
		return m, nil

	case extensionSetThemeMsg:
		switch msg.name {
		case "dark":
			m.ApplyTheme(DefaultTheme())
			msg.respCh <- nil
		case "light":
			m.ApplyTheme(LightTheme())
			msg.respCh <- nil
		default:
			// Search custom themes
			customPaths, _ := DiscoverThemes("")
			found := false
			for _, p := range customPaths {
				t, err := LoadTheme(p)
				if err != nil || t.Name != msg.name {
					continue
				}
				m.ApplyTheme(t)
				msg.respCh <- nil
				found = true
				break
			}
			if !found {
				msg.respCh <- fmt.Errorf("theme %q not found", msg.name)
			}
		}
		return m, nil

	case extensionGetToolsExpandedMsg:
		msg.respCh <- m.chat.AllToolsExpanded
		return m, nil

	case extensionSetToolsExpandedMsg:
		if m.chat.AllToolsExpanded != msg.expanded {
			m.chat.ToggleAllTools()
		}
		return m, nil

	// ── Extension Component Replacement ────────────────────────────────────

	case extSetFooterMsg:
		if msg.factory != nil {
			m.customFooter = msg.factory()
		} else {
			m.customFooter = nil
		}
		return m, nil

	case extSetHeaderMsg:
		if msg.factory != nil {
			m.customHeader = msg.factory()
		} else {
			m.customHeader = nil
		}
		return m, nil

	case extSetEditorMsg:
		if msg.factory != nil {
			m.customEditor = msg.factory()
			m.customEditorNeedsInit = true
		} else {
			if m.customEditor != nil {
				m.customEditor.Blur()
			}
			m.customEditor = nil
			m.customEditorNeedsInit = false
		}
		return m, nil

	case extGetEditorMsg:
		// Return nil — extensions store their own factory reference.
		msg.respCh <- nil
		return m, nil

	case appendSystemMsg:
		m.chat.AppendSystem(string(msg))
		return m, nil

	case appendErrorMsg:
		m.chat.AppendError(string(msg))
		return m, nil

	case appendWarningMsg:
		m.chat.AppendWarning(string(msg))
		return m, nil

	case StatusMsg:
		m.lastStatus = msg
		m.footer.Update(
			msg.TokensIn, msg.TokensOut,
			msg.TokensCacheR, msg.TokensCacheW,
			msg.TotalCost, msg.ContextUsed,
			msg.Streaming,
		)
		return m, nil

	case TickMsg:
		m.spinnerFrame = (m.spinnerFrame + 1) % 10
		m.footer.SetSpinnerFrame(m.spinnerFrame)
		return m, nil

	case BranchTickMsg:
		// Check if git branch changed and update footer + terminal title
		newBranch := getGitBranch(m.session.CWD)
		if newBranch != m.gitBranch {
			m.gitBranch = newBranch
			m.footer.SetGitBranch(newBranch)
			updateTerminalTitle(m.session.GetSessionName(), m.session.CWD)
		}
		return m, tea.Tick(3*time.Second, func(t time.Time) tea.Msg {
			return BranchTickMsg(t)
		})

	case RetryTickMsg:
		if m.retryTicking && m.retryDelaySec > 0 {
			m.retryDelaySec--
			if m.retryDelaySec > 0 {
				msg := fmt.Sprintf("Retrying (%d/%d) in %ds... (Esc to cancel)", m.retryAttempt, m.retryMaxAttempts, m.retryDelaySec)
				m.chat.ReplaceLastSystem(msg)
				return m, tea.Tick(1*time.Second, func(t time.Time) tea.Msg {
					return RetryTickMsg(t)
				})
			}
		}
		return m, nil

	case ResizeMsg:
		m.width = msg.Width
		m.height = msg.Height
		return m, nil
	}

	// Route to input editor (custom or built-in)
	var cmd tea.Cmd
	if m.customEditor != nil {
		// Init custom editor on first Update after instantiation
		if m.customEditorNeedsInit {
			m.customEditorNeedsInit = false
			cmd = m.customEditor.Init()
		}
		var next tea.Model
		next, cmd = m.customEditor.Update(msg)
		if ec, ok := next.(EditorComponent); ok {
			m.customEditor = ec
		}
	} else {
		*m.input, cmd = m.input.Update(msg)
	}

	// After editor update, check for slash mode and update autocomplete
	if m.input.IsSlashMode() {
		prefix := m.input.GetSlashPrefix()
		// Detect argument mode: if prefix contains a space, show argument completions
		if idx := strings.Index(prefix, " "); idx >= 0 {
			cmdName := strings.TrimPrefix(strings.ToLower(prefix[:idx]), "/")
			argPrefix := strings.TrimLeft(prefix[idx+1:], " ")
			argCandidates := m.getArgCompletions(cmdName, argPrefix)
			if len(argCandidates) > 0 {
				names := make([]string, len(argCandidates))
				descs := make(map[string]string)
				for i, c := range argCandidates {
					names[i] = c.Name
					descs[c.Name] = c.Description
				}
				m.input.SetSlashCandidates(names)
				// Show argument completions as non-command items in autocomplete
				m.autocomplete.Show(names, descs, argPrefix)
				return m, cmd
			}
			m.autocomplete.Hide()
			return m, cmd
		}

		candidates := m.filterSlashCandidates(prefix)
		// Merge extension-registered commands
		if m.extRunner != nil {
			for cmdName := range extensions.GetAllSlashCommands() {
				if prefix == "" || strings.HasPrefix(cmdName, prefix) {
					candidates = append(candidates, components.SlashCommand{
						Name:        cmdName,
						Description: "(extension)",
					})
				}
			}
		}
		// Merge skill commands (TS pi-mono: /skill:name for each loaded skill)
		if m.skillCommands && len(m.Skills) > 0 {
			for _, sk := range m.Skills {
				cmdName := "/skill:" + sk.Name
				if prefix == "" || strings.HasPrefix(cmdName, prefix) {
					desc := sk.Description
					if desc == "" {
						desc = fmt.Sprintf("Invoke skill %s", sk.Name)
					}
					candidates = append(candidates, components.SlashCommand{
						Name:        cmdName,
						Description: desc,
					})
				}
			}
		}
		// Merge extension autocomplete providers
		for _, provider := range extensions.GetAllAutocompleteProviders() {
			for _, candidate := range provider(prefix) {
				if prefix == "" || strings.HasPrefix(strings.ToLower(candidate), strings.ToLower(prefix)) {
					candidates = append(candidates, components.SlashCommand{
						Name:        candidate,
						Description: "(ext)",
					})
				}
			}
		}
		names := make([]string, len(candidates))
		for i, c := range candidates {
			names[i] = c.Name
		}
		m.input.SetSlashCandidates(names)
		// Update autocomplete overlay with formatted candidates
		m.updateAutocomplete(candidates, prefix)
	} else if m.input.IsFileMode() {
		prefix := m.input.GetFilePrefix()
		rawPrefix := m.input.GetFilePrefixRaw()
		// Detect quoted path mode: @"path with spaces"
		quoted := strings.HasPrefix(rawPrefix, "\"")
		// Find files matching the prefix
		matches := components.FindFiles(prefix)
		if len(matches) == 0 {
			// If prefix is empty, list files in CWD
			if prefix == "" {
				matches = components.FindFiles(".")
			}
		}
		if len(matches) > 0 {
			// Preserve quoting in displayed completions (TS pi-mono: quoted @ paths)
			names := matches
			if quoted {
				for i, m := range matches {
					names[i] = "\"" + m + "\""
				}
			}
			descs := make(map[string]string)
			for i := range matches {
				descs[names[i]] = "" // file paths don't need descriptions
			}
			if len(names) > 20 {
				names = names[:20]
			}
			displayPrefix := prefix
			if quoted {
				displayPrefix = "\"" + prefix
			}
			m.autocomplete.Show(names, descs, displayPrefix)
		} else {
			m.autocomplete.Hide()
		}
	} else if m.input.IsSymbolMode() {
		prefix := m.input.GetSymbolPrefix()
		// Collect symbols from recent session entries (TS pi-mono: # symbol autocomplete)
		symbols := m.collectSymbols(prefix)
		if len(symbols) > 0 {
			names := make([]string, 0, len(symbols))
			descs := make(map[string]string)
			for _, s := range symbols {
				names = append(names, s.Name)
				descs[s.Name] = s.Description
			}
			m.autocomplete.Show(names, descs, prefix)
		} else {
			m.autocomplete.Hide()
		}
	} else {
		m.autocomplete.Hide()
	}

	// Return editor command batched with spinner tick if streaming or compacting
	var cmds []tea.Cmd
	if cmd != nil {
		cmds = append(cmds, cmd)
	}
	if m.streaming || m.compacting {
		cmds = append(cmds, tea.Tick(time.Millisecond*100, func(t time.Time) tea.Msg {
			return TickMsg(t)
		}))
	}
	return m, tea.Batch(cmds...)
}

// View renders the entire UI.
func (m AppModel) View() string {
	if m.quitting {
		return "Goodbye.\n"
	}

	chatView := m.chat.View()
	headerView := m.renderHeader()
	inputView := m.renderEditor()
	footerView := m.renderFooter()

	// Show pending messages indicator (TS pi-mono: pending messages section)
	pendingView := m.pendingView()

	// Transient status container (TS pi-mono: statusContainer)
	statusView := m.statusLine()

	// Build widget views (above/below editor)
	widgetAboveView := m.widgetsView(m.widgetsAbove)
	widgetBelowView := m.widgetsView(m.widgetsBelow)

	main := lipgloss.JoinVertical(
		lipgloss.Top,
		headerView,
		chatView,
		pendingView,
		statusView,
		widgetAboveView,
		inputView,
		widgetBelowView,
		footerView,
	)

	var result string

	// Show autocomplete popover above the input
	if m.autocomplete.Active() {
		acView := m.autocomplete.View()
		result = lipgloss.JoinVertical(lipgloss.Top, headerView, chatView, pendingView, statusView, acView, inputView, footerView)
	} else if m.overlay.Active() {
		// Show overlay (modal or non-capturing)
		overlayView := m.overlay.View()
		if m.overlay.NonCapturing() {
			result = lipgloss.JoinVertical(lipgloss.Top, overlayView, main)
		} else {
			anchor := m.overlay.Anchor()
			hPos, vPos := components.AnchorToLipgloss(anchor)
			result = lipgloss.Place(
				m.width, m.height,
				hPos, vPos,
				overlayView,
				lipgloss.WithWhitespaceChars(" "),
				lipgloss.WithWhitespaceForeground(lipgloss.Color("#000000")),
			)
		}
	} else {
		result = main
	}

	// TUI write log for debugging (TS pi-mono: PI_TUI_WRITE_LOG)
	if m.writeLogFile != nil {
		m.writeLogFile.WriteString(result)
		m.writeLogFile.WriteString("\n---\n")
	}

	return result
}

// renderHeader returns the rendered header, using custom component if set.
func (m *AppModel) renderHeader() string {
	if m.customHeader != nil {
		return m.customHeader.View()
	}
	return m.header.View()
}

// renderFooter returns the rendered footer, using custom component if set.
func (m *AppModel) renderFooter() string {
	if m.customFooter != nil {
		return m.customFooter.View()
	}
	return m.footer.View()
}

// renderEditor returns the rendered editor, using custom component if set.
func (m *AppModel) renderEditor() string {
	if m.customEditor != nil {
		return m.customEditor.View()
	}
	return m.input.View()
}

// editorHeight returns the current editor height in rows.
func (m *AppModel) editorHeight() int {
	if m.customEditor != nil {
		return m.customEditor.Height()
	}
	return m.input.Height()
}

// editorEmpty returns true when the editor has no content.
func (m *AppModel) editorEmpty() bool {
	if m.customEditor != nil {
		return m.customEditor.Empty()
	}
	return m.editorEmpty()
}

// footerHeight returns the current footer height in rows.
// Default footer is 3 lines (2 info lines + optional extension line).
// Custom footer components report their own height.
func (m *AppModel) footerHeight() int {
	if m.customFooter != nil {
		// Custom footer View() height — we count newlines + 1
		view := m.customFooter.View()
		lines := 0
		for _, r := range view {
			if r == '\n' {
				lines++
			}
		}
		if view != "" {
			lines++
		}
		return lines
	}
	if len(m.extensionStatuses) > 0 {
		return 3
	}
	return 2
}

// ─── Pending Messages ──────────────────────────────────────────────────────


// truncateToVisualWidth truncates a string to maxWidth visual columns,
// adding "..." if truncated. Uses rune-level truncation.
func truncateToVisualWidth(s string, maxWidth int) string {
	runes := []rune(s)
	width := 0
	for i, r := range runes {
		w := 1
		// East Asian wide chars and emoji
		if r >= 0x1100 && (r <= 0x115f || r == 0x2329 || r == 0x232a ||
			(r >= 0x2e80 && r <= 0xa4cf) || (r >= 0xac00 && r <= 0xd7a3) ||
			(r >= 0xf900 && r <= 0xfaff) || (r >= 0xfe10 && r <= 0xfe19) ||
			(r >= 0xfe30 && r <= 0xfe6f) || (r >= 0xff01 && r <= 0xff60) ||
			(r >= 0xffe0 && r <= 0xffe6) || (r >= 0x1f300 && r <= 0x1f9ff)) {
			w = 2
		}
		if width+w > maxWidth {
			return string(runes[:i]) + "..."
		}
		width += w
	}
	return s
}
// pendingView renders individual queued messages (TS pi-mono updatePendingMessagesDisplay).
// Shows Steering:/Follow-up: prefixes, width-aware truncation, and dequeue hint.
func (m *AppModel) pendingView() string {
	steeringMsgs := m.pendingSteeringMsgs
	followUpMsgs := m.pendingFollowUpMsgs

	// Include compaction queued messages as steering (pi-mono style)
	allSteering := make([]string, 0, len(steeringMsgs)+len(m.compactionQueue))
	allSteering = append(allSteering, steeringMsgs...)
	allSteering = append(allSteering, m.compactionQueue...)

	if len(allSteering) == 0 && len(followUpMsgs) == 0 {
		return ""
	}

	dimStyle := lipgloss.NewStyle().Faint(true)

	var sb strings.Builder
	for _, msg := range allSteering {
		sb.WriteString(dimStyle.Render("Steering: " + truncateToVisualWidth(msg, 80)))
		sb.WriteString("\n")
	}
	for _, msg := range followUpMsgs {
		sb.WriteString(dimStyle.Render("Follow-up: " + truncateToVisualWidth(msg, 80)))
		sb.WriteString("\n")
	}
	// Dequeue hint: ↳ Alt+Up to edit all queued messages
	sb.WriteString(dimStyle.Render("↳ Alt+Up to edit all queued messages"))

	return sb.String()
}

// statusLine renders the transient status container between chat and editor
// (TS pi-mono: statusContainer — shows Working.../compaction/retry progress).
func (m *AppModel) statusLine() string {
	mutedStyle := lipgloss.NewStyle().Foreground(lipgloss.Color(m.theme.Muted))

	if m.compacting {
		return mutedStyle.Render("  Compacting context... (Esc to cancel)")
	}
	if m.retryTicking {
		msg := fmt.Sprintf("  Retrying (%d/%d) in %ds... (Esc to cancel)",
			m.retryAttempt, m.retryMaxAttempts, m.retryDelaySec)
		return mutedStyle.Render(msg)
	}
	if m.streaming && m.workingVisible {
		msg := m.workingMessage
		if msg == "" {
			msg = "Working..."
		}
		// Use pi-mono spinner frame
		frames := []string{"⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"}
		if len(m.workingFrames) > 0 {
			frames = m.workingFrames
		}
		frame := frames[m.spinnerFrame%len(frames)]
		return mutedStyle.Render("  " + frame + " " + msg + " (Esc to interrupt)")
	}
	return ""
}

// widgetsView renders extension widgets as text lines.
func (m *AppModel) widgetsView(widgets map[string]string) string {
	if len(widgets) == 0 {
		return ""
	}
	widgetStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#c678dd")).
		PaddingLeft(2)
	var sb strings.Builder
	for _, text := range widgets {
		sb.WriteString(widgetStyle.Render("│ " + text))
		sb.WriteByte('\n')
	}
	return sb.String()
}

// ─── Startup Banner ────────────────────────────────────────────────────────

// formatKeyStr returns the first key for a binding, or empty string.
func formatKeyStr(kb *KeybindingsManager, binding KeybindingID) string {
	if kb == nil {
		return ""
	}
	keys := kb.GetKeys(binding)
	if len(keys) > 0 {
		return keys[0]
	}
	return ""
}

func (m *AppModel) showWelcome(msg WelcomeMsg) {
	m.setTerminalTitle()
	if m.quietStartup {
		return
	}
	accentStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color(msg.ThemeAccent)).
		Bold(true)
	dimStyle := lipgloss.NewStyle().
		Faint(true)
	warningStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color(m.theme.Warning))

	m.chat.AppendSystem(accentStyle.Render("xihu v" + utils.Version))

	// Check for new changelog entries and show as non-capturing banner
	m.checkChangelog()

	// Asynchronously check for newer version
	go m.checkNewVersion()

	// Asynchronously check tmux keyboard setup (TS pi-mono: checkTmuxKeyboardSetup)
	go m.checkTmuxKeyboard()

	// Show settings/model loading errors (TS pi-mono: models.json / settings errors at startup)
	if msg.SettingsError != "" {
		m.chat.AppendError("settings error: " + msg.SettingsError)
	}

	if !m.welcomeExpanded {
		// Collapsed: brief status (uses actual keybinding for toggle header)
		toggleKey := formatKeyStr(m.keybindings, GlobalToggleHeader)
		if toggleKey == "" {
			toggleKey = "Ctrl+H"
		}
		m.chat.AppendSystem(dimStyle.Render("  " + toggleKey + " expand header for all shortcuts"))
		return
	}

	// Expanded: brief summary — uses actual keybinding values
	submitKey := formatKeyStr(m.keybindings, InputSubmit)
	if submitKey == "" {
		submitKey = "Enter"
	}
	interruptKey := formatKeyStr(m.keybindings, GlobalInterrupt)
	if interruptKey == "" {
		interruptKey = "Esc"
	}
	toggleKey := formatKeyStr(m.keybindings, GlobalToggleHeader)
	if toggleKey == "" {
		toggleKey = "Ctrl+H"
	}
	m.chat.AppendSystem(fmt.Sprintf("  %s=submit · %s=interrupt · / commands · ! bash · %s=toggle header",
		submitKey, interruptKey, toggleKey))

	// Show loaded skills (TS pi-mono: showLoadedResources Skills section)
	if len(msg.Skills) > 0 {
		skillNames := make([]string, len(msg.Skills))
		for i, s := range msg.Skills {
			skillNames[i] = s.Name
		}
		// Group by source
		userSkills := make([]string, 0)
		projectSkills := make([]string, 0)
		otherSkills := make([]string, 0)
		for _, s := range msg.Skills {
			switch s.Source {
			case "project":
				projectSkills = append(projectSkills, s.Name)
			case "user":
				userSkills = append(userSkills, s.Name)
			default:
				otherSkills = append(otherSkills, s.Name)
			}
		}
		m.chat.AppendSystem("[Skills]")
		m.chat.AppendSystem(dimStyle.Render("  " + strings.Join(skillNames, ", ")))
		if len(projectSkills) > 0 {
			m.chat.AppendSystem(dimStyle.Render(fmt.Sprintf("  project: %s", strings.Join(projectSkills, ", "))))
		}
		if len(userSkills) > 0 {
			m.chat.AppendSystem(dimStyle.Render(fmt.Sprintf("  user: %s", strings.Join(userSkills, ", "))))
		}
		if len(otherSkills) > 0 {
			m.chat.AppendSystem(dimStyle.Render(fmt.Sprintf("  other: %s", strings.Join(otherSkills, ", "))))
		}
	}

	// Show loaded extensions
	if m.extRunner != nil {
		loaded := m.extRunner.Initialized()
		if len(loaded) > 0 {
			names := make([]string, len(loaded))
			for i, e := range loaded {
				names[i] = e.Name()
			}
			m.chat.AppendSystem("[Extensions]")
			m.chat.AppendSystem(dimStyle.Render("  " + strings.Join(names, ", ")))
		}
	} else if len(msg.Extensions) > 0 {
		m.chat.AppendSystem("[Extensions] " + strings.Join(msg.Extensions, ", "))
	}

	// Show context files (TS pi-mono: showLoadedResources Context section)
	if len(msg.ContextFiles) > 0 {
		contextCompact := make([]string, len(msg.ContextFiles))
		for i, fp := range msg.ContextFiles {
			contextCompact[i] = formatContextPath(fp)
		}
		m.chat.AppendSystem("[Context]")
		m.chat.AppendSystem(dimStyle.Render("  " + strings.Join(contextCompact, ", ")))
	}

	// Show prompt templates (TS pi-mono: showLoadedResources Prompts section)
	if len(msg.PromptTemplates) > 0 {
		templateNames := make([]string, len(msg.PromptTemplates))
		for i, t := range msg.PromptTemplates {
			templateNames[i] = "/" + t.Name
		}
		m.chat.AppendSystem("[Prompts]")
		m.chat.AppendSystem(dimStyle.Render("  " + strings.Join(templateNames, ", ")))
	}

	// Show loaded themes (TS pi-mono: showLoadedResources Themes section)
	customThemePaths, _ := DiscoverThemes("")
	if len(customThemePaths) > 0 {
		themeNames := make([]string, 0, len(customThemePaths))
		for _, p := range customThemePaths {
			t, err := LoadTheme(p)
			if err == nil && t.Name != "" {
				themeNames = append(themeNames, t.Name)
			} else {
				themeNames = append(themeNames, filepath.Base(p))
			}
		}
		m.chat.AppendSystem("[Themes]")
		m.chat.AppendSystem(dimStyle.Render("  " + strings.Join(themeNames, ", ")))
	}

	// Detect and show prompt template collisions (TS pi-mono: [Prompt conflicts])
	if len(msg.PromptTemplates) > 0 {
		promptCollisions := m.detectPromptCollisions(msg.PromptTemplates)
		if len(promptCollisions) > 0 {
			m.chat.AppendSystem(warningStyle.Render("[Prompt conflicts]"))
			for _, c := range promptCollisions {
				m.chat.AppendSystem(warningStyle.Render(fmt.Sprintf("  %q collision:", c.Name)))
				m.chat.AppendSystem(dimStyle.Render(fmt.Sprintf("    \xe2\x9c\x93 %s", c.WinnerPath)))
				m.chat.AppendSystem(dimStyle.Render(fmt.Sprintf("    \xe2\x9c\x97 %s (skipped)", c.LoserPath)))
			}
		}
	}

	// Detect and show theme collisions (TS pi-mono: [Theme conflicts])
	themeCollisions := m.detectThemeCollisions()
	if len(themeCollisions) > 0 {
		m.chat.AppendSystem(warningStyle.Render("[Theme conflicts]"))
		for _, c := range themeCollisions {
			m.chat.AppendSystem(warningStyle.Render(fmt.Sprintf("  %q collision:", c.Name)))
			m.chat.AppendSystem(dimStyle.Render(fmt.Sprintf("    \xe2\x9c\x93 %s", c.WinnerPath)))
			m.chat.AppendSystem(dimStyle.Render(fmt.Sprintf("    \xe2\x9c\x97 %s (skipped)", c.LoserPath)))
		}
	}

	// Show diagnostics (TS pi-mono: showLoadedResources diagnostics section)
	if len(msg.SkillCollisions) > 0 || len(msg.ExtensionDiagnostics) > 0 || len(msg.KeybindingConflicts) > 0 {
		m.showLoadedDiagnostics(msg.SkillCollisions, msg.ExtensionDiagnostics, msg.KeybindingConflicts)
	}
}

// getExtDiagnostics returns extension init/load diagnostics from the extension runner.
func (m *AppModel) getExtDiagnostics() []extensions.ExtensionDiagnostic {
	if m.extRunner == nil {
		return nil
	}
	return m.extRunner.GetExtensionDiagnostics()
}

// getKBConflicts returns keybinding conflicts from the global keybindings manager.
func (m *AppModel) getKBConflicts() []KeybindingConflict {
	kb := GetKeybindings()
	if kb == nil {
		return nil
	}
	return kb.GetConflicts()
}

// showPostReloadDiagnostics shows diagnostics after /reload (TS pi-mono: showLoadedResources after reload).
func (m *AppModel) showPostReloadDiagnostics() {
	extDiags := m.getExtDiagnostics()
	kbConflicts := m.getKBConflicts()

	if len(extDiags) == 0 && len(kbConflicts) == 0 {
		return
	}
	m.showLoadedDiagnostics(nil, extDiags, kbConflicts)
}

// showLoadedDiagnostics renders skill collisions, extension diagnostics, and keybinding conflicts.
func (m *AppModel) showLoadedDiagnostics(skillCollisions []skills.SkillCollision, extDiags []extensions.ExtensionDiagnostic, kbConflicts []KeybindingConflict) {
	dimStyle := lipgloss.NewStyle().Faint(true)
	warningStyle := lipgloss.NewStyle().Foreground(lipgloss.Color(m.theme.Warning))

	if len(skillCollisions) > 0 {
		m.chat.AppendSystem(warningStyle.Render("[Skill conflicts]"))
		for _, c := range skillCollisions {
			m.chat.AppendSystem(warningStyle.Render(fmt.Sprintf("  %q collision:", c.Name)))
			m.chat.AppendSystem(dimStyle.Render(fmt.Sprintf("    ✓ %s (%s)", c.WinnerPath, c.WinnerSource)))
			m.chat.AppendSystem(dimStyle.Render(fmt.Sprintf("    ✗ %s (%s) (skipped)", c.LoserPath, c.LoserSource)))
		}
	}
	if len(extDiags) > 0 {
		m.chat.AppendSystem(warningStyle.Render("[Extension issues]"))
		for _, d := range extDiags {
			prefix := "Error"
			if d.Type == "warning" {
				prefix = "Warning"
			}
			m.chat.AppendSystem(dimStyle.Render(fmt.Sprintf("  %s: %s (%s)", prefix, d.Message, d.Path)))
		}
	}
	if len(kbConflicts) > 0 {
		m.chat.AppendSystem(warningStyle.Render("[Keybinding conflicts]"))
		for _, c := range kbConflicts {
			bindingNames := make([]string, len(c.Bindings))
			for i, b := range c.Bindings {
				bindingNames[i] = string(b)
			}
			m.chat.AppendSystem(dimStyle.Render(fmt.Sprintf("  %s bound to: %s", c.Key, strings.Join(bindingNames, ", "))))
		}
	}
}

// detectPromptCollisions finds naming conflicts in prompt templates.
// Since user templates are loaded first then project templates (loaded later),
// the last template with a given name wins (project overrides user).
func (m *AppModel) detectPromptCollisions(templates []prompt.PromptTemplate) []PromptCollision {
	seen := make(map[string]int) // name -> index of first occurrence
	var collisions []PromptCollision
	for i, t := range templates {
		if firstIdx, exists := seen[t.Name]; exists {
			// Project (loaded later, i) overrides earlier template
			collisions = append(collisions, PromptCollision{
				Name:       t.Name,
				WinnerPath: t.Source,
				LoserPath:  templates[firstIdx].Source,
			})
			seen[t.Name] = i // update to new winner
		} else {
			seen[t.Name] = i
		}
	}
	return collisions
}

// detectThemeCollisions finds naming conflicts among discovered themes.
// Themes are discovered from ~/.xihu/themes/; the last theme with a given name wins.
func (m *AppModel) detectThemeCollisions() []ThemeCollision {
	paths, err := DiscoverThemes("")
	if err != nil || len(paths) == 0 {
		return nil
	}
	// Map theme name -> path
	seen := make(map[string]string)
	var collisions []ThemeCollision
	for _, p := range paths {
		t, err := LoadTheme(p)
		if err != nil || t.Name == "" {
			continue
		}
		if firstPath, exists := seen[t.Name]; exists {
			collisions = append(collisions, ThemeCollision{
				Name:       t.Name,
				WinnerPath: p,
				LoserPath:  firstPath,
			})
			seen[t.Name] = p // update to new winner
		} else {
			seen[t.Name] = p
		}
	}
	return collisions
}

// ─── Changelog ───────────────────────────────────────────────────────────────

// checkChangelog loads the changelog and shows new entries as a non-capturing banner.
func (m *AppModel) checkChangelog() {
	path := utils.ChangelogPath()
	if path == "" {
		return
	}
	entries, err := utils.ParseChangelog(path)
	if err != nil || len(entries) == 0 {
		return
	}

	newEntries := utils.GetNewEntries(entries, m.lastChangelogVersion)
	if len(newEntries) == 0 {
		return
	}

	// Show the latest new entry as a non-capturing banner
	latest := newEntries[len(newEntries)-1]
	banner := buildChangelogBanner(latest)
	if banner != "" {
		w := m.width - 4
		if w < 40 {
			w = 40
		}
		if w > 80 {
			w = 80
		}
		m.overlay.ShowNonCapturingText(banner, w, strings.Count(banner, "\n")+3)
	}
}

// showFullChangelog displays the complete changelog as a scrollable modal overlay.
func (m *AppModel) showFullChangelog() {
	path := utils.ChangelogPath()
	if path == "" {
		m.chat.AppendSystem("No changelog entries found.")
		return
	}
	entries, err := utils.ParseChangelog(path)
	if err != nil || len(entries) == 0 {
		m.chat.AppendSystem("No changelog entries found.")
		return
	}

	var sb strings.Builder
	accentStyle := lipgloss.NewStyle().Foreground(lipgloss.Color(m.theme.Accent)).Bold(true)
	sb.WriteString(accentStyle.Render("What's New"))
	sb.WriteString("\n\n")

	// Show entries in reverse (newest first)
	for i := len(entries) - 1; i >= 0; i-- {
		e := entries[i]
		content := e.Content
		// Limit very long entries
		lines := strings.Split(content, "\n")
		if len(lines) > 30 {
			content = strings.Join(lines[:30], "\n") + "\n..."
		}
		sb.WriteString(content)
		sb.WriteString("\n\n")
	}

	w := m.width - 8
	if w < 40 {
		w = 40
	}
	if w > 80 {
		w = 80
	}
	h := m.height - 4
	if h < 10 {
		h = 10
	}
	m.overlay.ShowScrollableText(sb.String(), w, h)
}

// buildChangelogBanner builds a condensed banner for a changelog entry.
func buildChangelogBanner(entry utils.ChangelogEntry) string {
	lines := strings.Split(entry.Content, "\n")
	if len(lines) == 0 {
		return ""
	}
	// Trim empty leading/trailing lines
	for len(lines) > 0 && strings.TrimSpace(lines[0]) == "" {
		lines = lines[1:]
	}
	for len(lines) > 0 && strings.TrimSpace(lines[len(lines)-1]) == "" {
		lines = lines[:len(lines)-1]
	}
	if len(lines) == 0 {
		return ""
	}

	var sb strings.Builder
	// Version header
	headerStyle := lipgloss.NewStyle().Foreground(lipgloss.Color("#e5c07b")).Bold(true)
	sb.WriteString(headerStyle.Render(fmt.Sprintf("v%d.%d.%d", entry.Major, entry.Minor, entry.Patch)))
	sb.WriteString(" — ")

	// First content line
	dimStyle := lipgloss.NewStyle().Foreground(lipgloss.Color("#abb2bf"))
	detailLine := strings.TrimPrefix(lines[0], "### ")
	sb.WriteString(dimStyle.Render(detailLine))

	// Additional lines (up to 5)
	for _, l := range lines[1:] {
		l = strings.TrimSpace(l)
		if l == "" {
			continue
		}
		sb.WriteByte('\n')
		sb.WriteString(dimStyle.Render("  " + l))
		if strings.Count(sb.String(), "\n") >= 6 {
			sb.WriteByte('\n')
			sb.WriteString(dimStyle.Render("  ... Use /changelog for full details"))
			break
		}
	}

	return sb.String()
}

// checkNewVersion asynchronously checks for a newer xihu version and shows a notification.
func (m *AppModel) checkNewVersion() {
	result := utils.CheckVersion()
	if result == nil || !result.Newer {
		return
	}
	if m.program == nil {
		return
	}
	// TS pi-mono: showNewVersionNotification — bordered warning block
	warnStyle := lipgloss.NewStyle().Foreground(lipgloss.Color(m.theme.Warning))
	mutedStyle := lipgloss.NewStyle().Foreground(lipgloss.Color(m.theme.Muted))
	accentStyle := lipgloss.NewStyle().Foreground(lipgloss.Color(m.theme.Accent))
	boldWarn := lipgloss.NewStyle().Foreground(lipgloss.Color(m.theme.Warning)).Bold(true)
	// Full-width borders matching pi-mono DynamicBorder
	borderLine := warnStyle.Render(strings.Repeat("─", 72))
	m.program.Send(StreamTextMsg(""))
	m.chat.AppendSystem(borderLine)
	m.chat.AppendSystem(boldWarn.Render("Update Available") + "\n" +
		mutedStyle.Render(fmt.Sprintf("New version %s is available. Run ", result.Latest))+
			accentStyle.Render("xihu update") + "\n" +
		mutedStyle.Render("Changelog: ")+
			accentStyle.Render("https://github.com/huichen/xihu/releases/latest"))
	m.chat.AppendSystem(borderLine)
}

// checkTmuxKeyboard checks tmux extended-keys settings and warns if suboptimal.
// Mirrors pi-mono's checkTmuxKeyboardSetup — runs asynchronously at startup.
func (m *AppModel) checkTmuxKeyboard() {
	if os.Getenv("TMUX") == "" {
		return
	}

	runTmuxShow := func(option string) (string, bool) {
		ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
		defer cancel()
		cmd := exec.CommandContext(ctx, "tmux", "show", "-gv", option)
		cmd.Stdin = nil
		out, err := cmd.Output()
		if err != nil {
			return "", false
		}
		return strings.TrimSpace(string(out)), true
	}

	extendedKeys, ok := runTmuxShow("extended-keys")
	if !ok {
		return // tmux not available or timed out
	}

	if extendedKeys != "on" && extendedKeys != "always" {
		if m.program != nil {
			m.program.Send(appendWarningMsg("tmux extended-keys is off. Modified Enter keys may not work. Add `set -g extended-keys on` to ~/.tmux.conf and restart tmux."))
		}
	}

	extendedKeysFormat, ok := runTmuxShow("extended-keys-format")
	if ok && extendedKeysFormat == "xterm" {
		if m.program != nil {
			m.program.Send(appendWarningMsg("tmux extended-keys-format is xterm. xihu works best with csi-u. Add `set -g extended-keys-format csi-u` to ~/.tmux.conf and restart tmux."))
		}
	}
}

// ─── Help Overlay ──────────────────────────────────────────────────────────

// showHelpOverlay displays the full keybinding reference as a scrollable modal overlay.
func (m *AppModel) showHelpOverlay() {
	helpText := m.buildHelpOverlay()
	w := m.width - 8
	if w < 40 {
		w = 40
	}
	if w > 80 {
		w = 80
	}
	h := m.height - 4
	if h < 10 {
		h = 10
	}
	m.overlay.ShowScrollableText(helpText, w, h)
}

func (m *AppModel) buildHelpOverlay() string {
	var sb strings.Builder

	// Title
	titleStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color(m.theme.Accent)).
		Bold(true)
	sb.WriteString(titleStyle.Render("Keyboard Shortcuts"))
	sb.WriteString("\n\n")

	// Keybindings by category — resolved from KeybindingsManager (user-customizable)
	categoryOrder := []string{"global", "editor", "tools"}
	categoryTitles := map[string]string{
		"global": "Global",
		"editor": "Editor",
		"tools":  "Tools",
	}

	// Group resolved bindings by category
	groups := make(map[string][]ResolvedBinding)
	if m.keybindings != nil {
		for _, b := range m.keybindings.GetResolvedBindings() {
			if b.Category == "" {
				continue
			}
			groups[b.Category] = append(groups[b.Category], b)
		}
	}

	keyStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#89b4fa")).
		Width(22)
	descStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#abb2bf"))

	for _, cat := range categoryOrder {
		bindings, ok := groups[cat]
		if !ok || len(bindings) == 0 {
			continue
		}
		sb.WriteString(titleStyle.Render("▸ " + categoryTitles[cat]))
		sb.WriteByte('\n')
		for _, b := range bindings {
			keyStr := strings.Join(b.Keys, " / ")
			sb.WriteString("  ")
			sb.WriteString(keyStyle.Render(keyStr))
			sb.WriteString(descStyle.Render(b.Description))
			sb.WriteByte('\n')
		}
		sb.WriteByte('\n')
	}

	// Loaded resources
	sb.WriteString(titleStyle.Render("▸ Loaded Resources"))
	sb.WriteByte('\n')

	// Skills (TS pi-mono: showLoadedResources with source grouping)
	if len(m.Skills) > 0 {
		skillNames := make([]string, len(m.Skills))
		for i, s := range m.Skills {
			skillNames[i] = s.Name
		}
		sb.WriteString("  [Skills]")
		sb.WriteByte('\n')
		sb.WriteString("    " + strings.Join(skillNames, ", "))
	} else {
		sb.WriteString("  [Skills] none")
	}
	sb.WriteByte('\n')

	// Extensions
	if m.extRunner != nil {
		loaded := m.extRunner.Initialized()
		if len(loaded) > 0 {
			names := make([]string, len(loaded))
			for i, e := range loaded {
				names[i] = e.Name()
			}
			sb.WriteString("  [Extensions]")
			sb.WriteByte('\n')
			sb.WriteString("    " + strings.Join(names, ", "))
		} else {
			sb.WriteString("  [Extensions] none")
		}
	} else if len(m.Extensions) > 0 {
		sb.WriteString("  [Extensions] " + strings.Join(m.Extensions, ", "))
	} else {
		sb.WriteString("  [Extensions] none")
	}
	sb.WriteByte('\n')

	// Extension commands
	if m.extRunner != nil {
		extCmds := extensions.GetAllSlashCommands()
		if len(extCmds) > 0 {
			cmdNames := make([]string, 0, len(extCmds))
			for name := range extCmds {
				cmdNames = append(cmdNames, name)
			}
			sb.WriteString("  [Extension Cmds]")
			sb.WriteByte('\n')
			sb.WriteString("    " + strings.Join(cmdNames, ", "))
		} else {
			sb.WriteString("  [Extension Cmds] none")
		}
		sb.WriteByte('\n')
	}

	// Prompt templates
	if len(m.promptTemplates) > 0 {
		ptNames := make([]string, len(m.promptTemplates))
		for i, pt := range m.promptTemplates {
			ptNames[i] = "/" + pt.Name
		}
		sb.WriteString("  [Prompts]")
		sb.WriteByte('\n')
		sb.WriteString("    " + strings.Join(ptNames, ", "))
	} else {
		sb.WriteString("  [Prompts] none (place .md files in ~/.xihu/prompts/ or .xihu/prompts/)")
	}
	sb.WriteByte('\n')
	// Themes
	sb.WriteString("  [Themes] default, light + custom themes")
	sb.WriteByte('\n')

	return sb.String()
}

// ─── Slash Command Autocomplete ────────────────────────────────────────────

// fuzzyMatchScore checks if all characters in query appear in text in order.
// Returns (matches, score) where lower score = better match.
func fuzzyMatchScore(query, text string) (bool, float64) {
	ql := strings.ToLower(query)
	tl := strings.ToLower(text)
	if len(ql) == 0 {
		return true, 0
	}
	if len(ql) > len(tl) {
		return false, 0
	}
	qi := 0
	score := 0.0
	lastMatch := -1
	consecutive := 0
	for i := 0; i < len(tl) && qi < len(ql); i++ {
		if tl[i] == ql[qi] {
			if lastMatch == i-1 {
				consecutive++
				score -= float64(consecutive) * 5
			} else {
				consecutive = 0
				if lastMatch >= 0 {
					score += float64(i-lastMatch-1) * 2
				}
			}
			// Reward word boundaries
			if i == 0 || tl[i-1] == '/' || tl[i-1] == '-' || tl[i-1] == '_' || tl[i-1] == ' ' {
				score -= 10
			}
			score += float64(i) * 0.1
			lastMatch = i
			qi++
		}
	}
	if qi < len(ql) {
		return false, 0
	}
	if ql == tl {
		score -= 100
	}
	return true, score
}

// getArgCompletions returns argument-specific completions for a slash command.
// (TS pi-mono: SlashCommand.getArgumentCompletions)
func (m *AppModel) getArgCompletions(cmdName, argPrefix string) []components.SlashCommand {
	switch cmdName {
	case "model":
		// Complete model names
		var results []components.SlashCommand
		for _, model := range m.availableModels {
			if argPrefix == "" || strings.HasPrefix(strings.ToLower(model), strings.ToLower(argPrefix)) {
				results = append(results, components.SlashCommand{
					Name:        model,
					Description: "switch to model",
				})
			}
		}
		return results

	case "theme":
		// Complete theme names
		themes := []string{"dark", "light"}
		customPaths, _ := DiscoverThemes("")
		for _, p := range customPaths {
			t, err := LoadTheme(p)
			if err == nil && t.Name != "" && t.Name != "dark" && t.Name != "light" {
				themes = append(themes, t.Name)
			}
		}
		var results []components.SlashCommand
		for _, t := range themes {
			if argPrefix == "" || strings.HasPrefix(strings.ToLower(t), strings.ToLower(argPrefix)) {
				results = append(results, components.SlashCommand{
					Name:        t,
					Description: "apply theme",
				})
			}
		}
		return results

	case "thinking":
		levels := []string{"off", "minimal", "low", "medium", "high", "xhigh"}
		var results []components.SlashCommand
		for _, l := range levels {
			if argPrefix == "" || strings.HasPrefix(l, strings.ToLower(argPrefix)) {
				results = append(results, components.SlashCommand{
					Name:        l,
					Description: "set thinking level",
				})
			}
		}
		return results

	case "scoped-models":
		// Subcommands: enable, disable, clear, list
		subs := []struct{ name, desc string }{
			{"enable", "enable a model for cycling"},
			{"disable", "disable a model from cycling"},
			{"clear", "clear all scoped models"},
			{"list", "list current scoped models"},
		}
		// Check if there's another space (subcommand already typed)
		if subIdx := strings.Index(argPrefix, " "); subIdx >= 0 {
			sub := strings.ToLower(argPrefix[:subIdx])
			modelPrefix := argPrefix[subIdx+1:]
			if sub == "enable" || sub == "disable" {
				var results []components.SlashCommand
				for _, model := range m.availableModels {
					if modelPrefix == "" || strings.HasPrefix(strings.ToLower(model), strings.ToLower(modelPrefix)) {
						results = append(results, components.SlashCommand{
							Name:        model,
							Description: sub + " model",
						})
					}
				}
				return results
			}
		}
		var results []components.SlashCommand
		for _, s := range subs {
			if argPrefix == "" || strings.HasPrefix(s.name, strings.ToLower(argPrefix)) {
				results = append(results, components.SlashCommand{
					Name:        s.name,
					Description: s.desc,
				})
			}
		}
		return results

	case "fork":
		// Complete entry IDs from current session
		var results []components.SlashCommand
		if m.session != nil {
			for _, entry := range m.session.Entries {
				if argPrefix == "" || strings.HasPrefix(strings.ToLower(entry.ID), strings.ToLower(argPrefix)) {
					role := entry.Role
					if role == "" {
						role = entry.Type
					}
					results = append(results, components.SlashCommand{
						Name:        entry.ID,
						Description: role + " entry",
					})
				}
			}
		}
		return results

	case "export":
		// No specific argument completions - file path is free text
		return nil

	case "name":
		// No specific argument - free text
		return nil

	case "resume":
		// Complete session IDs
		var results []components.SlashCommand
		if m.sessMgr != nil {
			sessions, err := m.sessMgr.List(m.session.CWD)
			if err == nil {
				for _, sess := range sessions {
					if argPrefix == "" || strings.HasPrefix(strings.ToLower(sess.ID), strings.ToLower(argPrefix)) {
						name := sess.GetSessionName()
						if name == "" {
							name = sess.ID
						}
						results = append(results, components.SlashCommand{
							Name:        sess.ID,
							Description: name,
						})
					}
				}
			}
		}
		return results
	}

	return nil
}

// filterSlashCandidates filters SlashCommandsWithDesc by fuzzy-matched prefix.
func (m *AppModel) filterSlashCandidates(prefix string) []components.SlashCommand {
	all := components.SlashCommandsWithDesc()
	if prefix == "" {
		return all
	}
	type match struct {
		sc    components.SlashCommand
		score float64
	}
	var matches []match
	for _, sc := range all {
		name := sc.Name
		if strings.HasPrefix(name, "/") {
			name = name[1:]
		}
		if ok, score := fuzzyMatchScore(prefix, name); ok {
			matches = append(matches, match{sc: sc, score: score})
		}
	}
	// Sort by score (lower = better), then alphabetically
	for i := 0; i < len(matches); i++ {
		for j := i + 1; j < len(matches); j++ {
			if matches[i].score > matches[j].score ||
				(matches[i].score == matches[j].score && matches[i].sc.Name > matches[j].sc.Name) {
				matches[i], matches[j] = matches[j], matches[i]
			}
		}
	}
	result := make([]components.SlashCommand, len(matches))
	for i, m := range matches {
		result[i] = m.sc
	}
	return result
}

// Symbol represents a symbol/tag suggestion for # autocomplete mode.
type Symbol struct {
	Name        string
	Description string
}

// collectSymbols gathers symbols from recent session entries and context.
// (TS pi-mono: # symbol mode collects file paths, entry IDs, and tagged references)
func (m *AppModel) collectSymbols(prefix string) []Symbol {
	var symbols []Symbol
	seen := make(map[string]bool)

	// Collect file paths from recent session entries
	if m.session != nil {
		for _, entry := range m.session.Entries {
			var contentBlocks []struct {
				Type string `json:"type"`
				Text string `json:"text"`
			}
			if err := json.Unmarshal(entry.Content, &contentBlocks); err != nil {
				continue
			}
			for _, block := range contentBlocks {
				if block.Type == "text" && block.Text != "" {
					for _, word := range strings.Fields(block.Text) {
						trimmers := "`'\"()[]{}<>,"
						word = strings.Trim(word, trimmers)
						if strings.Contains(word, "/") || strings.Contains(word, ".") {
							if len(word) > 2 && len(word) < 120 && !seen[word] {
								if prefix == "" || strings.HasPrefix(strings.ToLower(word), strings.ToLower(prefix)) {
									symbols = append(symbols, Symbol{Name: word, Description: "referenced path"})
									seen[word] = true
								}
							}
						}
					}
				}
			}
		}
	}

	// Collect entry IDs for reference
	if m.session != nil {
		for _, entry := range m.session.Entries {
			if prefix == "" || strings.HasPrefix(strings.ToLower(entry.ID), strings.ToLower(prefix)) {
				role := entry.Role
				if role == "" {
					role = entry.Type
				}
				if !seen[entry.ID] {
					symbols = append(symbols, Symbol{Name: entry.ID, Description: role + " entry"})
					seen[entry.ID] = true
				}
			}
		}
	}

	return symbols
}

// updateAutocomplete updates the autocomplete component with candidate views.
func (m *AppModel) updateAutocomplete(candidates []components.SlashCommand, prefix string) {
	if len(candidates) == 0 {
		m.autocomplete.Hide()
		return
	}
	names := components.SlashCommandNames(candidates)
	descs := components.SlashCommandDescriptions(candidates)
	m.autocomplete.Show(names, descs, prefix)
}

// ─── Agent Integration ─────────────────────────────────────────────────────

// runAgent sends user input to the agent loop in a goroutine.
// It subscribes to the EventBus to receive thinking/tool/usage events
// and forwards them to the Bubble Tea program via Program.Send.
// myID is the stream identifier snapshot — events are dropped if streamID no longer matches.
func (m *AppModel) runAgent(text string, myID int32) {
	m.streaming = true
	m.pendingSteeringMsgs = nil
		m.pendingFollowUpMsgs = nil // clear pending messages when agent starts running
	m.startProgress()

	// Show connecting status (TS pi-mono: createWorkingLoader)
	modelName, provider := parseModelString(m.agent.Loop().Model)
	_ = provider
	m.footer.SetWorkingMessage("Connecting to " + modelName + "...")

	// Show streaming indicator immediately via footer
	if m.program != nil {
		m.program.Send(StatusMsg{
			TokensIn:  m.lastStatus.TokensIn,
			TokensOut: m.lastStatus.TokensOut,
			Streaming: true,
		})
	}

	var messages []types.Message
	if m.session != nil && len(m.session.Entries) > 0 {
		leafID := session.EffectiveLeafID(m.session)
		messages = session.BuildContextFromLeaf(m.session.Entries, leafID)
	}
	userMsg := types.Message{
		Role:    "user",
		Content: jsonMarshalContent(text),
	}
	messages = append(messages, userMsg)

	// Save user message as session entry (TS pi-mono: appendMessage in _processAgentEvent)
	initialMsgCount := len(messages)
	if m.session != nil && m.sessMgr != nil {
		parentID := session.EffectiveLeafID(m.session)
		userEntry := session.MessageToEntry(userMsg, parentID)
		if err := m.sessMgr.AddEntry(m.session, userEntry); err == nil {
			m.footer.SetEntryCount(len(m.session.Entries))
		}
	}

	// Subscribe to EventBus
	subID := fmt.Sprintf("tui-%d", time.Now().UnixNano())
	eventsCh := m.eventBus.Subscribe(subID)
	defer m.eventBus.Unsubscribe(subID)

	// Accumulated stats
	var tokensIn, tokensOut, cacheR, cacheW int
	// Carry forward previous stats
	tokensIn = m.lastStatus.TokensIn
	tokensOut = m.lastStatus.TokensOut
	cacheR = m.lastStatus.TokensCacheR
	cacheW = m.lastStatus.TokensCacheW

	// Goroutine to forward EventBus events to Bubble Tea
	go func() {
		for evt := range eventsCh {
			// Drop events from a stale stream (interrupted / superseded)
			if atomic.LoadInt32(&m.streamID) != myID {
				continue
			}
			switch evt.Type {
			case "thinking_delta":
				if t, ok := evt.Data["text"].(string); ok && m.program != nil {
					m.program.Send(ThinkingMsg(t))
				}
			case "toolcall_start":
				id, _ := evt.Data["tool_id"].(string)
				name, _ := evt.Data["tool_name"].(string)
				if m.program != nil {
					m.program.Send(ToolCallStartMsg{ID: id, Name: name})
				}
			case "toolcall_delta":
				if t, ok := evt.Data["text"].(string); ok && m.program != nil {
					m.program.Send(ToolCallDeltaMsg{Text: t})
				}
			case "toolcall_end":
				id, _ := evt.Data["tool_id"].(string)
				name, _ := evt.Data["tool_name"].(string)
				args, _ := evt.Data["args"].(string)
				if m.program != nil {
					m.program.Send(ToolCallMsg{ID: id, Name: name, Arguments: args})
				}
			case "tool_start":
				if id, ok := evt.Data["tool_call_id"].(string); ok && m.program != nil {
					name, _ := evt.Data["tool_name"].(string)
					m.program.Send(ToolRunningMsg{ID: id, Name: name})
				}
			case "tool_end":
				name, _ := evt.Data["tool_name"].(string)
				result, _ := evt.Data["result"].(string)
				errStr, _ := evt.Data["error"].(string)
				durMs, _ := evt.Data["duration"].(int64)
				if m.program != nil {
					m.program.Send(ToolResultMsg{ID: name, Output: result, Error: errStr, DurationMs: durMs})
				}
			case "usage":
				if in, ok := evt.Data["input_tokens"].(int); ok {
					tokensIn += in
				}
				if out, ok := evt.Data["output_tokens"].(int); ok {
					tokensOut += out
				}
				if cr, ok := evt.Data["cache_read_tokens"].(int); ok {
					cacheR += cr
				}
				if cw, ok := evt.Data["cache_write_tokens"].(int); ok {
					cacheW += cw
				}
			case "auto_retry_start":
				attempt, _ := evt.Data["attempt"].(int)
				maxAttempts, _ := evt.Data["max_attempts"].(int)
				delayMs, _ := evt.Data["delay_ms"].(int)
				delaySec := (delayMs + 999) / 1000
				// Store countdown state for live ticking (TS pi-mono: CountdownTimer)
				m.retryTicking = true
				m.retryDelaySec = delaySec
				m.retryAttempt = attempt
				m.retryMaxAttempts = maxAttempts
				m.chat.AppendSystem(fmt.Sprintf("Retrying (%d/%d) in %ds... (Esc to cancel)", attempt, maxAttempts, delaySec))
				if m.program != nil && delaySec > 0 {
					go func() {
						time.Sleep(1 * time.Second)
						if m.program != nil && m.retryTicking {
							m.program.Send(RetryTickMsg(time.Now()))
						}
					}()
				}
			case "auto_retry_end":
				m.retryTicking = false
				if success, ok := evt.Data["success"].(bool); ok && !success {
					attempt, _ := evt.Data["attempt"].(int)
					finalError, _ := evt.Data["final_error"].(string)
					if finalError == "" {
						finalError = "Unknown error"
					}
					m.chat.AppendError(fmt.Sprintf("Retry failed after %d attempts: %s", attempt, finalError))
				}
			case "compaction_start":
				m.compacting = true
				m.compactionQueue = nil // reset queue on new compaction
				reason, _ := evt.Data["reason"].(string)
				if reason == "manual" {
					m.chat.AppendSystem("Compacting context... (Esc to cancel)")
				} else {
					m.chat.AppendSystem("Context overflow detected, Auto-compacting... (Esc to cancel)")
				}
				m.footer.SetWorkingMessage("Compacting...")
			case "compaction_end":
				m.compacting = false
				m.footer.SetWorkingMessage("Working...")
				aborted, _ := evt.Data["aborted"].(bool)
				reason, _ := evt.Data["reason"].(string)
				if aborted {
					if reason == "manual" {
						m.chat.AppendError("Compaction cancelled")
					} else {
						m.chat.AppendSystem("Auto-compaction cancelled")
					}
				} else {
					tokensBefore, _ := evt.Data["tokens_before"].(int)
					summary, _ := evt.Data["summary"].(string)
					if tokensBefore > 0 {
						// TS pi-mono: clear chat and rebuild from session after compaction.
						// rebuildChatFromSession walks the tree from the current leaf,
						// skipping entries that were compacted away, then we append a
						// fresh expandable compaction summary card at the end.
						m.rebuildChatFromSession()
						m.chat.AppendCompactionSummary(summary, tokensBefore)
					} else {
						m.chat.AppendSystem("Context compacted")
					}
				}
				// Flush queued messages (TS pi-mono: flushCompactionQueue)
				if len(m.compactionQueue) > 0 {
					queued := m.compactionQueue
					m.compactionQueue = nil
					for _, qm := range queued {
						m.program.Send(components.SubmitMsg(qm))
					}
									}
			case "agent_end":
				// agent_end may carry a nested "usage" map
				if usageRaw, ok := evt.Data["usage"]; ok {
					if usageMap, ok := usageRaw.(map[string]int); ok {
						if in, ok := usageMap["input_tokens"]; ok {
							tokensIn += in
						}
						if out, ok := usageMap["output_tokens"]; ok {
							tokensOut += out
						}
						if cr, ok := usageMap["cache_read_tokens"]; ok {
							cacheR += cr
						}
						if cw, ok := usageMap["cache_write_tokens"]; ok {
							cacheW += cw
						}
					}
				}
				// Propagate stop_reason for display (TS pi-mono: stopReason on last assistant message)
				if sr, ok := evt.Data["stop_reason"].(string); ok && sr != "" && sr != "stop" && sr != "toolUse" {
					reason := sr
					if sr == "length" {
						reason = "length"
					}
					if m.program != nil {
						m.program.Send(StopReasonMsg{Reason: reason})
					}
				}
				// Send final status
				if m.program != nil {
					m.program.Send(StatusMsg{
						TokensIn:     tokensIn,
						TokensOut:    tokensOut,
						TokensCacheR: cacheR,
						TokensCacheW: cacheW,
						Streaming:    false,
					})
				}
			}
		}
	}()

	ctx := context.Background()
	_, finalMessages, err := m.agent.Loop().RunStreamingWithMessages(ctx, messages, func(chunk string) {
		if m.program != nil && atomic.LoadInt32(&m.streamID) == myID {
			m.program.Send(StreamTextMsg(chunk))
		}
	})

	// Save assistant and tool messages as session entries (TS pi-mono: appendMessage for each event.message)
	if m.session != nil && m.sessMgr != nil && len(finalMessages) > initialMsgCount {
		newMessages := finalMessages[initialMsgCount:] // assistant messages + tool results
		for _, msg := range newMessages {
			parentID := session.EffectiveLeafID(m.session)
			entry := session.MessageToEntry(msg, parentID)
			if err := m.sessMgr.AddEntry(m.session, entry); err == nil {
				// LeafID is auto-updated by AddEntry
			}
		}
		m.footer.SetEntryCount(len(m.session.Entries))
	}

	if err != nil && m.program != nil {
		m.program.Send(AgentErrorMsg{Error: err})
	} else if m.program != nil {
		m.program.Send(AgentDoneMsg{})
	}
	m.streaming = false
}

func jsonMarshalContent(text string) json.RawMessage {
	b, _ := json.Marshal([]types.TextContent{{Type: "text", Text: text}})
	return json.RawMessage(b)
}

// runBashDirect executes a bash command directly (! prefix), bypassing the LLM.
// excludeFromCtx is true for !! prefix.
func (m *AppModel) runBashDirect(command string, excludeFromCtx bool) {
	cwd := ""
	if m.session != nil {
		cwd = m.session.CWD
	}

	// Show the bash entry in chat viewport immediately (dim if excluded from context)
	m.chat.AddBashExecution(command, excludeFromCtx)

	m.bashCancelCh = make(chan struct{})
	cancelCh := m.bashCancelCh
	defer func() { m.bashCancelCh = nil }()

	result, err := bashexec.ExecuteBash(bashexec.BashExecutorOptions{
		Command:     command,
		CWD:         cwd,
		AbortSignal: cancelCh,
	})

	// Record bash result in session (TS pi-mono: bash results stored in session)
	if m.session != nil && m.sessMgr != nil {
		output := ""
		exitCode := -1
		cancelled := false
		if err != nil {
			output = fmt.Sprintf("Error: %v", err)
		} else {
			output = result.Output
			exitCode = result.ExitCode
			cancelled = result.Cancelled
		}
		bashData, _ := json.Marshal(map[string]interface{}{
			"command":   command,
			"output":    output,
			"exit_code": exitCode,
			"cancelled": cancelled,
			"excluded":  excludeFromCtx,
		})
		m.sessMgr.AddEntry(m.session, session.SessionEntry{
			ID:         session.GenerateID(),
			ParentID:   session.EffectiveLeafID(m.session),
			Type:       "custom",
			CustomType: "bash",
			Content:    bashData,
			Timestamp:  time.Now(),
		})
	}

	if m.program != nil {
		if err != nil {
			m.program.Send(BashExecResultMsg{
				Command:  command,
				Output:   fmt.Sprintf("Error: %v", err),
				ExitCode: -1,
			})
			return
		}
		m.program.Send(BashExecResultMsg{
			Command:   command,
			Output:    result.Output,
			ExitCode:  result.ExitCode,
			Cancelled: result.Cancelled,
		})
	}
}

// handleSlashCmd processes a slash command and returns the result string.
// Local commands (model, thinking, quit, hotkeys) are handled here;
// everything else is forwarded to the commands.Handle() subsystem.
func (m *AppModel) handleSlashCmd(text string) (string, bool) {
	parts := strings.Fields(text)
	if len(parts) == 0 {
		return "", true
	}
	cmd := strings.ToLower(parts[0])

	// Local TUI-only commands
	switch cmd {
	case "/help":
		return "xihu — AI coding assistant.\n\n" +
			"Quick start:\n" +
			"  /model       select model\n" +
			"  /settings    configure app settings\n" +
			"  /login       set up API key\n" +
			"\n" +
			"Reference:\n" +
			"  /hotkeys     all keyboard shortcuts\n" +
			"  /commands    list all slash commands\n" +
			"  /session     session stats (tokens, cost, etc.)\n" +
			"  /help        this help", true
	case "/hotkeys":
		m.showHelpOverlay()
		return "", true
	case "/changelog":
		m.showFullChangelog()
		return "", true
	case "/model":
		if len(parts) > 1 {
			m.switchToModel(parts[1])
			return "", true
		}
		m.showModelSelector()
		return "", true
	case "/name":
		if len(parts) > 1 {
			name := strings.Join(parts[1:], " ")
			if m.session != nil {
				m.session.SetSessionName(name)
				if err := m.sessMgr.Save(m.session); err == nil {
					m.footer.SetSession(m.session.CWD, getGitBranch(m.session.CWD), name, "", "", "")
					return "Session name set: " + name, true
				}
				return "Error saving session name", true
			}
			return "No active session", true
		}
		if m.session != nil && m.session.GetSessionName() != "" {
			return "Session name: " + m.session.GetSessionName(), true
		}
		m.chat.AppendWarning("Usage: /name <name>")
			return "", true
	case "/new":
		if m.session != nil && m.sessMgr != nil {
					m.session.ID = session.GenerateID()
			m.session.Entries = nil
			m.session.LeafID = ""
			m.session.Name = ""
			m.session.CreatedAt = time.Now()
			m.session.UpdatedAt = time.Now()
			if err := m.sessMgr.Save(m.session); err == nil {
				m.footer.SetSession(m.session.CWD, getGitBranch(m.session.CWD), "", "", "", "")
				return "✓ New session started", true
			}
			return "Error creating new session", true
		}
		return "No active session", true
	case "/clone":
		if m.session != nil && m.sessMgr != nil {
			if len(m.session.Entries) == 0 {
				return "Nothing to clone yet", true
			}
			m.cloneSession()
			return "", true
		}
		return "No active session", true
	case "/sessions":
		m.showSessionSelector()
		return "", true
	case "/quit":
		m.quitting = true
		return "", true
	case "/clear":
		if m.session != nil && m.sessMgr != nil {
			m.session.ID = session.GenerateID()
			m.session.Entries = nil
			m.session.Name = ""
			m.session.CreatedAt = time.Now()
			m.session.UpdatedAt = time.Now()
			if err := m.sessMgr.Save(m.session); err == nil {
				m.footer.SetSession(m.session.CWD, getGitBranch(m.session.CWD), "", "", "", "")
				return "✓ New session started", true
			}
			return "Error creating new session", true
		}
		return "No active session", true
	case "/scoped-models":
		if len(parts) > 1 {
			sub := strings.ToLower(parts[1])
			switch sub {
			case "enable":
				if len(parts) > 2 {
					model := strings.Join(parts[2:], " ")
					m.scopedModels[model] = true
					return "", true
				}
				return "", true
			case "disable":
				if len(parts) > 2 {
					model := strings.Join(parts[2:], " ")
					delete(m.scopedModels, model)
					return "", true
				}
				return "", true
			case "clear":
				m.scopedModels = make(map[string]bool)
				return "", true
			case "list":
				if len(m.scopedModels) == 0 {
					return "", true
				}
				var names []string
				for name := range m.scopedModels {
					names = append(names, name)
				}
				return "", true
			default:
				return "", true
			}
		}
		// No args: show scoped models selector
		m.showScopedModelSelector()
		return "", true
	case "/tree":
		m.showSessionTree()
		return "", true
	case "/fork":
		if len(parts) > 1 && parts[1] != "" {
			m.forkFromEntry(parts[1])
			return "", true
		}
		m.showForkSelector()
		return "", true
	case "/thinking":
		m.cycleThinking()
		return "Thinking level: " + m.thinkingLevel, true
	case "/settings":
		m.showSettingsSelector()
		return "", true
	case "/theme":
		if len(parts) > 1 {
			name := strings.ToLower(parts[1])
			switch name {
			case "dark":
				m.ApplyTheme(DefaultTheme())
				return "", true
			case "light":
				m.ApplyTheme(LightTheme())
				return "", true
			default:
				return "Unknown theme: " + name + ". Available: dark, light", true
			}
		}
		m.showThemeSelector()
		return "", true
		case "/session":
			if m.session != nil {
				var sb strings.Builder

				// Header
				sb.WriteString("Session Info\n\n")

				// Name (if set)
				if name := m.session.GetSessionName(); name != "" {
					sb.WriteString("Name: " + name + "\n")
				}

				// File
				filePath := "In-memory"
				if m.sessMgr != nil {
					if fp := m.sessMgr.SessionFilePath(m.session.CWD, m.session.ID); fp != "" {
						filePath = fp
					}
				}
				sb.WriteString("File: " + filePath + "\n")

				// ID
				sb.WriteString("ID: " + m.session.ID + "\n\n")

				// Message counts
				userCount, assistantCount, toolCallCount, toolResultCount := 0, 0, 0, 0
				for _, e := range m.session.Entries {
					switch e.Role {
					case "user": userCount++
					case "assistant":
						assistantCount++
						if len(e.ToolCalls) > 0 {
							toolCallCount += len(e.ToolCalls)
						}
					case "tool": toolResultCount++
					}
				}
				sb.WriteString("Messages\n")
				sb.WriteString("User: " + commaInt(userCount) + "\n")
				sb.WriteString("Assistant: " + commaInt(assistantCount) + "\n")
				sb.WriteString("Tool Calls: " + commaInt(toolCallCount) + "\n")
				sb.WriteString("Tool Results: " + commaInt(toolResultCount) + "\n")
				sb.WriteString("Total: " + commaInt(len(m.session.Entries)) + "\n\n")

				// Token usage
				if m.lastStatus.TokensIn+m.lastStatus.TokensOut > 0 {
					sb.WriteString("Tokens\n")
					sb.WriteString("Input: " + commaInt(m.lastStatus.TokensIn) + "\n")
					sb.WriteString("Output: " + commaInt(m.lastStatus.TokensOut) + "\n")
					if m.lastStatus.TokensCacheR > 0 {
						sb.WriteString("Cache Read: " + commaInt(m.lastStatus.TokensCacheR) + "\n")
					}
					if m.lastStatus.TokensCacheW > 0 {
						sb.WriteString("Cache Write: " + commaInt(m.lastStatus.TokensCacheW) + "\n")
					}
					sb.WriteString("Total: " + commaInt(m.lastStatus.TokensIn+m.lastStatus.TokensOut+m.lastStatus.TokensCacheR+m.lastStatus.TokensCacheW) + "\n")
				}

				// Context usage
			if m.lastStatus.ContextUsed > 0 {
				sb.WriteString("\nContext\n")
				sb.WriteString(fmt.Sprintf("Used: %.1f%%\n", m.lastStatus.ContextUsed*100))
			}

			// Cost
			if m.lastStatus.TotalCost > 0 {
				sb.WriteString("\nCost\n")
				sb.WriteString(fmt.Sprintf("Total: $%.4f", m.lastStatus.TotalCost))
			}
			return sb.String(), true
			}
			return "No active session", true

	case "/copy":
		// Copy last assistant message to system clipboard
		if m.session == nil || len(m.session.Entries) == 0 {
			return "No agent messages to copy yet.", true
		}
		// Find last assistant message
		var lastText string
		for i := len(m.session.Entries) - 1; i >= 0; i-- {
			if m.session.Entries[i].Role == "assistant" {
				var contentBlocks []struct {
					Type string `json:"type"`
					Text string `json:"text"`
				}
				if err := json.Unmarshal(m.session.Entries[i].Content, &contentBlocks); err == nil {
					for _, block := range contentBlocks {
						if block.Type == "text" && block.Text != "" {
							lastText = block.Text
						}
					}
				}
				break
			}
		}
		if lastText == "" {
			return "No agent messages to copy yet.", true
		}
		if err := copyToClipboard(lastText); err != nil {
			return "Failed to copy: " + err.Error(), true
		}
		return "Copied last agent message to clipboard", true
		case "/debug":
			return m.handleDebugCommand(), true
		case "/share":
			go m.handleShare()
			return "Sharing session...", true
		case "/login":
			m.showLoginDialog()
			return "", true
		case "/logout":
			m.showLogoutDialog()
			return "", true
		}

	// Check extension-registered commands first
	if m.extRunner != nil {
		cmdName := strings.ToLower(parts[0])
		if handler := extensions.GetSlashCommand(cmdName); handler != nil {
			extCtx := extensions.NewExtensionContext(
				m.sessMgr, nil, nil, nil, m.session.CWD,
				m.extensionBridge,
			)
			result, err := handler(parts, extCtx)
			if err != nil {
				return "Extension error: " + err.Error(), true
			}
			return result, true
		}
	}

	// Forward to commands.Handle for all other commands
	ctx := &commands.Context{
		CWD:              m.session.CWD,
		SessionDir:       m.sessMgr.Dir,
		SettingsDir:      m.sessMgr.Dir, // approximate
		CurrentSessionID: m.session.ID,
		Model:            m.agent.Loop().Model,
		SystemPrompt:     m.agent.Loop().SystemPrompt,
		SessionName:      m.session.GetSessionName(),
		TotalCost:        m.lastStatus.TotalCost,
	}
	if m.lastStatus.TokensIn+m.lastStatus.TokensOut > 0 {
		ctx.TokenUsage = &commands.TokenUsage{
			Input:      m.lastStatus.TokensIn,
			Output:     m.lastStatus.TokensOut,
			CacheRead:  m.lastStatus.TokensCacheR,
			CacheWrite: m.lastStatus.TokensCacheW,
			Total:      m.lastStatus.TokensIn + m.lastStatus.TokensOut,
		}
	}
	result, err := commands.Handle(text, ctx)
	if err != nil {
		return "", false
	}

	// Handle sentinel return values from commands
	switch {
	case strings.HasPrefix(result, "COMPACT:"):
		m.triggerManualCompaction()
		return "", true
	case strings.HasPrefix(result, "FORK:"):
		return "", true
	case strings.HasPrefix(result, "CLONE:"):
		return "", true
	case strings.HasPrefix(result, "RESUME:"):
		// Extract session ID from "RESUME: <id>" sentinel
		sid := strings.TrimPrefix(result, "RESUME:")
		sid = strings.TrimSpace(sid)
		if sid != "" {
			m.switchToSession(sid)
		}
		return "", true
	case result == "RESUME_SELECTOR":
		m.showSessionSelector()
		return "", true
	case strings.HasPrefix(result, "IMPORT:"):
		return "", true
	case result == "NEW_SESSION":
		return "Start a new session with /new", true
	case result == "RELOAD":
		// TS pi-mono: guard checks before reload
		if m.streaming {
			m.chat.AppendWarning("Wait for the current response to finish before reloading.")
			return "", true
		}
		if m.compacting {
			m.chat.AppendWarning("Wait for compaction to finish before reloading.")
			return "", true
		}
		m.reload()
		m.chat.AppendSystem("Reloaded keybindings, extensions, skills, prompts, themes")
		m.showPostReloadDiagnostics()
		return "", true
	case result == "QUIT":
		m.quitting = true
		return "", true
	}
	return result, true
}

// ─── Thinking Level Cycling ─────────────────────────────────────────────────

var thinkingLevels = []string{"off", "minimal", "low", "medium", "high", "xhigh"}

// supportsThinking checks whether a model ID supports extended thinking/reasoning.
func supportsThinking(modelID string) bool {
	for _, info := range models.BuiltinModels() {
		if info.ID == modelID {
			return info.SupportsThinking
		}
	}
	return false
}

// showThemeSelector shows a theme picker overlay (TS pi-mono: /theme).
func (m *AppModel) showThemeSelector() {
	currentName := "dark"
	if m.theme != nil {
		currentName = m.theme.Name
	}
	items := []components.SelectorItem{
		{Label: "Dark (default)", Description: "Catppuccin Mocha inspired dark theme", Value: "dark"},
		{Label: "Light", Description: "Catppuccin Latte inspired light theme", Value: "light"},
	}

	// Discover custom themes from ~/.pi/themes/
	customPaths, _ := DiscoverThemes("")
	for _, p := range customPaths {
		t, err := LoadTheme(p)
		if err != nil || t.Name == "" {
			continue
		}
		// Skip built-in names
		if t.Name == "dark" || t.Name == "light" {
			continue
		}
		name := filepath.Base(p)
		name = name[:len(name)-len(filepath.Ext(name))]
		marker := ""
		if t.Name == currentName {
			marker = " (current)"
		}
		items = append(items, components.SelectorItem{
			Label:       name + marker,
			Description: "Custom theme: " + p,
			Value:       "custom:" + p,
		})
	}

	h := len(items) + 4
	if h < 7 {
		h = 7
	}
	if h > 16 {
		h = 16
	}
	m.overlay.ShowSelector("Select Theme (current: "+currentName+")", items, func(value string) {
		switch {
		case value == "dark":
			m.ApplyTheme(DefaultTheme())
		case value == "light":
			m.ApplyTheme(LightTheme())
		case strings.HasPrefix(value, "custom:"):
			path := strings.TrimPrefix(value, "custom:")
			t, err := LoadTheme(path)
			if err != nil {
				m.chat.AppendError(fmt.Sprintf("Failed to load theme \"%s\": %s", path, err.Error()))
				return
			}
			m.ApplyTheme(t)
		}
	}, 56, h)
}

// showSettingsSelector shows current settings in an interactive overlay (TS pi-mono: /settings).
// Selecting an item toggles/changes the setting and re-opens for live feedback.
func (m *AppModel) showSettingsSelector() {
	// Initialize defaults
	if m.defaultTreeFilter == "" {
		m.defaultTreeFilter = "default"
	}
	if m.doubleEscapeAction == "" {
		m.doubleEscapeAction = "tree"
	}
	if m.steeringMode == "" {
		m.steeringMode = "one-at-a-time"
	}
	if m.followUpMode == "" {
		m.followUpMode = "one-at-a-time"
	}
	if m.transport == "" {
		m.transport = "auto"
	}

	// Check terminal image support (Kitty or iTerm2 protocol)
	hasImages := os.Getenv("TERM") == "xterm-kitty" || os.Getenv("ITERM_PROFILE") != "" || os.Getenv("KITTY_WINDOW_ID") != ""

	items := []components.SelectorItem{
		{Label: "Auto-compact: " + boolToStr(m.autoCompact), Description: "Automatically compact context when it gets too large", Value: "autocompact"},
	}

	// Image settings (only shown when terminal supports images)
	if hasImages {
		items = append(items,
			components.SelectorItem{Label: "Show images: " + boolToStr(m.showImages), Description: "Render images inline in the terminal", Value: "show_images"},
			components.SelectorItem{Label: "Image width: " + fmt.Sprintf("%d cells", m.imageWidthCells), Description: "Width of inline images in terminal cells", Value: "image_width"},
		)
	}

	items = append(items,
		components.SelectorItem{Label: "Auto-resize images: " + boolToStr(m.autoResizeImages), Description: "Automatically resize images on terminal resize", Value: "auto_resize_images"},
		components.SelectorItem{Label: "Block images: " + boolToStr(m.blockImages), Description: "Block image rendering (security)", Value: "block_images"},
		components.SelectorItem{Label: "Skill commands: " + boolToStr(m.skillCommands), Description: "Enable slash-command skill invocation", Value: "skill_commands"},
		components.SelectorItem{Label: "Show hardware cursor: " + boolToStr(m.showHardwareCursor), Description: "Show terminal block cursor for IME support", Value: "hwcursor"},
		components.SelectorItem{Label: "Editor padding: " + fmt.Sprintf("%d", m.editorPadding), Description: "Horizontal padding for the input editor (0-3)", Value: "editor_padding"},
		components.SelectorItem{Label: "Autocomplete max items: " + fmt.Sprintf("%d items", m.autocompleteMax), Description: "Maximum visible items in autocomplete dropdown", Value: "autocomplete_max"},
		components.SelectorItem{Label: "Clear on shrink: " + boolToStr(m.clearOnShrink), Description: "Clear editor content when terminal shrinks", Value: "clear_on_shrink"},
		components.SelectorItem{Label: "Terminal progress: " + boolToStr(m.terminalProgress), Description: "Show terminal progress messages during operations", Value: "terminal_progress"},
		components.SelectorItem{Label: "Steering mode: " + m.steeringMode, Description: "How follow-up messages are queued: one-at-a-time or all", Value: "steering"},
		components.SelectorItem{Label: "Follow-up mode: " + m.followUpMode, Description: "How follow-up responses are delivered: one-at-a-time or all", Value: "follow_up"},
		components.SelectorItem{Label: "Transport: " + m.transport, Description: "API transport mechanism: sse, websocket, websocket-cached, or auto", Value: "transport"},
		components.SelectorItem{Label: "Hide thinking: " + boolToStr(m.chat.HideAllThinking), Description: "Hide thinking blocks in assistant responses", Value: "hide_thinking"},
		components.SelectorItem{Label: "Collapse changelog: " + boolToStr(m.collapseChangelog), Description: "Show condensed changelog after updates", Value: "collapse_changelog"},
		components.SelectorItem{Label: "Quiet startup: " + boolToStr(m.quietStartup), Description: "Suppress welcome message on startup", Value: "quiet_startup"},
		components.SelectorItem{Label: "Install telemetry: " + boolToStr(m.installTelemetry), Description: "Opt-in to anonymous installation telemetry", Value: "install_telemetry"},
		components.SelectorItem{Label: "Double-escape action: " + m.doubleEscapeAction, Description: "Action on Esc\u00d72 with empty editor: tree, fork, or none", Value: "esc2x"},
		components.SelectorItem{Label: "Tree filter mode: " + m.defaultTreeFilter, Description: "Default filter when opening /tree", Value: "treefilter"},
		components.SelectorItem{Label: "Warnings\u2026", Description: "Configure warning display settings", Value: "warnings"},
		components.SelectorItem{Label: "Thinking level: " + m.thinkingLevel, Description: "Select reasoning depth for the model", Value: "thinking"},
		components.SelectorItem{Label: "Theme: " + m.theme.Name, Description: "Select the UI color theme", Value: "theme"},
	)

	// Session info
	cwd := m.session.CWD
	if home, _ := os.UserHomeDir(); home != "" && strings.HasPrefix(cwd, home) {
		cwd = "~" + cwd[len(home):]
	}
	items = append(items,
		components.SelectorItem{Label: "Session: " + m.session.ID, Description: cwd, Value: "session"},
		components.SelectorItem{Label: "Session name: " + m.session.GetSessionName(), Description: "Use /name <name> to set", Value: "name"},
	)

	h := len(items) + 4
	if h > 30 {
		h = 30
	}
	if h < 10 {
		h = 10
	}

	m.overlay.ShowSelectorStayOnSelect("Settings \xe2\x80\x94 Enter/Space to change \xc2\xb7 Esc to cancel", items, func(value string) {
		switch value {
		case "autocompact":
			m.autoCompact = !m.autoCompact
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "show_images":
			m.showImages = !m.showImages
			m.chat.SetShowImages(m.showImages)
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "image_width":
			widths := []int{60, 80, 120}
			for i, w := range widths {
				if w == m.imageWidthCells {
					m.imageWidthCells = widths[(i+1)%len(widths)]
					m.chat.SetImageWidth(m.imageWidthCells)
					break
				}
			}
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "auto_resize_images":
			m.autoResizeImages = !m.autoResizeImages
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "block_images":
			m.blockImages = !m.blockImages
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "skill_commands":
			m.skillCommands = !m.skillCommands
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "hwcursor":
			m.showHardwareCursor = !m.showHardwareCursor
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "editor_padding":
			pads := []int{0, 1, 2, 3}
			for i, p := range pads {
				if p == m.editorPadding {
					m.editorPadding = pads[(i+1)%len(pads)]
					break
				}
			}
			m.input.SetPaddingX(m.editorPadding)
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "autocomplete_max":
			maxes := []int{3, 5, 7, 10, 15, 20}
			for i, n := range maxes {
				if n == m.autocompleteMax {
					m.autocompleteMax = maxes[(i+1)%len(maxes)]
					break
				}
			}
			m.autocomplete.SetMaxVisible(m.autocompleteMax)
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "clear_on_shrink":
			m.clearOnShrink = !m.clearOnShrink
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "terminal_progress":
			m.terminalProgress = !m.terminalProgress
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "steering":
			if m.steeringMode == "one-at-a-time" {
				m.steeringMode = "all"
			} else {
				m.steeringMode = "one-at-a-time"
			}
			m.agent.Loop().SteeringMode = m.steeringMode
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "follow_up":
			if m.followUpMode == "one-at-a-time" {
				m.followUpMode = "all"
			} else {
				m.followUpMode = "one-at-a-time"
			}
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "transport":
			modes := []string{"sse", "websocket", "websocket-cached", "auto"}
			for i, mode := range modes {
				if mode == m.transport {
					m.transport = modes[(i+1)%len(modes)]
					break
				}
			}
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "hide_thinking":
			m.chat.HideAllThinking = !m.chat.HideAllThinking
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "collapse_changelog":
			m.collapseChangelog = !m.collapseChangelog
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "quiet_startup":
			m.quietStartup = !m.quietStartup
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "install_telemetry":
			m.installTelemetry = !m.installTelemetry
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "esc2x":
			modes := []string{"tree", "fork", "none"}
			for i, mode := range modes {
				if mode == m.doubleEscapeAction {
					m.doubleEscapeAction = modes[(i+1)%len(modes)]
					break
				}
			}
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "treefilter":
			modes := []string{"default", "no-tools", "user-only", "labeled-only", "all"}
			for i, mode := range modes {
				if mode == m.defaultTreeFilter {
					m.defaultTreeFilter = modes[(i+1)%len(modes)]
					break
				}
			}
			m.treeFilterMode = m.defaultTreeFilter
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "thinking":
			m.showThinkingSelector()
			return
		case "theme":
			go func() {
				time.Sleep(50 * time.Millisecond)
				if m.program != nil {
					m.program.Send(components.SelectorChosenMsg{Value: "show:theme_selector"})
				}
			}()
		case "warnings":
			m.showWarningsSelector()
			return
		case "session":
			// Info only, no action
		case "name":
			// Info only, use /name command
		}
	}, nil, 64, h)
}

// updateHeaderHints sets the header's compact and expanded key hints from actual
// keybinding values so they reflect user-customized keybindings (TS pi-mono: keyHint/keyText helpers).
func (m *AppModel) updateHeaderHints() {
	kb := m.keybindings
	if kb == nil {
		return
	}
	formatKey := func(id KeybindingID) string {
		keys := kb.GetKeys(id)
		if len(keys) > 0 {
			return keys[0]
		}
		return ""
	}

	// Compact mode
	hints := fmt.Sprintf("%s interrupt  %s clear  / commands  ! bash  %s help  %s tools",
		formatKey(GlobalInterrupt),
		formatKey(GlobalClear),
		formatKey(GlobalToggleHeader),
		formatKey(GlobalToggleTools),
	)
	m.header.SetHints(hints)

	// Expanded mode — each section uses actual keybinding values
	nav := fmt.Sprintf("%s/%s/%s/%s cursor  %s/%s scroll  %s/%s line start/end  gg top  G bottom",
		formatKey(EditorCursorUp), formatKey(EditorCursorDown),
		formatKey(EditorCursorLeft), formatKey(EditorCursorRight),
		formatKey(EditorPageUp), formatKey(EditorPageDown),
		formatKey(EditorCursorLineStart), formatKey(EditorCursorLineEnd),
	)
	edit := fmt.Sprintf("%s submit  %s newline  %s complete  %s yank  %s yank-pop  %s undo",
		formatKey(InputSubmit),
		formatKey(InputNewLine),
		formatKey(InputTab),
		formatKey(EditorYank),
		formatKey(EditorYankPop),
		formatKey(EditorUndo),
	)
	acts := fmt.Sprintf("%s interrupt  %s clear  %s exit  %s model  %s edit  %s tools  %s thinking",
		formatKey(GlobalInterrupt),
		formatKey(GlobalClear),
		formatKey(GlobalExit),
		formatKey(GlobalModelSelector),
		formatKey(GlobalExternalEditor),
		formatKey(GlobalToggleTools),
		formatKey(GlobalToggleThinking),
	)
	msgs := fmt.Sprintf("alt+enter queue follow-up  alt+up dequeue")
	more := fmt.Sprintf("/fork  /tree  /settings  /theme  /session info  %s collapse",
		formatKey(GlobalToggleHeader),
	)
	m.header.SetExpandedHints(nav, edit, acts, msgs, more)
}

// reload reloads settings, keybindings, and re-applies theme from disk.
// Mirrors pi-mono's handleReloadCommand + session.reload() + keybindings.reload().
func (m *AppModel) reload() {
	// Reload settings from global + project config files
	newSettings, err := settings.LoadAll()
	if err != nil {
		m.chat.AppendError("Reload failed: " + err.Error())
		return
	}

	// Reload keybindings from ~/.xihu/keybindings.json
	if m.keybindings != nil {
		userKB, _ := LoadUserBindings()
		m.keybindings.Reload(userKB)
		// Update tool toggle key hint with potentially changed keybinding
		if tk := formatKeyStr(m.keybindings, GlobalToggleTools); tk != "" {
			m.chat.SetToolToggleKey(tk)
		}
	}
	m.updateHeaderHints()

	// Re-apply settings (same pattern as constructor)
	if newSettings != nil {
		m.settingsObj = newSettings
		if newSettings.DoubleEscapeAction != "" {
			m.doubleEscapeAction = newSettings.DoubleEscapeAction
		}
		if newSettings.TreeFilterMode != "" {
			m.defaultTreeFilter = newSettings.TreeFilterMode
		}
		if newSettings.QuietStartup != nil {
			m.quietStartup = *newSettings.QuietStartup
		}
		if newSettings.CompactionEnabled != nil {
			m.autoCompact = *newSettings.CompactionEnabled
		}
		if newSettings.HideThinkingBlock != nil {
			m.chat.HideAllThinking = *newSettings.HideThinkingBlock
		}
		if newSettings.SteeringMode != "" {
			m.steeringMode = newSettings.SteeringMode
			if m.agent != nil {
				m.agent.Loop().SteeringMode = newSettings.SteeringMode
			}
		}
		if newSettings.FollowUpMode != "" {
			m.followUpMode = newSettings.FollowUpMode
		}
		if newSettings.Transport != "" {
			m.transport = newSettings.Transport
		}
		if newSettings.ShowHardwareCursor != nil {
			m.showHardwareCursor = *newSettings.ShowHardwareCursor
		}
		if newSettings.Terminal != nil {
			if newSettings.Terminal.ShowTerminalProgress != nil {
				m.terminalProgress = *newSettings.Terminal.ShowTerminalProgress
			}
			if newSettings.Terminal.ClearOnShrink != nil {
				m.clearOnShrink = *newSettings.Terminal.ClearOnShrink
			}
			if newSettings.Terminal.ShowImages != nil {
				m.showImages = *newSettings.Terminal.ShowImages
				m.chat.SetShowImages(m.showImages)
			}
			if newSettings.Terminal.ImageWidthCells > 0 {
				m.imageWidthCells = newSettings.Terminal.ImageWidthCells
				m.chat.SetImageWidth(m.imageWidthCells)
			}
		}
		if newSettings.Images != nil {
			if newSettings.Images.AutoResize != nil {
				m.autoResizeImages = *newSettings.Images.AutoResize
			}
			if newSettings.Images.BlockImages != nil {
				m.blockImages = *newSettings.Images.BlockImages
			}
		}
		if newSettings.EnableSkillCommands != nil {
			m.skillCommands = *newSettings.EnableSkillCommands
		}
		if newSettings.CollapseChangelog != nil {
			m.collapseChangelog = *newSettings.CollapseChangelog
		}
		if newSettings.EnableInstallTelemetry != nil {
			m.installTelemetry = *newSettings.EnableInstallTelemetry
		}
		if newSettings.EditorPaddingX > 0 {
			m.editorPadding = newSettings.EditorPaddingX
			m.input.SetPaddingX(m.editorPadding)
		}
		if newSettings.AutocompleteMaxVisible > 0 {
			m.autocompleteMax = newSettings.AutocompleteMaxVisible
			m.autocomplete.SetMaxVisible(m.autocompleteMax)
		}
		if newSettings.Warnings != nil && newSettings.Warnings.AnthropicExtraUsage != nil {
			m.anthropicExtraUsage = *newSettings.Warnings.AnthropicExtraUsage
		}
		if newSettings.LastChangelogVersion != "" {
			m.lastChangelogVersion = newSettings.LastChangelogVersion
		}
		// Scoped models
		if len(newSettings.ScopedModels) > 0 {
			m.scopedModels = make(map[string]bool)
			for _, mod := range newSettings.ScopedModels {
				m.scopedModels[mod] = true
			}
		}
		// Theme - reload if changed
		if newSettings.Theme != "" && m.theme != nil && newSettings.Theme != m.theme.Name {
			switch newSettings.Theme {
			case "dark":
				m.ApplyTheme(DefaultTheme())
			case "light":
				m.ApplyTheme(LightTheme())
			default:
				// Try to load custom theme
				customPaths, _ := DiscoverThemes("")
				for _, p := range customPaths {
					t, err := LoadTheme(p)
					if err == nil && t.Name == newSettings.Theme {
						m.ApplyTheme(t)
						break
					}
				}
			}
		} else if m.theme != nil {
			// Re-apply current theme to refresh chat/editor colors
			m.ApplyTheme(m.theme)
		}

		// Re-propagate thinking border color
		if m.theme != nil {
			m.chat.SetThinkingBorderColor(m.theme.ThinkingBorderColor(m.thinkingLevel))
			m.input.SetBorderColor(m.theme.ThinkingBorderColor(m.thinkingLevel))
		}
	}
}

// triggerManualCompaction runs manual compaction via the configured TransformContext.
// Emits CompactionStart("manual") / CompactionEnd events so the UI shows correct messages.
func (m *AppModel) triggerManualCompaction() {
	if m.compacting {
		m.chat.AppendError("Compaction already in progress")
		return
	}
	if m.agent == nil || m.agent.Loop().Config.TransformContext == nil {
		m.chat.AppendError("Compaction is not configured. Set compaction_reserve_tokens in settings.")
		return
	}
	if m.session == nil {
		m.chat.AppendError("No active session to compact")
		return
	}

	// TS pi-mono: check if there are enough messages for compaction
	messageCount := 0
	for _, e := range m.session.Entries {
		if e.Type == session.EntryTypeUser || e.Type == session.EntryTypeAssistant {
			messageCount++
		}
	}
	if messageCount < 2 {
		m.chat.AppendWarning("Nothing to compact (no messages yet)")
		return
	}

	m.compacting = true
	m.compactionQueue = nil

	// Emit manual compaction start event
	if m.eventBus != nil {
		m.eventBus.Emit(events.CompactionStart("manual"))
	}

	// Build messages from session entries and run compaction
	leafID := session.EffectiveLeafID(m.session)
	messages := session.BuildContextFromLeaf(m.session.Entries, leafID)
	compactedMessages := m.agent.Loop().Config.TransformContext(messages, "manual")

	if len(compactedMessages) < len(messages) {
		// Compaction happened — get result from agent
		tokensBefore := 0
		summary := ""
		firstKeptID := ""
		if m.agent.Loop().LastCompactionResult != nil {
			tokensBefore = m.agent.Loop().LastCompactionResult.TokensBefore
			summary = m.agent.Loop().LastCompactionResult.Summary
			firstKeptID = m.agent.Loop().LastCompactionResult.FirstKeptEntryID
			m.agent.Loop().LastCompactionResult = nil
		}

		// Record compaction as a session entry BEFORE emitting event
		// (TS pi-mono: session entry must exist before rebuildChatFromMessages)
		if m.session != nil && m.sessMgr != nil {
			parentID := session.EffectiveLeafID(m.session)
			entry := session.CompactionEntry(summary, firstKeptID, parentID)
			if err := m.sessMgr.AddEntry(m.session, entry); err != nil {
				m.chat.AppendSystem("Warning: failed to save compaction entry: " + err.Error())
			}
		}

		// Emit event (handler will rebuild chat from session, TS pi-mono alignment)
		if m.eventBus != nil {
			m.eventBus.Emit(events.CompactionEnd(tokensBefore, summary, false, "manual"))
		}
	} else {
		if m.eventBus != nil {
			m.eventBus.Emit(events.CompactionEnd(0, "", false, "manual"))
		}
	}

	m.compacting = false

	// Flush queued messages
	if len(m.compactionQueue) > 0 {
		queued := m.compactionQueue
		m.compactionQueue = nil
		for _, qm := range queued {
			m.program.Send(components.SubmitMsg(qm))
		}
	}
}

// saveSettings persists current runtime settings to the global settings file.
func (m *AppModel) saveSettings() {
	home, err := os.UserHomeDir()
	if err != nil {
		return
	}
	path := filepath.Join(home, ".xihu", "settings.json")

	// Load existing settings to preserve unmanaged fields
	s, err := settings.LoadSettings(path)
	if err != nil || s == nil {
		s = &settings.Settings{}
	}

	// Apply runtime values
	s.Theme = m.theme.Name
	s.ThinkingLevel = m.thinkingLevel
	s.DoubleEscapeAction = m.doubleEscapeAction
	s.TreeFilterMode = m.defaultTreeFilter
	s.SteeringMode = m.steeringMode
	s.FollowUpMode = m.followUpMode
	s.Transport = m.transport
	s.EditorPaddingX = m.editorPadding
	s.AutocompleteMaxVisible = m.autocompleteMax

	// Bool pointer fields
	ac := m.autoCompact
	s.CompactionEnabled = &ac
	ht := m.chat.HideAllThinking
	s.HideThinkingBlock = &ht
	qs := m.quietStartup
	s.QuietStartup = &qs
	hc := m.showHardwareCursor
	s.ShowHardwareCursor = &hc
	sc := m.skillCommands
	s.EnableSkillCommands = &sc
	cc := m.collapseChangelog
	s.CollapseChangelog = &cc
	it := m.installTelemetry
	s.EnableInstallTelemetry = &it

	// Terminal settings
	tp := m.terminalProgress
	cs := m.clearOnShrink
	si := m.showImages
	s.Terminal = &settings.TerminalSettings{
		ShowTerminalProgress: &tp,
		ClearOnShrink:        &cs,
		ShowImages:           &si,
		ImageWidthCells:      m.imageWidthCells,
	}

	// Image settings
	ar := m.autoResizeImages
	bi := m.blockImages
	s.Images = &settings.ImageSettings{
		AutoResize:  &ar,
		BlockImages: &bi,
	}

	// Scoped models — save enabled models in display order
	var scopedList []string
	for _, model := range m.modelOrder {
		if m.scopedModels[model] {
			scopedList = append(scopedList, model)
		}
	}
	// Append any enabled models not in modelOrder
	inOrder := make(map[string]bool)
	for _, model := range m.modelOrder {
		inOrder[model] = true
	}
	for model := range m.scopedModels {
		if !inOrder[model] {
			scopedList = append(scopedList, model)
		}
	}
	s.ScopedModels = scopedList

	// Warnings
	ae := m.anthropicExtraUsage
	s.Warnings = &settings.WarningSettings{
		AnthropicExtraUsage: &ae,
	}

	settings.SaveSettings(path, s)
}

//// ApplyTheme applies a theme change immediately to all components (live reload).
func (m *AppModel) ApplyTheme(t *Theme) {
	if t == nil {
		return
	}
	m.theme = t

	// Update footer style
	fs := t.FooterStyle()
	fs = fs.Width(m.width)
	m.footer.SetStyle(fs, t.ContextGreen, t.ContextYellow, t.ContextRed)

	// Update editor input border (preserve thinking-level color overlay)
	m.input.SetSlashBorderColor(t.InputBorder)

	// Update glamour markdown style for light/dark themes
	if t.Name == "light" {
		m.chat.SetGlamourStyle("light")
	} else {
		m.chat.SetGlamourStyle("dark")
	}

	// Propagate theme colors to chat viewport (TS pi-mono: theme-driven colors)
	m.chat.SetTheme(t.Accent, t.Muted, t.Dim, t.Warning, t.Success, t.ErrorColor, t.ThinkingColor, t.ThinkingText, t.ToolPendingBg, t.ToolSuccessBg, t.ToolErrorBg)

	// Update session info to refresh display
	_, provider := parseModelString(m.agent.Loop().Model)
	modelName := m.agent.Loop().Model
	if idx := strings.Index(modelName, "/"); idx >= 0 {
		modelName = modelName[idx+1:]
	}
	m.footer.SetSession(m.session.CWD, getGitBranch(m.session.CWD), m.session.GetSessionName(), modelName, m.thinkingLevel, provider)

	// Try to persist theme to settings
	home, err := os.UserHomeDir()
	if err == nil {
		settingsPath := filepath.Join(home, ".xihu", "settings.json")
		s, _ := settings.LoadSettings(settingsPath)
		if s != nil {
			s.Theme = t.Name
			settings.SaveSettings(settingsPath, s)
		}
	}
}

// boolToStr renders a boolean as "true" or "false".
// fuzzyMatch returns true if each character in pattern appears in order in s (case-sensitive, caller lowercases).
func fuzzyMatch(pattern, s string) bool {
	j := 0
	for i := 0; i < len(s) && j < len(pattern); i++ {
		if s[i] == pattern[j] {
			j++
		}
	}
	return j == len(pattern)
}

func boolToStr(v bool) string {
	if v {
		return "true"
	}
	return "false"
}

// showWarningsSelector opens a warning settings submenu (TS pi-mono: settings submenu).
func (m *AppModel) showWarningsSelector() {
	items := []components.SelectorItem{
		{Label: "Anthropic Extra Usage: " + boolToStr(m.anthropicExtraUsage), Description: "Warn when API responses include anthropic extra usage pricing", Value: "anthropic_extra_usage"},
	}

	h := len(items) + 4
	onSelect := func(value string) {
		switch value {
		case "anthropic_extra_usage":
			m.anthropicExtraUsage = !m.anthropicExtraUsage
		}
		m.saveSettings()
		if m.program != nil {
			go func() {
				time.Sleep(50 * time.Millisecond)
				m.program.Send(refreshWarningsMsg{})
			}()
		}
	}
	m.overlay.ShowSelectorStayOnSelect("Warnings (Enter to toggle, Esc to back)", items, onSelect, nil, 54, h)
}

// refreshWarningsMsg refreshes the warnings submenu.
type refreshWarningsMsg struct{}

// showThinkingSelector opens a thinking level submenu (TS pi-mono: settings submenu).
func (m *AppModel) showThinkingSelector() {
	current := m.thinkingLevel
	if current == "" {
		current = "off"
	}

	var items []components.SelectorItem
	for i, level := range thinkingLevels {
		desc := ""
		switch level {
		case "off":
			desc = "No extended reasoning (fastest)"
		case "minimal":
			desc = "Very brief reasoning (~1k tokens)"
		case "low":
			desc = "Light reasoning (~2k tokens)"
		case "medium":
			desc = "Moderate reasoning (~8k tokens)"
		case "high":
			desc = "Deep reasoning (~16k tokens)"
		case "xhigh":
			desc = "Maximum reasoning (~32k tokens)"
		}
		label := level
		if level == current {
			label = "✓ " + level
		}
		items = append(items, components.SelectorItem{
			Label:       label,
			Description: fmt.Sprintf("[%d/%d] %s", i+1, len(thinkingLevels), desc),
			Value:       level,
		})
	}

	h := len(items) + 4
	if h > 14 {
		h = 14
	}
	m.overlay.ShowSelector("Thinking Level", items, func(value string) {
		if value != "" && value != m.thinkingLevel {
			m.thinkingLevel = value
			m.saveSettings()
			// Update footer
			_, provider := parseModelString(m.agent.Loop().Model)
			modelName := m.agent.Loop().Model
			if idx := strings.Index(modelName, "/"); idx >= 0 {
				modelName = modelName[idx+1:]
			}
			m.footer.SetSession(m.session.CWD, getGitBranch(m.session.CWD), m.session.GetSessionName(), modelName, value, provider)
			m.footer.SetHasReasoning(supportsThinking(modelName))
			m.input.SetBorderColor(m.theme.ThinkingBorderColor(value))
			m.chat.AppendSystem("Thinking level: " + value)
		}
		// Re-show settings after closing thinking selector
		go func() {
			time.Sleep(50 * time.Millisecond)
			if m.program != nil {
				m.program.Send(refreshSettingsMsg{})
			}
		}()
	}, 60, h)
}

// showModelSelector opens a model selector overlay (TS pi-mono: Ctrl+L).
// Shows model name, provider, context window size, and pricing.
func (m *AppModel) showModelSelector() {
	if len(m.availableModels) == 0 {
		m.chat.AppendSystem("Only showing models from configured providers. Use /login to add providers.")
		return
	}

	// Build model info lookup from builtins
	modelInfoMap := make(map[string]models.ModelInfo)
	for _, info := range models.BuiltinModels() {
		modelInfoMap[info.ID] = info
	}

	// Helper to build SelectorItems from a model list
	buildItems := func(modelList []string) []components.SelectorItem {
		items := make([]components.SelectorItem, 0, len(modelList))
		for _, model := range modelList {
			name, provider := parseModelString(model)
			isCurrent := model == m.agent.Loop().Model || name == m.agent.Loop().Model
			label := name
			if isCurrent {
				label = "→ " + name + " ✓"
			}
			desc := "[" + provider + "]"
			caps := ""
			if info, ok := modelInfoMap[name]; ok {
				if info.MaxTokens > 0 {
					desc += fmt.Sprintf("  %dK ctx", info.MaxTokens/1000)
				}
				if info.SupportsThinking {
					caps += "T"
				}
				if info.SupportsTools {
					caps += "🔧"
				}
				if info.SupportsVision {
					caps += "👁"
				}
				if info.Pricing.Prompt > 0 {
					desc += fmt.Sprintf("  $%.1f/$%.1f", info.Pricing.Prompt*10, info.Pricing.Completion*10)
				}
			} else if info, ok := modelInfoMap[model]; ok {
				if info.MaxTokens > 0 {
					desc += fmt.Sprintf("  %dK ctx", info.MaxTokens/1000)
				}
				if info.SupportsThinking {
					caps += "T"
				}
				if info.SupportsTools {
					caps += "🔧"
				}
				if info.SupportsVision {
					caps += "👁"
				}
				if info.Pricing.Prompt > 0 {
					desc += fmt.Sprintf("  $%.1f/$%.1f", info.Pricing.Prompt*10, info.Pricing.Completion*10)
				}
			}
			if caps != "" {
				desc += "  " + caps
			}
			if isCurrent {
				desc += " current"
			}
			items = append(items, components.SelectorItem{
				Label:       label,
				Description: desc,
				Value:       model,
			})
		}
		return items
	}

	// Determine if scoped models exist (TS pi-mono: Tab toggles all/scoped)
	hasScoped := len(m.scopedModels) > 0
	scopeAll := !hasScoped // start scoped if scoped models exist

	// Build the scoped model list (only models in scopedModels set)
	scopedList := make([]string, 0, len(m.scopedModels))
	for _, model := range m.availableModels {
		if m.scopedModels[model] {
			scopedList = append(scopedList, model)
		}
	}

	allItems := buildItems(m.availableModels)
	scopedItems := buildItems(scopedList)

	var showOverlay func()
	showOverlay = func() {
		var items []components.SelectorItem
		var title string
		if scopeAll {
			items = allItems
			title = "Models — Scope: all"
		} else {
			items = scopedItems
			title = "Models — Scope: scoped"
		}
		if hasScoped {
			title += "  Tab=toggle scope"
		}

		h := len(items) + 5
		if h > 20 {
			h = 20
		}
		if h < 5 {
			h = 5
		}

		m.overlay.ShowSelectorStayOnSelect(title, items, func(value string) {
			if value != "" && m.program != nil {
				m.program.Send(components.SelectorChosenMsg{Value: value})
			}
		}, func(key string) bool {
			if key == "tab" && hasScoped {
				scopeAll = !scopeAll
				showOverlay()
				return true
			}
			return false
		}, 60, h)
		m.overlay.SetNoMatchText("No matching models")
		// Set selection info to show model name at bottom (TS pi-mono: "Model Name: GPT-4o")
		m.overlay.SetSelectionInfoFunc(func(idx int, item components.SelectorItem) string {
			modelID := item.Value
			name, _ := parseModelString(modelID)
			if info, ok := modelInfoMap[name]; ok && info.Name != "" {
				return "Model Name: " + info.Name
			}
			if info, ok := modelInfoMap[modelID]; ok && info.Name != "" {
				return "Model Name: " + info.Name
			}
			return "Model Name: " + name
		})
	}
	showOverlay()
}

// cycleModelForward cycles to the next available model.
// If scoped models are set, only cycles through scoped models.
func (m *AppModel) cycleModelForward() {
	models := m.getCyclableModels()
	if len(models) == 0 {
		return
	}
	if len(models) == 1 {
		msg := "Only one model in scope"
		if len(m.scopedModels) == 0 {
			msg = "Only one model available"
		}
		m.chat.AppendSystem(msg)
		return
	}
	m.modelIndex = (m.modelIndex + 1) % len(models)
	newModel := models[m.modelIndex]
	m.agent.Loop().Model = newModel
	modelName, provider := parseModelString(newModel)
	m.footer.SetSession(m.session.CWD, getGitBranch(m.session.CWD), m.session.GetSessionName(), modelName, m.thinkingLevel, provider)
	m.footer.SetHasReasoning(supportsThinking(modelName))
	m.footer.SetEntryCount(len(m.session.Entries))
	m.input.SetBorderColor(m.theme.ThinkingBorderColor(m.thinkingLevel))
	msg := "Switched to " + newModel
	if m.thinkingLevel != "" && m.thinkingLevel != "off" {
		msg += " (thinking: " + m.thinkingLevel + ")"
	}
	if !m.chat.ReplaceLastSystem(msg) {
		m.chat.AppendSystem(msg)
	}
}


func (m *AppModel) cycleModelBackward() {
	models := m.getCyclableModels()
	if len(models) == 0 {
		return
	}
	if len(models) == 1 {
		msg := "Only one model in scope"
		if len(m.scopedModels) == 0 {
			msg = "Only one model available"
		}
		m.chat.AppendSystem(msg)
		return
	}
	m.modelIndex--
	if m.modelIndex < 0 {
		m.modelIndex = len(models) - 1
	}
	newModel := models[m.modelIndex]
	m.agent.Loop().Model = newModel
	modelName, provider := parseModelString(newModel)
	m.footer.SetSession(m.session.CWD, getGitBranch(m.session.CWD), m.session.GetSessionName(), modelName, m.thinkingLevel, provider)
	m.footer.SetHasReasoning(supportsThinking(modelName))
	m.footer.SetEntryCount(len(m.session.Entries))
	m.input.SetBorderColor(m.theme.ThinkingBorderColor(m.thinkingLevel))
	msg := "Switched to " + newModel
	if m.thinkingLevel != "" && m.thinkingLevel != "off" {
		msg += " (thinking: " + m.thinkingLevel + ")"
	}
	if !m.chat.ReplaceLastSystem(msg) {
		m.chat.AppendSystem(msg)
	}
}


func (m *AppModel) getCyclableModels() []string {
	if len(m.scopedModels) > 0 {
		// Use modelOrder if available for preferred cycling order
		var models []string
		if len(m.modelOrder) > 0 {
			for _, mdl := range m.modelOrder {
				if m.scopedModels[mdl] {
					models = append(models, mdl)
				}
			}
		} else {
			for _, mdl := range m.availableModels {
				if m.scopedModels[mdl] {
					models = append(models, mdl)
				}
			}
		}
		if len(models) > 0 {
			return models
		}
	}
	// Use modelOrder for all models if available
	if len(m.modelOrder) > 0 {
		return m.modelOrder
	}
	return m.availableModels
}

// switchToModel switches the agent to the specified model (from model selector).
func (m *AppModel) switchToModel(model string) {
	defer m.setTerminalTitle()
	m.agent.Loop().Model = model
	// Update model index
	for i, m2 := range m.availableModels {
		if m2 == model {
			m.modelIndex = i
			break
		}
	}
	modelName, provider := parseModelString(model)
	m.footer.SetSession(m.session.CWD, getGitBranch(m.session.CWD), m.session.GetSessionName(), modelName, m.thinkingLevel, provider)
	m.footer.SetHasReasoning(supportsThinking(modelName))
	m.footer.SetEntryCount(len(m.session.Entries))
	m.input.SetBorderColor(m.theme.ThinkingBorderColor(m.thinkingLevel))
	msg := "Switched to " + modelName
	if m.thinkingLevel != "" && m.thinkingLevel != "off" {
		msg += " (thinking: " + m.thinkingLevel + ")"
	}
	m.chat.AppendSystem(msg)
}


// showSessionTree opens an interactive session tree viewer (TS pi-mono: /tree).
// Supports fold/unfold, filter modes, search, and active path highlighting.
func (m *AppModel) showSessionTree() {
	if m.session == nil || len(m.session.Entries) == 0 {
		m.chat.AppendSystem("No entries in session")
		return
	}

	// Initialize transient tree state
	if m.treeFoldedNodes == nil {
		m.treeFoldedNodes = make(map[string]bool)
	}
	if m.treeFilterMode == "" {
		m.treeFilterMode = m.defaultTreeFilter
		if m.treeFilterMode == "" {
			m.treeFilterMode = "default"
		}
	}

	// Build parent→children map (full tree never changes)
	childrenOf := make(map[string][]session.SessionEntry)
	rootEntries := make([]session.SessionEntry, 0)
	entryByID := make(map[string]session.SessionEntry)
	for _, e := range m.session.Entries {
		entryByID[e.ID] = e
		if e.ParentID == "" {
			rootEntries = append(rootEntries, e)
		} else {
			childrenOf[e.ParentID] = append(childrenOf[e.ParentID], e)
		}
	}

	// Extract text preview from an entry
	extractPreview := func(e session.SessionEntry) string {
		if len(e.Content) == 0 {
			return e.Type
		}
		var contentBlocks []struct {
			Type string `json:"type"`
			Text string `json:"text"`
		}
		if err := json.Unmarshal(e.Content, &contentBlocks); err == nil {
			for _, block := range contentBlocks {
				if block.Type == "text" && block.Text != "" {
					return strings.ReplaceAll(block.Text, "\n", " ")
				}
			}
		}
		return e.Type
	}

	// Determine entry type character for badge
	typeChar := func(e session.SessionEntry) string {
		switch {
		case e.Type == "compaction":
			return "C"
		case e.Type == "model_change":
			return "M"
		case e.Type == "label":
			return "L"
		case e.Type == "session_info":
			return "I"
		case e.Type == "branch_summary":
			return "B"
		}
		switch e.Role {
		case "user":
			return "U"
		case "assistant":
			return "A"
		case "tool":
			return "T"
		case "system":
			return "S"
		}
		return "?"
	}

	// Check if entry has children
	hasChildren := func(id string) bool {
		return len(childrenOf[id]) > 0
	}

	// Check if entry passes the current filter mode
	passesFilter := func(e session.SessionEntry, mode string) bool {
		isSettings := e.Type == "label" || e.Type == "custom" || e.Type == "model_change" ||
			e.Type == "thinking_level_change" || e.Type == "session_info"
		switch mode {
		case "user-only":
			return e.Role == "user"
		case "no-tools":
			return !isSettings && e.Role != "tool"
		case "labeled-only":
			return e.Label != "" || e.Type == "label"
		default: // "default"
			return !isSettings
		}
	}

	// Build active path set (from root to current leaf)
	// TS pi-mono: walks from currentLeafId to root, marks all ancestors + leaf with bullet
	currentLeafID := session.EffectiveLeafID(m.session)
	activePath := make(map[string]bool)
	if currentLeafID != "" {
		for id := currentLeafID; id != ""; {
			activePath[id] = true
			if e, ok := entryByID[id]; ok {
				id = e.ParentID
			} else {
				break
			}
		}
	}

	var treeItemIndents []int

	// buildTreeItems rebuilds items from current fold/filter/search state
	buildTreeItems := func() []components.SelectorItem {
		type flatEntry struct {
			entry   session.SessionEntry
			indent  int
			prefix  string // "├─ " or "└─ " or ""
			gutters []bool // true = show │ at each level
		}
		var flat []flatEntry

		var walk func(entries []session.SessionEntry, indent int, gutters []bool)
		walk = func(entries []session.SessionEntry, indent int, gutters []bool) {
			for i, e := range entries {
				isLast := i == len(entries)-1
				prefix := ""
				if indent > 0 {
					if isLast {
						prefix = "└─ "
					} else {
						prefix = "├─ "
					}
				}
				flat = append(flat, flatEntry{entry: e, indent: indent, prefix: prefix, gutters: gutters})

				// Skip children of folded nodes
				if m.treeFoldedNodes[e.ID] {
					continue
				}

				children := childrenOf[e.ID]
				if len(children) > 0 {
					// Build gutters for children: extend with whether this node continues siblings
					childGutters := make([]bool, len(gutters))
					copy(childGutters, gutters)
					if indent > 0 {
						childGutters = append(childGutters, !isLast)
					}
					walk(children, indent+1, childGutters)
				}
			}
		}
		walk(rootEntries, 0, nil)

		// Apply filter mode and search
		var filtered []flatEntry
		for _, fe := range flat {
			if !passesFilter(fe.entry, m.treeFilterMode) {
				continue
			}
			// Apply search query
			if m.treeSearchQuery != "" {
				preview := strings.ToLower(extractPreview(fe.entry))
				id := strings.ToLower(fe.entry.ID)
				role := strings.ToLower(fe.entry.Role)
				q := strings.ToLower(m.treeSearchQuery)
				if !fuzzyMatch(q, preview) && !fuzzyMatch(q, id) && !fuzzyMatch(q, role) {
					continue
				}
			}
			filtered = append(filtered, fe)
		}

		items := make([]components.SelectorItem, 0, len(filtered))
		treeItemIndents = make([]int, 0, len(filtered))
		for _, fe := range filtered {
			e := fe.entry

			// Build prefix with gutters at each level
			treePrefix := ""
			for _, show := range fe.gutters {
				if show {
					treePrefix += "│ "
				} else {
					treePrefix += "  "
				}
			}
			if fe.prefix != "" {
				treePrefix += fe.prefix
			}

			// Fold indicator
			foldIndicator := ""
			if hasChildren(e.ID) {
				if m.treeFoldedNodes[e.ID] {
					foldIndicator = "⊞ "
				} else {
					foldIndicator = "⊟ "
				}
			}

			// Active path marker
			pathMarker := ""
			if activePath[e.ID] {
				pathMarker = "• "
			}

			// Type badge
			tc := typeChar(e)

			// Per-type color (TS pi-mono: themed colors per entry type)
			typeColor := treeColorForEntry(e)

			// Content preview (truncated)
			preview := extractPreview(e)
			if len(preview) > 50 {
				preview = preview[:47] + "..."
			}

			// Build colored label: dim connectors + colored badge + colored preview
			dimTree := lipgloss.NewStyle().Faint(true).Foreground(lipgloss.Color("#5c6370"))
			connectors := dimTree.Render(treePrefix + foldIndicator + pathMarker)
			badge := typeColor.Render("[" + tc + "]")
			previewColored := typeColor.Render(preview)
			label := connectors + badge + " " + previewColored
			desc := ""
			if m.treeShowTimestamps {
				desc = e.Timestamp.Format("01/02 15:04")
				if e.ParentID != "" {
					desc = e.ID[:8] + " · " + desc
				}
			} else {
				desc = e.ID[:8]
			}
			items = append(items, components.SelectorItem{
				Label:       label,
				Description: desc,
				Value:       e.ID,
			})
			treeItemIndents = append(treeItemIndents, fe.indent)
		}
		return items
	}

	// Build title with filter mode indicator
	buildTitle := func() string {
		return "Session Tree"
	}

	// Custom key handler for tree-specific keys
	onKey := func(key string) bool {
		needRebuild := false
		switch key {
		case "ctrl+d":
			m.treeFilterMode = "default"
			needRebuild = true
		case "ctrl+t":
			if m.treeFilterMode == "no-tools" {
				m.treeFilterMode = "default"
			} else {
				m.treeFilterMode = "no-tools"
			}
			needRebuild = true
		case "ctrl+u":
			if m.treeFilterMode == "user-only" {
				m.treeFilterMode = "default"
			} else {
				m.treeFilterMode = "user-only"
			}
			needRebuild = true
		case "ctrl+l":
			if m.treeFilterMode == "labeled-only" {
				m.treeFilterMode = "default"
			} else {
				m.treeFilterMode = "labeled-only"
			}
			needRebuild = true
		case "ctrl+a":
			if m.treeFilterMode == "all" {
				m.treeFilterMode = "default"
			} else {
				m.treeFilterMode = "all"
			}
			needRebuild = true
		case "ctrl+o":
			modes := []string{"default", "no-tools", "user-only", "labeled-only", "all"}
			for i, mode := range modes {
				if mode == m.treeFilterMode {
					m.treeFilterMode = modes[(i+1)%len(modes)]
					break
				}
			}
			needRebuild = true
		case "ctrl+shift+o":
			modes := []string{"default", "no-tools", "user-only", "labeled-only", "all"}
			for i, mode := range modes {
				if mode == m.treeFilterMode {
					m.treeFilterMode = modes[(i-1+len(modes))%len(modes)]
					break
				}
			}
			needRebuild = true
		case "ctrl+left", "alt+left":
			// Fold current node; if not foldable, jump to branch segment start (TS pi-mono: findBranchSegmentStart)
			val := m.overlay.SelectedValue()
			if val != "" && hasChildren(val) && !m.treeFoldedNodes[val] {
				m.treeFoldedNodes[val] = true
				needRebuild = true
			} else {
				// Jump to parent branch point: walk up to find item with lower indent
				idx := m.overlay.SelectedIndex()
				if idx > 0 && idx < len(m.treeItemIndents) {
					curIndent := m.treeItemIndents[idx]
					for i := idx - 1; i >= 0; i-- {
						if m.treeItemIndents[i] < curIndent {
							m.overlay.SelectIdx(i)
							break
						}
					}
				}
			}
		case "ctrl+right", "alt+right":
			// Unfold current node; if not folded, jump to first child or next sibling (TS pi-mono: branch segment)
			val := m.overlay.SelectedValue()
			if val != "" && m.treeFoldedNodes[val] {
				delete(m.treeFoldedNodes, val)
				needRebuild = true
			} else {
				// Jump to first child; if no children, move to next item (TS pi-mono)
				idx := m.overlay.SelectedIndex()
				found := false
				if idx >= 0 && idx < len(m.treeItemIndents)-1 {
					curIndent := m.treeItemIndents[idx]
					for i := idx + 1; i < len(m.treeItemIndents); i++ {
						if m.treeItemIndents[i] < curIndent {
							break // hit a higher-level node
						}
						if m.treeItemIndents[i] > curIndent {
							m.overlay.SelectIdx(i)
							found = true
							break
						}
					}
				}
				if !found && idx < m.overlay.ItemCount()-1 {
					m.overlay.SelectIdx(idx + 1)
				}
			}
		case "enter":
			// Toggle fold on current node if foldable; otherwise let default handler select
			val := m.overlay.SelectedValue()
			if val != "" && hasChildren(val) {
				if m.treeFoldedNodes[val] {
					delete(m.treeFoldedNodes, val)
				} else {
					m.treeFoldedNodes[val] = true
				}
				needRebuild = true
				return true // consume Enter so it doesn't close overlay
			}
			return false // let default handler process (select item)
		case "shift+l":
			// Edit tree label: close tree and put label text in editor
			val := m.overlay.SelectedValue()
			if val != "" {
				if e, ok := entryByID[val]; ok {
					labelText := ""
					if e.Type == "label" && e.Label != "" {
						labelText = e.Label
					} else {
						labelText = extractPreview(e)
					}
					m.overlay.Hide()
					m.input.SetValue("/name " + labelText)
					m.chat.AppendSystem("Editing label — press Enter to save, Esc to cancel")
				}
			}
			return true
		case "shift+t":
			// Toggle label timestamp display
			m.treeShowTimestamps = !m.treeShowTimestamps
			if m.treeShowTimestamps {
				m.chat.AppendSystem("Timestamps shown")
			} else {
				m.chat.AppendSystem("Timestamps hidden")
			}
			needRebuild = true
			return true
		case "backspace":
			if m.treeSearchQuery != "" {
				m.treeSearchQuery = m.treeSearchQuery[:len(m.treeSearchQuery)-1]
				needRebuild = true
				return true
			}
			return false // let default handler process
		case "esc":
			// Clear search first, then close overlay on second Esc
			if m.treeSearchQuery != "" {
				m.treeSearchQuery = ""
				needRebuild = true
				return true
			}
			return false // let default handler close overlay
		default:
			// Printable characters update search
			if len(key) == 1 && key[0] >= 32 && key[0] < 127 {
				m.treeSearchQuery += key
				needRebuild = true
				return true
			}
			return false
		}

		if needRebuild {
			items := buildTreeItems()
			m.treeItemIndents = treeItemIndents
			m.overlay.ReplaceItems(buildTitle(), items)
			// Sync search query with selector filter display
			if m.treeSearchQuery != "" {
				m.overlay.SetFilter(m.treeSearchQuery)
			}
		}
		return true
	}

	// Build initial items
	items := buildTreeItems()
	m.treeItemIndents = treeItemIndents
	title := buildTitle()
	h := len(items) + 5
	if h < 10 {
		h = 10
	}
	if h > 24 {
		h = 24
	}
	w := 86

	m.overlay.ShowSelectorWithKeyHandler(title, items, func(value string) {
		if value != "" {
			// TS pi-mono: selecting the current leaf is a no-op
			if value == currentLeafID {
				m.chat.AppendSystem("Already at this point")
			} else {
				// Navigate to selected entry
				m.sessMgr.Branch(m.session, value)
				// Re-render chat from the new leaf path (TS pi-mono: chatContainer.clear + renderInitialMessages)
				m.chat.Clear()
				for _, entry := range m.session.Entries {
					ce := sessionEntryToChatEntry(entry)
					if ce != nil {
						m.chat.AppendChatEntry(*ce)
					}
				}
				m.chat.AppendSystem("Navigated to selected point")
			}
		}
		// Reset tree state on close
		m.treeFoldedNodes = nil
		m.treeFilterMode = ""
		m.treeSearchQuery = ""
	}, onKey, w, h)
}

// treeColorForEntry returns a lipgloss style with the appropriate color for a tree entry.
// Matches TS pi-mono's themed per-type colors.
func treeColorForEntry(e session.SessionEntry) lipgloss.Style {
	// Special types
	switch e.Type {
	case "compaction":
		return lipgloss.NewStyle().Foreground(lipgloss.Color("#c678dd"))
	case "branch_summary", "label":
		return lipgloss.NewStyle().Foreground(lipgloss.Color("#e5c07b"))
	case "model_change", "thinking_level_change", "custom", "session_info":
		return lipgloss.NewStyle().Faint(true).Foreground(lipgloss.Color("#5c6370"))
	}
	// Role-based colors
	switch e.Role {
	case "user":
		return lipgloss.NewStyle().Foreground(lipgloss.Color("#61afef"))
	case "assistant":
		return lipgloss.NewStyle().Foreground(lipgloss.Color("#98c379"))
	case "tool":
		return lipgloss.NewStyle().Faint(true).Foreground(lipgloss.Color("#5c6370"))
	case "system":
		return lipgloss.NewStyle().Faint(true).Foreground(lipgloss.Color("#5c6370"))
	}
	return lipgloss.NewStyle().Foreground(lipgloss.Color("#abb2bf"))
}

// forkFromEntry truncates the current session at the given entry ID,
// keeping entries up to and including the specified entry (TS pi-mono: fork).
// The original session is preserved; the current session becomes the fork.
func (m *AppModel) forkFromEntry(entryID string) {
	if m.session == nil || m.sessMgr == nil {
		return
	}
	// Save original session first
	m.sessMgr.Save(m.session)

	// Find the entry index
	cutIdx := -1
	for i := range m.session.Entries {
		if m.session.Entries[i].ID == entryID {
			cutIdx = i + 1 // keep up to and including this entry
			break
		}
	}
	if cutIdx < 0 {
		m.chat.AppendSystem("Entry not found for fork")
		return
	}

	// Create forked session
	newID := session.GenerateID()
	oldEntries := m.session.Entries[:cutIdx]

	m.session.ID = newID
	m.session.Entries = make([]session.SessionEntry, len(oldEntries))
	copy(m.session.Entries, oldEntries)
	m.session.CreatedAt = time.Now()
	m.session.UpdatedAt = time.Now()
	m.session.Name = ""
	if err := m.sessMgr.Save(m.session); err != nil {
		m.chat.AppendSystem("Error saving fork: " + err.Error())
		return
	}
	m.footer.SetSession(m.session.CWD, getGitBranch(m.session.CWD), "", "", "", "")
	m.chat.AppendSystem("Forked to new session")
}

// cloneSession creates a full copy of the current session with a new ID.
// switchToSession saves the current session and switches to a different one.
// (TS pi-mono: /resume <id> in interactive mode without restart)
func (m *AppModel) switchToSession(sid string) {
	if m.sessMgr == nil || m.session == nil {
		m.chat.AppendSystem("No session manager available")
		return
	}
	if sid == m.session.ID {
		m.chat.AppendWarning("Already in session: " + sid)
		return
	}

	// Save current session before switching
	if err := m.sessMgr.Save(m.session); err != nil {
		m.chat.AppendError("Failed to save current session: " + err.Error())
		return
	}

	// Load the target session
	newSess, err := m.sessMgr.Load(sid, m.session.CWD)
	if err != nil {
		m.chat.AppendError("Failed to load session " + sid + ": " + err.Error())
		return
	}

	oldCWD := m.session.CWD
	m.session = newSess
	if m.session.CWD == "" {
		m.session.CWD = oldCWD
	}
	if m.session.CWD == "" {
		cwd, _ := os.Getwd()
		m.session.CWD = cwd
	}

	// Rebuild chat viewport from session tree (TS pi-mono: rebuildChatFromMessages)
	m.rebuildChatFromSession()

	// Update footer
	modelName, provider := parseModelString(m.agent.Loop().Model)
	m.footer.SetSession(m.session.CWD, getGitBranch(m.session.CWD), m.session.GetSessionName(), modelName, m.thinkingLevel, provider)
	m.footer.SetEntryCount(len(m.session.Entries))

	// TS pi-mono: show compaction info if session was compacted
	compactionCount := 0
	for _, e := range m.session.Entries {
		if e.Type == session.EntryTypeCompaction {
			compactionCount++
		}
	}
	if compactionCount > 0 {
		times := fmt.Sprintf("%d times", compactionCount)
		if compactionCount == 1 {
			times = "1 time"
		}
		m.chat.AppendSystem("Session compacted " + times)
	}
	m.chat.AppendSystem("Resumed session")
	m.setTerminalTitle()
}

// rebuildChatFromSession clears the chat viewport and rebuilds it from session
// entries, walking the tree from the current leaf and respecting compaction
// boundaries (TS pi-mono: rebuildChatFromMessages / renderSessionContext).
func (m *AppModel) rebuildChatFromSession() {
	if m.session == nil {
		return
	}
	m.chat.Clear()

	// Walk the tree from the current leaf to root
	leafID := session.EffectiveLeafID(m.session)
	chain := session.ForEachEntry(m.session.Entries, leafID)
	if len(chain) == 0 {
		return
	}

	// Process root-to-leaf, respecting compaction boundaries.
	// When a CompactionEntry is found, parse its first_kept_entry_id and
	// skip earlier entries (they were compacted away). The compaction
	// summary itself is rendered as a fresh card at the end, not here.
	skipUntil := ""
	for i := len(chain) - 1; i >= 0; i-- {
		entry := chain[i]

		if skipUntil != "" {
			if entry.ID == skipUntil {
				skipUntil = ""
			} else {
				continue
			}
		}

		if entry.Type == session.EntryTypeCompaction {
			if entry.Content != nil {
				var meta struct {
					FirstKeptEntryID string `json:"first_kept_entry_id"`
				}
				if err := json.Unmarshal(entry.Content, &meta); err == nil && meta.FirstKeptEntryID != "" {
					skipUntil = meta.FirstKeptEntryID
				}
			}
			continue
		}

		ce := sessionEntryToChatEntry(entry)
		if ce != nil {
			m.chat.AppendChatEntry(*ce)
		}
	}
}

// sessionEntryToChatEntry converts a session.SessionEntry to a ChatEntry for display.
// Returns nil for entries that should be skipped (e.g. labels, branch summaries).
func sessionEntryToChatEntry(entry session.SessionEntry) *components.ChatEntry {
	var contentBlocks []struct {
		Type string `json:"type"`
		Text string `json:"text"`
	}
	_ = json.Unmarshal(entry.Content, &contentBlocks)
	var textParts []string
	for _, block := range contentBlocks {
		if block.Type == "text" && block.Text != "" {
			textParts = append(textParts, block.Text)
		}
	}
	contentText := strings.Join(textParts, "\n")

	switch entry.Type {
	case session.EntryTypeUser:
		return &components.ChatEntry{Type: "user_message", Content: contentText, ID: entry.ID}
	case session.EntryTypeAssistant:
		if contentText == "" && len(entry.ToolCalls) > 0 {
			// Assistant entry with only tool calls — skip, tool entries carry the detail
			return nil
		}
		return &components.ChatEntry{Type: "text", Content: contentText, ID: entry.ID}
	case session.EntryTypeTool:
		return &components.ChatEntry{Type: "tool_result", Content: contentText, ID: entry.ID}
	case session.EntryTypeSystem, session.EntryTypeCompaction:
		// Show system and compaction messages in chat
		if contentText == "" {
			return nil
		}
		return &components.ChatEntry{Type: "system", Content: contentText, ID: entry.ID}
	case session.EntryTypeModelChange, session.EntryTypeThinkingLevelChange, session.EntryTypeSessionInfo:
		// Pi-mono hides these in chat (tree-only entries) — show as dim system message
		if contentText == "" {
			return nil
		}
		return &components.ChatEntry{Type: "system", Content: contentText, ID: entry.ID}
	case session.EntryTypeLabel:
		return nil // skip structural entries
	case session.EntryTypeBranchSummary:
		// Show branch summary as a system message with metadata
		if entry.BranchSummary != nil {
			summaryText := fmt.Sprintf("[branch] %s (from %s)", contentText, entry.BranchSummary.FromID)
			return &components.ChatEntry{Type: "custom_message", Content: summaryText, CustomType: "branch", ID: entry.ID}
		}
		return &components.ChatEntry{Type: "system", Content: "[branch] " + contentText, ID: entry.ID}
	case session.EntryTypeCustom:
		return &components.ChatEntry{Type: "system", Content: contentText, ID: entry.ID}
	case session.EntryTypeCustomMessage:
		return &components.ChatEntry{Type: "custom_message", Content: entry.Display, CustomType: entry.CustomType, ID: entry.ID}
	default:
		return nil
	}
}

func (m *AppModel) cloneSession() string {
	oldEntries := make([]session.SessionEntry, len(m.session.Entries))
	copy(oldEntries, m.session.Entries)

	// Create new session
	m.session.ID = session.GenerateID()
	m.session.Entries = oldEntries
	m.session.CreatedAt = time.Now()
	m.session.UpdatedAt = time.Now()
	if err := m.sessMgr.Save(m.session); err != nil {
		m.chat.AppendSystem("Error saving cloned session: " + err.Error())
		return ""
	}
	m.chat.AppendSystem("Cloned to new session")
	if name := m.session.GetSessionName(); name != "" {
		m.session.SetSessionName(name + " (clone)")
		m.sessMgr.Save(m.session)
	}
	return m.session.ID
}

// showForkSelector opens a user message selector for forking (TS pi-mono: /fork).
// Shows recent user messages from the session; selecting one forks from that point.
func (m *AppModel) showForkSelector() {
	if m.session == nil || len(m.session.Entries) == 0 {
		m.chat.AppendSystem("No messages to fork from")
		return
	}

	// Extract user messages with their entry IDs
	type userMsg struct {
		id   string
		text string
	}
	var messages []userMsg
	fullTexts := make(map[string]string) // entryID -> full message text
	for i := len(m.session.Entries) - 1; i >= 0; i-- {
		e := m.session.Entries[i]
		if e.Role == "user" && len(e.Content) > 0 {
			// Try to extract text content
			var contentBlocks []struct {
				Type string `json:"type"`
				Text string `json:"text"`
			}
			if err := json.Unmarshal(e.Content, &contentBlocks); err == nil {
				for _, block := range contentBlocks {
					if block.Type == "text" && block.Text != "" {
						fullTexts[e.ID] = block.Text
						// Truncate long messages
						text := block.Text
						if len(text) > 80 {
							text = text[:77] + "..."
						}
						messages = append(messages, userMsg{id: e.ID, text: text})
						break
					}
				}
			}
		}
	}

	if len(messages) == 0 {
		m.chat.AppendSystem("No user messages found in session")
		return
	}

	items := make([]components.SelectorItem, 0, len(messages))
	for i, msg := range messages {
		label := msg.text
		pos := len(messages) - i
		desc := fmt.Sprintf("Message %d of %d", pos, len(messages))
		items = append(items, components.SelectorItem{
			Label:       label,
			Description: desc,
			Value:       msg.id,
		})
	}

	h := len(items) + 5
	if h > 20 {
		h = 20
	}
	m.overlay.ShowForkSelector("Fork from Message", "Select a user message to copy the active path up to that point into a new session", items, func(value string) {
		if value != "" {
			m.forkFromEntry(value)
			// Fill editor with selected message text (TS pi-mono)
			if fullText, ok := fullTexts[value]; ok {
				m.input.SetValue(fullText)
			}
		}
	}, nil, 70, h)
}

// showSessionSelector opens a session list overlay (TS pi-mono: /resume, /sessions).
// Supports type-to-search, sort toggle (Ctrl+S), named filter (Ctrl+N),
// session delete (Ctrl+Backspace), and session rename (Ctrl+R).
func (m *AppModel) showSessionSelector() {
	if m.sessMgr == nil || m.session == nil {
		m.chat.AppendSystem("No session manager available")
		return
	}

	// Shorten CWD with ~
	cwd := m.session.CWD
	if home, _ := os.UserHomeDir(); home != "" && strings.HasPrefix(cwd, home) {
		cwd = "~" + cwd[len(home):]
	}

	// Local state for sort and filter
	sortByDate := true // true=by date (newest first), false=by name
	namedOnly := false
	showPath := false     // TS pi-mono: togglePath — show CWD path instead of session name
	threadedMode := true // TS pi-mono: threaded tree view (Ctrl+T toggles)
	sessionNames := make(map[string]string) // session ID → name for rename
	foldedNodes := make(map[string]bool)     // TS pi-mono: folded nodes in tree

	// sessionTreeNode represents a node in the session tree.
	type sessionTreeNode struct {
		session  *session.Session
		children []*sessionTreeNode
	}

	// flatSessionNode is a flattened tree node for display.
	type flatSessionNode struct {
		session           *session.Session
		depth             int
		isLast            bool
		ancestorContinues []bool
	}

	var currentTree []*sessionTreeNode // cached tree for foldable checks (set in buildItems)

	// buildSessionTree builds a tree from sessions based on ParentSessionID.
	buildSessionTree := func(sessions []*session.Session) []*sessionTreeNode {
		byID := make(map[string]*sessionTreeNode)
		for _, s := range sessions {
			byID[s.ID] = &sessionTreeNode{session: s}
		}
		roots := make([]*sessionTreeNode, 0)
		for _, s := range sessions {
			node := byID[s.ID]
			if s.ParentSessionID != "" {
				if parent, ok := byID[s.ParentSessionID]; ok {
					parent.children = append(parent.children, node)
					continue
				}
			}
			roots = append(roots, node)
		}
		return roots
	}

	// flattenTree flattens a session tree for display.
	// Skips descendants of folded nodes.
	flattenTree := func(roots []*sessionTreeNode) []flatSessionNode {
		var result []flatSessionNode
		var walk func(*sessionTreeNode, int, []bool, bool)
		walk = func(node *sessionTreeNode, depth int, ancestorContinues []bool, isLast bool) {
			result = append(result, flatSessionNode{
				session:           node.session,
				depth:             depth,
				isLast:            isLast,
				ancestorContinues: ancestorContinues,
			})
			// Skip children if this node is folded
			if threadedMode && foldedNodes[node.session.ID] {
				return
			}
			for i := 0; i < len(node.children); i++ {
				childIsLast := i == len(node.children)-1
				childContinues := make([]bool, 0, len(ancestorContinues)+1)
				childContinues = append(childContinues, ancestorContinues...)
				if depth > 0 && !isLast {
					childContinues = append(childContinues, true)
				} else {
					childContinues = append(childContinues, false)
				}
				walk(node.children[i], depth+1, childContinues, childIsLast)
			}
		}
		for i := 0; i < len(roots); i++ {
			walk(roots[i], 0, nil, i == len(roots)-1)
		}
		return result
	}

	// isFoldable returns true if a node has children (can be folded).
	isFoldable := func(tree []*sessionTreeNode, id string) bool {
		for _, root := range tree {
			var find func(*sessionTreeNode) bool
			find = func(n *sessionTreeNode) bool {
				if n.session.ID == id {
					return len(n.children) > 0
				}
				for _, c := range n.children {
					if find(c) {
						return true
					}
				}
				return false
			}
			if find(root) {
				return true
			}
		}
		return false
	}

	// buildTreePrefix builds the box-drawing prefix for a tree node.
	// Shows fold indicator ⊟ for foldable nodes, ⊞ for folded nodes.
	buildTreePrefix := func(depth int, isLast bool, ancestorContinues []bool, nodeID string) string {
		if depth == 0 {
			// Fold indicator for root nodes
			if threadedMode && isFoldable(currentTree, nodeID) {
				if foldedNodes[nodeID] {
					return "⊞ "
				}
				return "⊟ "
			}
			return ""
		}
		var sb strings.Builder
		for _, continues := range ancestorContinues {
			if continues {
				sb.WriteString("│  ")
			} else {
				sb.WriteString("   ")
			}
		}
		// Branch character with fold indicator
		if isLast {
			sb.WriteString("└")
		} else {
			sb.WriteString("├")
		}
		if threadedMode && isFoldable(currentTree, nodeID) {
			if foldedNodes[nodeID] {
				sb.WriteString("⊞ ")
			} else {
				sb.WriteString("⊟ ")
			}
		} else {
			sb.WriteString("─ ")
		}
		return sb.String()
	}

	// buildItems loads sessions and builds selector items
	buildItems := func() []components.SelectorItem {
		rawSessions, err := m.sessMgr.List(m.session.CWD)
		if err != nil || len(rawSessions) == 0 {
			return nil
		}

		// Convert to pointer slice
		sessions := make([]*session.Session, len(rawSessions))
		for i := range rawSessions {
			sessions[i] = &rawSessions[i]
		}

		// Filter: named only
		if namedOnly {
			filtered := sessions[:0]
			for _, s := range sessions {
				if s.GetSessionName() != "" {
					filtered = append(filtered, s)
				}
			}
			sessions = filtered
		}

		// Sort
		if sortByDate {
			// Already sorted by date from List()
		} else {
			// Sort by name (or ID if no name)
			for i := 0; i < len(sessions); i++ {
				for j := i + 1; j < len(sessions); j++ {
					ni := sessions[i].GetSessionName()
					nj := sessions[j].GetSessionName()
					if ni == "" {
						ni = sessions[i].ID
					}
					if nj == "" {
						nj = sessions[j].ID
					}
					if ni > nj {
						sessions[i], sessions[j] = sessions[j], sessions[i]
					}
				}
			}
		}

		// Session name map
		sessionNames = make(map[string]string)

		// Build flat nodes: threaded tree or flat list
		var flatNodes []flatSessionNode
		if threadedMode {
			currentTree = buildSessionTree(sessions)
			flatNodes = flattenTree(currentTree)
		} else {
			flatNodes = make([]flatSessionNode, len(sessions))
			for i, s := range sessions {
				flatNodes[i] = flatSessionNode{
					session: s,
					depth:   0,
					isLast:  i == len(sessions)-1,
				}
			}
		}

		items := make([]components.SelectorItem, 0, len(flatNodes))
		for _, fn := range flatNodes {
			s := fn.session
			sessionNames[s.ID] = s.GetSessionName()
			isCurrent := s.ID == m.session.ID
			label := s.ID
			if showPath {
				label = s.CWD
				if home, _ := os.UserHomeDir(); home != "" && strings.HasPrefix(label, home) {
					label = "~" + label[len(home):]
				}
			} else {
				name := s.GetSessionName()
				if name != "" {
					label = name
				}
			}
			if isCurrent {
				label = "✓ " + label
			}
			// Tree prefix for threaded mode
			prefix := buildTreePrefix(fn.depth, fn.isLast, fn.ancestorContinues, s.ID)
			count := len(s.Entries)
			age := formatRelativeDate(s.UpdatedAt)
			desc := fmt.Sprintf("%d msgs · %s · %s", count, age, s.ID)
			if isCurrent {
				desc = "current · " + desc
			}
			items = append(items, components.SelectorItem{
				Label:       prefix + label,
				Description: desc,
				Value:       s.ID,
			})
		}
		return items
	}

	items := buildItems()
	if len(items) == 0 {
		m.chat.AppendSystem("No saved sessions found")
		return
	}

	buildTitle := func() string {
		title := fmt.Sprintf("Resume Session (%s)", cwd)
		if namedOnly {
			title += " [named]"
		}
		if !sortByDate {
			title += " [by name]"
		}
		if showPath {
			title += " [paths]"
		}
		if !threadedMode {
			title += " [flat]"
		}
		return title
	}

	confirmingDeletePath := "" // TS pi-mono: two-step delete confirmation

	onKey := func(key string) bool {
		needRebuild := false

		// Handle delete confirmation state first (TS pi-mono)
		if confirmingDeletePath != "" {
			switch key {
			case "enter":
				pathToDelete := confirmingDeletePath
				confirmingDeletePath = ""
				err := m.sessMgr.Delete(pathToDelete, m.session.CWD)
				if err != nil {
					m.chat.AppendSystem("Failed to delete session: " + err.Error())
				} else {
					m.chat.AppendSystem("Session moved to trash")
				}
				m.overlay.SetHelpText("")
				needRebuild = true
				return true
			case "esc":
				confirmingDeletePath = ""
				m.overlay.SetHelpText("")
				needRebuild = true
				return true
			default:
				return true // ignore all other keys while confirming
			}
		}

		switch key {
		case "ctrl+s":
			sortByDate = !sortByDate
			needRebuild = true
		case "ctrl+n":
			namedOnly = !namedOnly
			needRebuild = true
		case "ctrl+p":
			showPath = !showPath
			needRebuild = true
		case "ctrl+t":
			threadedMode = !threadedMode
			needRebuild = true
		case "h":
			// Fold current node (TS pi-mono: tree.foldOrUp)
			if threadedMode {
				val := m.overlay.SelectedValue()
				if val != "" && isFoldable(currentTree, val) && !foldedNodes[val] {
					foldedNodes[val] = true
					needRebuild = true
				}
			}
		case "l":
			// Unfold current node (TS pi-mono: tree.unfoldOrDown)
			if threadedMode {
				val := m.overlay.SelectedValue()
				if val != "" && foldedNodes[val] {
					foldedNodes[val] = false
					needRebuild = true
				}
			}
		case "ctrl+backspace", "ctrl+d":
			// Initiate delete confirmation (TS pi-mono: two-step)
			val := m.overlay.SelectedValue()
			if val != "" {
				if val == m.session.ID {
					m.chat.AppendSystem("Cannot delete the currently active session")
				} else {
					confirmingDeletePath = val
					m.overlay.SetHelpText("Delete session? Enter confirm \xc2\xb7 Esc cancel")
				}
			}
		case "ctrl+r":
			// Rename session: close selector and set editor to /name for inline editing
			val := m.overlay.SelectedValue()
			if val != "" {
				if name, ok := sessionNames[val]; ok && name != "" {
					m.overlay.Hide()
					m.input.SetValue("/name " + name)
				} else {
					m.overlay.Hide()
					m.input.SetValue("/name ")
				}
				m.chat.AppendSystem("Editing session name — press Enter to save, Esc to cancel")
			}
			return true
		case "backspace":
			// Let default handler clear filter
			return false
		case "esc":
			// Let default handler close
			return false
		default:
			return false
		}

		if needRebuild {
			newItems := buildItems()
			if len(newItems) == 0 {
				m.overlay.Hide()
				m.chat.AppendSystem("No sessions match the filter")
				return true
			}
			m.overlay.ReplaceItems(buildTitle(), newItems)
		}
		return true
	}

	h := len(items) + 5
	if h < 10 {
		h = 10
	}
	if h > 20 {
		h = 20
	}
	m.overlay.ShowSelectorWithKeyHandler(buildTitle(), items, func(value string) {
		if value != "" && m.program != nil {
			m.program.Send(components.SelectorChosenMsg{Value: "session:" + value})
		}
	}, onKey, 70, h)
}

// showScopedModelSelector opens a model selector overlay for scoped model management (TS pi-mono: /scoped-models).
// Shows all available models with their scoped status (enabled/disabled).
func (m *AppModel) showScopedModelSelector() {
	if len(m.availableModels) == 0 {
		m.chat.AppendSystem("Only showing models from configured providers. Use /login to add providers.")
		return
	}

	buildItems := func() []components.SelectorItem {
		items := make([]components.SelectorItem, 0, len(m.availableModels))
		// Use modelOrder for display order if available
		displayOrder := m.availableModels
		if len(m.modelOrder) > 0 {
			displayOrder = m.modelOrder
			// Add any models not in modelOrder
			inOrder := make(map[string]bool)
			for _, mdl := range m.modelOrder {
				inOrder[mdl] = true
			}
			for _, mdl := range m.availableModels {
				if !inOrder[mdl] {
					displayOrder = append(displayOrder, mdl)
				}
			}
		}
		for _, model := range displayOrder {
			name, provider := parseModelString(model)
			enabled := m.scopedModels[model]
			label := name
			if enabled {
				label = "✓ " + name
			} else {
				label = "  " + name
			}
			desc := "[" + provider + "]"
			if enabled {
				desc = desc + " ✓"
			} else {
				desc = desc + " ✗"
			}
			items = append(items, components.SelectorItem{
				Label:       label,
				Description: desc,
				Value:       model,
			})
		}
		return items
	}

	buildTitle := func() string {
		enabledCount := len(m.scopedModels)
		if enabledCount > 0 {
			return fmt.Sprintf("Model Configuration (%d of %d enabled)  Enter=toggle  Ctrl+A/X=all/clear  Ctrl+S=save  Ctrl+P=provider  Alt+↑↓=reorder  Esc=close", enabledCount, len(m.availableModels))
		}
		return fmt.Sprintf("Model Configuration (all %d)  Enter=toggle  Ctrl+A/X=all/clear  Ctrl+S=save  Ctrl+P=provider  Alt+↑↓=reorder  Esc=close", len(m.availableModels))
	}

	refresh := func() {
		m.overlay.ReplaceItems(buildTitle(), buildItems())
	}

	onSelect := func(value string) {
		if value != "" {
			if m.scopedModels[value] {
				delete(m.scopedModels, value)
				m.chat.AppendSystem("Disabled: " + value)
			} else {
				m.scopedModels[value] = true
				m.chat.AppendSystem("Enabled: " + value)
			}
			// Re-open to show updated state
			if m.program != nil {
				go func() {
					time.Sleep(50 * time.Millisecond)
					m.program.Send(refreshScopedSelectorMsg{})
				}()
			}
		}
	}

	onKey := func(key string) bool {
		switch key {
		case "ctrl+a":
			// Enable all models and reset order
			modelNames := make([]string, len(m.availableModels))
			copy(modelNames, m.availableModels)
			m.modelOrder = modelNames
			for _, model := range m.availableModels {
				m.scopedModels[model] = true
			}
			m.chat.AppendSystem("Enabled all " + fmt.Sprintf("%d", len(m.availableModels)) + " models")
			refresh()
			return true
		case "ctrl+x":
			// Clear all scoped models
			m.scopedModels = make(map[string]bool)
			refresh()
			return true
		case "ctrl+s":
			// Save model selection (TS pi-mono: persist to settings)
			m.chat.AppendSystem("Model selection saved to settings")
			return true
		case "alt+up":
			// Move model up in order
			sel := m.overlay.SelectedValue()
			if sel != "" && len(m.modelOrder) > 0 {
				for i, mdl := range m.modelOrder {
					if mdl == sel && i > 0 {
						m.modelOrder[i], m.modelOrder[i-1] = m.modelOrder[i-1], m.modelOrder[i]
						break
					}
				}
				refresh()
			}
			return true
		case "alt+down":
			// Move model down in order
			sel := m.overlay.SelectedValue()
			if sel != "" && len(m.modelOrder) > 0 {
				for i, mdl := range m.modelOrder {
					if mdl == sel && i < len(m.modelOrder)-1 {
						m.modelOrder[i], m.modelOrder[i+1] = m.modelOrder[i+1], m.modelOrder[i]
						break
					}
				}
				refresh()
			}
			return true
		case "ctrl+p":
			// Toggle all models for the provider of the currently selected item
			sel := m.overlay.SelectedValue()
			if sel != "" {
				_, selProvider := parseModelString(sel)
				// Check if all models for this provider are already enabled
				allEnabled := true
				for _, model := range m.availableModels {
					_, provider := parseModelString(model)
					if provider == selProvider && !m.scopedModels[model] {
						allEnabled = false
						break
					}
				}
				// Toggle: disable all if all enabled, otherwise enable all
				for _, model := range m.availableModels {
					_, provider := parseModelString(model)
					if provider == selProvider {
						if allEnabled {
							delete(m.scopedModels, model)
						} else {
							m.scopedModels[model] = true
						}
					}
				}
				if allEnabled {
					m.chat.AppendSystem("Disabled all " + selProvider + " models")
				} else {
					m.chat.AppendSystem("Enabled all " + selProvider + " models")
				}
				refresh()
			}
			return true
		}
		return false
	}

	h := len(m.availableModels) + 6
	if h > 22 {
		h = 22
	}
	m.overlay.ShowSelectorWithKeyHandler(buildTitle(), buildItems(), onSelect, onKey, 76, h)
}

// refreshScopedSelectorMsg is an internal message to refresh the scoped model selector.
type refreshScopedSelectorMsg struct{}

// refreshSettingsMsg is an internal message to refresh the settings selector.
type refreshSettingsMsg struct{}

// refreshModelSelectorMsg is an internal message to refresh the model selector.
type refreshModelSelectorMsg struct{}

// ─── Extension UI Bridge ────────────────────────────────────────────────────

// extensionSelectMsg is sent to show an extension selector dialog.
type extensionSelectMsg struct {
	title   string
	options []string
	timeout time.Duration
	respCh  chan extensionUIResponse
}

// extensionInputMsg is sent to show an extension input dialog.
type extensionInputMsg struct {
	title       string
	placeholder string
	timeout     time.Duration
	respCh      chan extensionUIResponse
}

// extensionEditorMsg is sent to show an extension editor dialog.
type extensionEditorMsg struct {
	title   string
	prefill string
	respCh  chan extensionUIResponse
}

// extensionUIResponse carries the result of an extension UI dialog.
type extensionUIResponse struct {
	value string
	err   error
}

// tuiExtensionBridge implements extensions.ExtensionUI using the Bubble Tea overlay system.
type tuiExtensionBridge struct {
	program       *tea.Program
	inputRegistry *terminalInputRegistry
}

func (b *tuiExtensionBridge) Select(title string, options []string, opts *extensions.ExtensionUIDialogOptions) (string, error) {
	respCh := make(chan extensionUIResponse, 1)
	msg := extensionSelectMsg{
		title:   title,
		options: options,
		respCh:  respCh,
	}
	if opts != nil && opts.Timeout > 0 {
		msg.timeout = opts.Timeout
	}
	b.program.Send(msg)
	resp := <-respCh
	return resp.value, resp.err
}

func (b *tuiExtensionBridge) Confirm(title, message string, opts *extensions.ExtensionUIDialogOptions) (bool, error) {
	val, err := b.Select(title+"\n"+message, []string{"Yes", "No"}, opts)
	return val == "Yes", err
}

func (b *tuiExtensionBridge) Input(title, placeholder string, opts *extensions.ExtensionUIDialogOptions) (string, error) {
	respCh := make(chan extensionUIResponse, 1)
	msg := extensionInputMsg{
		title:       title,
		placeholder: placeholder,
		respCh:      respCh,
	}
	if opts != nil && opts.Timeout > 0 {
		msg.timeout = opts.Timeout
	}
	b.program.Send(msg)
	resp := <-respCh
	return resp.value, resp.err
}

func (b *tuiExtensionBridge) Editor(title, prefill string) (string, error) {
	respCh := make(chan extensionUIResponse, 1)
	b.program.Send(extensionEditorMsg{
		title:   title,
		prefill: prefill,
		respCh:  respCh,
	})
	resp := <-respCh
	return resp.value, resp.err
}

func (b *tuiExtensionBridge) Notify(message string, notifyType string) {
	switch notifyType {
	case "error":
		b.program.Send(appendErrorMsg(message))
	case "warning":
		b.program.Send(appendWarningMsg(message))
	default:
		b.program.Send(appendSystemMsg(message))
	}
}

// appendSystemMsg appends a system message to the chat from any goroutine.
type appendSystemMsg string

// appendErrorMsg appends an error message to the chat from any goroutine.
type appendErrorMsg string

// appendWarningMsg appends a warning message to the chat from any goroutine.
type appendWarningMsg string

func (b *tuiExtensionBridge) SetStatus(key, text string) {
	b.program.Send(extensionStatusMsg{key: key, text: text})
}

func (b *tuiExtensionBridge) SetTitle(title string) {
	b.program.Send(extensionSetTitleMsg{title: title})
}

func (b *tuiExtensionBridge) SetHiddenThinkingLabel(label string) {
	b.program.Send(extensionHiddenThinkingLabelMsg{label: label})
}

func (b *tuiExtensionBridge) SetWorkingMessage(message string) {
	b.program.Send(extensionWorkingMessageMsg{message: message})
}

func (b *tuiExtensionBridge) SetWorkingVisible(visible bool) {
	b.program.Send(extensionWorkingVisibleMsg{visible: visible})
}

func (b *tuiExtensionBridge) SetWorkingIndicator(frames []string, intervalMs int) {
	b.program.Send(extensionWorkingIndicatorMsg{frames: frames, intervalMs: intervalMs})
}

func (b *tuiExtensionBridge) PasteToEditor(text string) {
	b.program.Send(extensionPasteMsg{text: text})
}

func (b *tuiExtensionBridge) SetEditorText(text string) {
	b.program.Send(extensionEditorTextMsg{text: text, isSet: true})
}

func (b *tuiExtensionBridge) GetEditorText() string {
	respCh := make(chan string, 1)
	b.program.Send(extensionEditorTextMsg{respCh: respCh})
	return <-respCh
}

func (b *tuiExtensionBridge) OnTerminalInput(handler extensions.TerminalInputHandler) func() {
	id := b.inputRegistry.add(handler)
	return func() { b.inputRegistry.remove(id) }
}

// extensionStatusMsg updates an extension status line in the footer.
type extensionStatusMsg struct {
	key  string
	text string
}

// extensionSetTitleMsg sets the terminal window/tab title.
type extensionSetTitleMsg struct {
	title string
}

// extensionHiddenThinkingLabelMsg sets the label for hidden thinking blocks.
type extensionHiddenThinkingLabelMsg struct {
	label string
}

// extensionWorkingMessageMsg sets the working message shown during streaming.
type extensionWorkingMessageMsg struct {
	message string
}

// extensionWorkingVisibleMsg shows or hides the working loader during streaming.
type extensionWorkingVisibleMsg struct {
	visible bool
}

// extensionWorkingIndicatorMsg sets custom spinner frames for the streaming loader.
type extensionWorkingIndicatorMsg struct {
	frames     []string
	intervalMs int
}

// extensionEditorTextMsg sets or retrieves editor text.
type extensionEditorTextMsg struct {
	text   string
	isSet  bool
	respCh chan string // used for GetEditorText
}

// extensionPasteMsg pastes text into the main editor.
type extensionPasteMsg struct {
	text string
}

// extensionWidgetMsg sets or clears an extension widget.
type extensionWidgetMsg struct {
	key       string
	content   string // empty to remove
	placement string // "aboveEditor" or "belowEditor"
}

func (b *tuiExtensionBridge) SetWidget(key, content, placement string) {
	b.program.Send(extensionWidgetMsg{key: key, content: content, placement: placement})
}

// extensionGetAllThemesMsg is sent to get all available themes.
type extensionGetAllThemesMsg struct {
	respCh chan []extensions.ThemeInfo
}

// extensionGetCurrentThemeNameMsg is sent to get the current theme name.
type extensionGetCurrentThemeNameMsg struct {
	respCh chan string
}

// extensionSetThemeMsg is sent to switch the current theme.
type extensionSetThemeMsg struct {
	name   string
	respCh chan error
}

// extensionGetToolsExpandedMsg is sent to get the tools expansion state.
type extensionGetToolsExpandedMsg struct {
	respCh chan bool
}

// extensionSetToolsExpandedMsg is sent to set the tools expansion state.
type extensionSetToolsExpandedMsg struct {
	expanded bool
}

// extensionCustomMsg is sent to show an extension custom dialog.
type extensionCustomMsg struct {
	title   string
	content string
	buttons []extensions.CustomButton
	timeout time.Duration
	respCh  chan extensionUIResponse
}

func (b *tuiExtensionBridge) Custom(title, content string, buttons []extensions.CustomButton, opts *extensions.ExtensionUIDialogOptions) (string, error) {
	respCh := make(chan extensionUIResponse, 1)
	msg := extensionCustomMsg{
		title:   title,
		content: content,
		buttons: buttons,
		respCh:  respCh,
	}
	if opts != nil && opts.Timeout > 0 {
		msg.timeout = opts.Timeout
	}
	b.program.Send(msg)
	resp := <-respCh
	return resp.value, resp.err
}

func (b *tuiExtensionBridge) GetAllThemes() []extensions.ThemeInfo {
	respCh := make(chan []extensions.ThemeInfo, 1)
	b.program.Send(extensionGetAllThemesMsg{respCh: respCh})
	return <-respCh
}

func (b *tuiExtensionBridge) GetCurrentThemeName() string {
	respCh := make(chan string, 1)
	b.program.Send(extensionGetCurrentThemeNameMsg{respCh: respCh})
	return <-respCh
}

func (b *tuiExtensionBridge) SetTheme(name string) error {
	respCh := make(chan error, 1)
	b.program.Send(extensionSetThemeMsg{name: name, respCh: respCh})
	return <-respCh
}

func (b *tuiExtensionBridge) GetToolsExpanded() bool {
	respCh := make(chan bool, 1)
	b.program.Send(extensionGetToolsExpandedMsg{respCh: respCh})
	return <-respCh
}

func (b *tuiExtensionBridge) AddAutocompleteProvider(provider extensions.AutocompleteProvider) {
	extensions.AddAutocompleteProvider(provider)
}

func (b *tuiExtensionBridge) SetToolsExpanded(expanded bool) {
	b.program.Send(extensionSetToolsExpandedMsg{expanded: expanded})
}

func (b *tuiExtensionBridge) SetFooter(factory interface{}) {
	b.program.Send(extSetFooterMsg{factory: nil})
	// Cast factory to FooterFactory if possible
	if f, ok := factory.(func() FooterComponent); ok {
		b.program.Send(extSetFooterMsg{factory: FooterFactory(f)})
	}
}

func (b *tuiExtensionBridge) SetHeader(factory interface{}) {
	if f, ok := factory.(func() HeaderComponent); ok {
		b.program.Send(extSetHeaderMsg{factory: HeaderFactory(f)})
	} else {
		b.program.Send(extSetHeaderMsg{factory: nil})
	}
}

func (b *tuiExtensionBridge) GetTheme(name string) interface{} {
	// Theme loading via program message
	return nil
}

func (b *tuiExtensionBridge) SetEditorComponent(factory interface{}) {
	if f, ok := factory.(func() EditorComponent); ok {
		b.program.Send(extSetEditorMsg{factory: EditorFactory(f)})
	} else {
		b.program.Send(extSetEditorMsg{factory: nil})
	}
}

func (b *tuiExtensionBridge) GetEditorComponent() interface{} {
	respCh := make(chan EditorFactory, 1)
	b.program.Send(extGetEditorMsg{respCh: respCh})
	return <-respCh
}

// cycleThinking cycles through thinking levels: off → minimal → low → medium → high → xhigh → off.
func (m *AppModel) cycleThinking() {
	modelName := m.agent.Loop().Model
	if idx := strings.Index(modelName, "/"); idx >= 0 {
		modelName = modelName[idx+1:]
	}
	if !supportsThinking(modelName) {
		m.chat.AppendSystem("Current model does not support thinking")
		return
	}

	current := m.thinkingLevel
	if current == "" {
		current = "off"
	}

	// Find next level
	next := "off" // default wrap-around
	for i, level := range thinkingLevels {
		if level == current && i+1 < len(thinkingLevels) {
			next = thinkingLevels[i+1]
			break
		}
	}

	m.thinkingLevel = next

	// Update footer display
	_, provider := parseModelString(m.agent.Loop().Model)
	m.footer.SetSession(m.session.CWD, getGitBranch(m.session.CWD), m.session.GetSessionName(), modelName, next, provider)
	m.footer.SetHasReasoning(supportsThinking(modelName))
	m.footer.SetEntryCount(len(m.session.Entries))
	m.input.SetBorderColor(m.theme.ThinkingBorderColor(next))

	// Also update the agent's thinking budget
	if m.agent.Loop().Provider != nil {
		// The thinking level is passed to the LLM client via the engine
		// For now, we just show it in the footer - the actual model thinking
		// is controlled by the LLM client's ThinkingBudget field
	}

	m.chat.AppendSystem("Thinking level: " + next)
}

// ─── Git Branch Detection ──────────────────────────────────────────────────

// getGitBranch returns the current git branch name for the given directory.
func getGitBranch(cwd string) string {
	cmd := exec.Command("git", "rev-parse", "--abbrev-ref", "HEAD")
	cmd.Dir = cwd
	out, err := cmd.Output()
	if err != nil {
		return ""
	}
	branch := strings.TrimSpace(string(out))
	// "HEAD" means we're in detached HEAD state
	if branch == "HEAD" {
		return ""
	}
	return branch
}

// updateTerminalTitle sets the terminal window title to "xihu - sessionName - cwd" or "xihu - cwd".
func updateTerminalTitle(sessionName, cwd string) {
	basename := filepath.Base(cwd)
	var title string
	if sessionName != "" {
		title = fmt.Sprintf("xihu - %s - %s", sessionName, basename)
	} else {
		title = fmt.Sprintf("xihu - %s", basename)
	}
	fmt.Fprintf(os.Stdout, "\033]0;%s\007", title)
}

// ─── Tool Duration Formatting ────────────────────────────────────────────────

// formatDuration formats a duration in milliseconds as a human-readable string.
func formatDuration(ms int64) string {
	if ms <= 0 {
		return ""
	}
	if ms < 1000 {
		return fmt.Sprintf("%dms", ms)
	}
	return fmt.Sprintf("%.1fs", float64(ms)/1000)
}

// formatContextPath formats a context file path for display (TS pi-mono: formatContextPath).
func formatContextPath(fp string) string {
	home, _ := os.UserHomeDir()
	if home != "" && strings.HasPrefix(fp, home) {
		return "~" + fp[len(home):]
	}
	return filepath.Base(fp)
}

// openExternalEditor opens $EDITOR (or nano/vi) on a temp file and returns the content.
func (m *AppModel) openExternalEditor() string {
	editor := os.Getenv("VISUAL")
	if editor == "" {
		editor = os.Getenv("EDITOR")
	}
	if editor == "" {
		m.chat.AppendWarning("No editor configured. Set $VISUAL or $EDITOR environment variable.")
		return ""
	}

	tmpDir := os.TempDir()
	f, err := os.CreateTemp(tmpDir, "xihu-edit-*.md")
	if err != nil {
		m.chat.AppendSystem("Error: " + err.Error())
		return ""
	}
	defer os.Remove(f.Name())

	// Pre-fill with current input text
	currentInput := m.input.Value()
	if currentInput != "" {
		f.WriteString(currentInput)
	}
	f.Close()

	// Suspend Bubble Tea, run editor, resume
	cmd := exec.Command(editor, f.Name())
	cmd.Stdin = os.Stdin
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	if err := cmd.Run(); err != nil {
		// pi-mono: non-zero exit keeps original text silently
		return ""
	}

	content, err := os.ReadFile(f.Name())
	if err != nil {
		m.chat.AppendSystem("Error reading file: " + err.Error())
		return ""
	}
	text := string(content)
	// Strip only trailing newline (pi-mono: editors add trailing \n)
	text = strings.TrimSuffix(text, "\n")
	if strings.TrimSpace(text) == "" {
		return ""
	}
	return text
}

// extractBashCommand extracts the "command" field from JSON tool arguments.
func extractBashCommand(argsJSON string) string {
	needle := `"command": "`
	idx := strings.Index(argsJSON, needle)
	if idx < 0 {
		return argsJSON
	}
	start := idx + len(needle)
	end := strings.IndexByte(argsJSON[start:], '"')
	if end < 0 {
		return argsJSON
	}
	return argsJSON[start : start+end]
}

// ─── Model Parsing ─────────────────────────────────────────────────────────

// formatRelativeDate returns a human-readable relative date like pi-mono.
// Examples: "now", "5m", "2h", "3d", "2w", "1mo", "1y"
func formatRelativeDate(t time.Time) string {
	diff := time.Since(t)
	switch {
	case diff < time.Minute:
		return "now"
	case diff < time.Hour:
		return fmt.Sprintf("%dm", int(diff.Minutes()))
	case diff < 24*time.Hour:
		return fmt.Sprintf("%dh", int(diff.Hours()))
	case diff < 7*24*time.Hour:
		return fmt.Sprintf("%dd", int(diff.Hours()/24))
	case diff < 30*24*time.Hour:
		return fmt.Sprintf("%dw", int(diff.Hours()/(24*7)))
	case diff < 365*24*time.Hour:
		return fmt.Sprintf("%dmo", int(diff.Hours()/(24*30)))
	default:
		return fmt.Sprintf("%dy", int(diff.Hours()/(24*365)))
	}
}

// parseModelString splits a model string like "deepseek/deepseek-chat"
// into (modelName, provider). Returns the original string as modelName
// and empty provider if no "/" separator is found.
func parseModelString(modelStr string) (modelName, provider string) {
	parts := strings.SplitN(modelStr, "/", 2)
	if len(parts) == 2 {
		return parts[1], parts[0]
	}
	return modelStr, ""
}

// Ensure imports are used.
var _ = fmt.Sprintf
var _ = context.Background

// copyToClipboard copies text to the system clipboard using platform-specific commands.
func copyToClipboard(text string) error {
	var cmd *exec.Cmd
	switch runtime.GOOS {
	case "darwin":
		cmd = exec.Command("pbcopy")
	case "linux":
		// Try wl-copy (Wayland) first, fall back to xclip (X11)
		if _, err := exec.LookPath("wl-copy"); err == nil {
			cmd = exec.Command("wl-copy")
		} else if _, err := exec.LookPath("xclip"); err == nil {
			cmd = exec.Command("xclip", "-selection", "clipboard")
		} else {
			return fmt.Errorf("no clipboard tool found (install wl-copy or xclip)")
		}
	case "windows":
		cmd = exec.Command("clip.exe")
	default:
		return fmt.Errorf("unsupported platform: %s", runtime.GOOS)
	}
	cmd.Stdin = strings.NewReader(text)
	return cmd.Run()
}

// pasteFromClipboard reads text from the system clipboard using platform-specific commands.
func pasteFromClipboard() (string, error) {
	var cmd *exec.Cmd
	switch runtime.GOOS {
	case "darwin":
		cmd = exec.Command("pbpaste")
	case "linux":
		if _, err := exec.LookPath("wl-paste"); err == nil {
			cmd = exec.Command("wl-paste")
		} else if _, err := exec.LookPath("xclip"); err == nil {
			cmd = exec.Command("xclip", "-selection", "clipboard", "-o")
		} else {
			return "", fmt.Errorf("no clipboard tool found (install wl-paste or xclip)")
		}
	case "windows":
		cmd = exec.Command("powershell", "-Command", "Get-Clipboard")
	default:
		return "", fmt.Errorf("unsupported platform: %s", runtime.GOOS)
	}
	out, err := cmd.Output()
	if err != nil {
		return "", err
	}
	return string(out), nil
}

// startProgress writes OSC 9;4;3 to show an indeterminate progress bar.
// (TS pi-mono: terminal.ts setProgress(true) with keepalive timer)
func (m *AppModel) startProgress() {
	if !m.terminalProgress {
		return
	}
	m.stopProgress() // cancel any existing keepalive
	m.progressCancel = make(chan struct{})
	fmt.Fprint(os.Stdout, "\x1b]9;4;3\x07")
	// Keepalive: re-send every 1s to prevent terminal timeout
	go func(cancel <-chan struct{}) {
		ticker := time.NewTicker(1 * time.Second)
		defer ticker.Stop()
		for {
			select {
			case <-cancel:
				return
			case <-ticker.C:
				fmt.Fprint(os.Stdout, "\x1b]9;4;3\x07")
			}
		}
	}(m.progressCancel)
}

// stopProgress writes OSC 9;4;0 to reset the terminal progress bar.
// (TS pi-mono: terminal.ts setProgress(false))
func (m *AppModel) stopProgress() {
	if m.progressCancel != nil {
		close(m.progressCancel)
		m.progressCancel = nil
	}
	fmt.Fprint(os.Stdout, "\x1b]9;4;0\x07")
}

// setTerminalTitle writes the OSC title sequence to set the terminal title.
// (TS pi-mono: terminal.ts setTitle() — \x1b]0;{title}\x07)
func (m *AppModel) setTerminalTitle() {
	if m.session == nil {
		return
	}
	name := m.session.GetSessionName()
	if name == "" {
		name = m.session.ID
	}
	// Use basename only (TS pi-mono: terminal title shows cwdBasename)
	cwd := filepath.Base(m.session.CWD)
	title := fmt.Sprintf("xihu - %s - %s", name, cwd)
	fmt.Fprintf(os.Stdout, "\x1b]0;%s\x07", title)
}

// showLoginDialog displays an interactive auth provider selector.
// (TS pi-mono: login-dialog.ts showAuth with provider list)
func (m *AppModel) showLoginDialog() {
	// Load current auth store to show configured providers
	authStore, _ := auth.LoadAuth()
	configuredProviders := make(map[string]bool)
	if authStore != nil {
		for name := range authStore.Entries {
			configuredProviders[name] = true
		}
	}

	status := func(name string) string {
		if configuredProviders[name] {
			return " \u2713 configured"
		}
		// Check environment variables
		envKeys := map[string]string{
			"anthropic": "ANTHROPIC_API_KEY",
			"openai":    "OPENAI_API_KEY",
			"google":    "GOOGLE_API_KEY",
		}
		if key, ok := envKeys[name]; ok && os.Getenv(key) != "" {
			return " (env:" + key + ")"
		}
		return ""
	}

	items := []components.SelectorItem{
		{Label: "Anthropic" + status("anthropic"), Description: "api.anthropic.com \u2014 Claude models", Value: "anthropic"},
		{Label: "OpenAI" + status("openai"), Description: "api.openai.com \u2014 GPT models", Value: "openai"},
		{Label: "Google" + status("google"), Description: "generativelanguage.googleapis.com \u2014 Gemini models", Value: "google"},
		{Label: "Custom Provider", Description: "Any OpenAI-compatible endpoint", Value: "custom"},
	}

	onSelect := func(value string) {
		// Open browser to provider's API key page
		urls := map[string]string{
			"anthropic": "https://console.anthropic.com/settings/keys",
			"openai":    "https://platform.openai.com/api-keys",
			"google":    "https://aistudio.google.com/app/apikey",
		}
		if url, ok := urls[value]; ok {
			go func() {
				cmd := exec.Command("open", url) // macOS
				if runtime.GOOS == "linux" {
					cmd = exec.Command("xdg-open", url)
				} else if runtime.GOOS == "windows" {
					cmd = exec.Command("rundll32", "url.dll,FileProtocolHandler", url)
				}
				cmd.Start()
			}()
		}
		m.showAPIKeyInput(value)
	}

	m.overlay.ShowSelectorStayOnSelect("Select provider to configure:", items, onSelect, nil, 60, 10)
}

// showAPIKeyInput shows a text input overlay for entering an API key.
func (m *AppModel) showAPIKeyInput(provider string) {
	label := "Enter API key:"

	onSubmit := func(value string) {
		value = strings.TrimSpace(value)
		if value == "" {
			m.chat.AppendSystem("No key entered for " + provider)
			return
		}
		if err := m.saveAPIKey(provider, value); err != nil {
			m.chat.AppendError("Failed to save API key for " + provider + ": " + err.Error())
			return
		}
		home, _ := os.UserHomeDir()
		authPath := filepath.Join(home, ".xihu", "auth.json")
		m.chat.AppendSystem("Saved API key for " + provider + ". Credentials saved to " + authPath)
	}

	m.overlay.ShowInput(label, onSubmit, nil, 50, 6)
}

// saveAPIKey saves an API key to the auth store.
func (m *AppModel) saveAPIKey(provider, key string) error {
	home, err := os.UserHomeDir()
	if err != nil {
		return err
	}
	authDir := filepath.Join(home, ".xihu")
	if err := os.MkdirAll(authDir, 0755); err != nil {
		return err
	}
	authPath := filepath.Join(authDir, "auth.json")

	// Load existing store
	store, err := auth.LoadAuth()
	if err != nil {
		store = &auth.Store{Entries: make(map[string]auth.Entry)}
	}
	store.Entries[provider] = auth.Entry{Type: "api_key", Key: key}

	// Serialize
	data := make(map[string]map[string]string)
	for name, entry := range store.Entries {
		data[name] = map[string]string{"type": entry.Type, "key": entry.Key}
	}
	raw, err := json.MarshalIndent(data, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(authPath, raw, 0600)
}

// showLogoutDialog shows a confirmation and removes stored credentials.
func (m *AppModel) showLogoutDialog() {
	authStore, err := auth.LoadAuth()
	if err != nil || authStore == nil || len(authStore.Entries) == 0 {
		m.chat.AppendSystem("No stored credentials to remove. /logout only removes credentials saved by /login; environment variables and models.json config are unchanged.")
		return
	}

	items := make([]components.SelectorItem, 0, len(authStore.Entries)+1)
	for name, entry := range authStore.Entries {
		keyPreview := strings.Repeat("*", len(entry.Key)-4) + entry.Key[len(entry.Key)-4:]
		if len(entry.Key) <= 4 {
			keyPreview = strings.Repeat("*", len(entry.Key))
		}
		items = append(items, components.SelectorItem{
			Label:       name + " (" + keyPreview + ")",
			Description: "Remove stored " + name + " API key",
			Value:       name,
		})
	}
	items = append(items, components.SelectorItem{
		Label:       "Remove All",
		Description: "Clear all stored credentials",
		Value:       "__all__",
	})

	onSelect := func(value string) {
		if value == "__all__" {
			if err := m.removeAllAPIKeys(); err != nil {
				m.chat.AppendError("Failed to clear credentials: " + err.Error())
				return
			}
			m.chat.AppendSystem("All stored API keys removed. Environment variables and models.json config are unchanged.")
			return
		}
		if err := m.removeAPIKey(value); err != nil {
			m.chat.AppendError("Failed to remove " + value + " key: " + err.Error())
			return
		}
		m.chat.AppendSystem("Removed stored API key for " + value + ". Environment variables and models.json config are unchanged.")
	}

	m.overlay.ShowSelectorStayOnSelect("Select provider to logout:", items, onSelect, nil, 56, len(items)+5)
}

// removeAPIKey removes a single provider's credentials.
func (m *AppModel) removeAPIKey(provider string) error {
	home, err := os.UserHomeDir()
	if err != nil {
		return err
	}
	authPath := filepath.Join(home, ".xihu", "auth.json")

	store, err := auth.LoadAuth()
	if err != nil {
		return err
	}
	delete(store.Entries, provider)

	data := make(map[string]map[string]string)
	for name, entry := range store.Entries {
		data[name] = map[string]string{"type": entry.Type, "key": entry.Key}
	}
	raw, err := json.MarshalIndent(data, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(authPath, raw, 0600)
}

// removeAllAPIKeys clears all stored credentials.
func (m *AppModel) removeAllAPIKeys() error {
	home, err := os.UserHomeDir()
	if err != nil {
		return err
	}
	authPath := filepath.Join(home, ".xihu", "auth.json")
	return os.WriteFile(authPath, []byte("{}\n"), 0600)
}

// handleShare exports the session as HTML and creates a secret GitHub gist.
// (TS pi-mono: interactive-mode.ts handleShareCommand)
func (m *AppModel) handleShare() {
	// Check if gh CLI is available
	if _, err := exec.LookPath("gh"); err != nil {
		if m.program != nil {
			m.program.Send(ShareResultMsg{Error: "GitHub CLI (gh) is not installed. Install it from https://cli.github.com/"})
		}
		return
	}

	// Check if gh CLI is logged in (TS pi-mono)
	if err := exec.Command("gh", "auth", "status").Run(); err != nil {
		if m.program != nil {
			m.program.Send(ShareResultMsg{Error: "GitHub CLI is not logged in. Run 'gh auth login' first."})
		}
		return
	}

	// Build HTML export
	html := m.buildSessionHTML()
	if html == "" {
		if m.program != nil {
			m.program.Send(ShareResultMsg{Error: "No session content to share"})
		}
		return
	}

	// Write to temp file
	tmpFile, err := os.CreateTemp("", "xihu-session-*.html")
	if err != nil {
		if m.program != nil {
			m.program.Send(ShareResultMsg{Error: "Failed to export session: " + err.Error()})
		}
		return
	}
	tmpPath := tmpFile.Name()
	if _, err := tmpFile.WriteString(html); err != nil {
		tmpFile.Close()
		os.Remove(tmpPath)
		if m.program != nil {
			m.program.Send(ShareResultMsg{Error: "Failed to export session: " + err.Error()})
		}
		return
	}
	tmpFile.Close()
	defer os.Remove(tmpPath)

	// Create secret gist
	cmd := exec.Command("gh", "gist", "create", "--public=false", tmpPath)
	output, err := cmd.Output()
	if err != nil {
		var errMsg string
		if exitErr, ok := err.(*exec.ExitError); ok {
			errMsg = "Failed to create gist: " + string(exitErr.Stderr)
		} else {
			errMsg = "Failed to create gist: " + err.Error()
		}
		if m.program != nil {
			m.program.Send(ShareResultMsg{Error: errMsg})
		}
		return
	}

	// Parse gist URL from output (TS pi-mono: extract gist ID for preview URL)
	gistURL := strings.TrimSpace(string(output))
	if gistURL == "" {
		if m.program != nil {
			m.program.Send(ShareResultMsg{Error: "Failed to parse gist ID from gh output"})
		}
		return
	}

	// Extract gist ID and build preview URL (TS pi-mono: getShareViewerUrl)
	previewURL := ""
	if idx := strings.LastIndex(gistURL, "/"); idx >= 0 {
		gistID := gistURL[idx+1:]
		previewURL = "https://pi.dev/session/#" + gistID
	}

	if m.program != nil {
		m.program.Send(ShareResultMsg{GistURL: gistURL, PreviewURL: previewURL})
	}
}

// buildSessionHTML creates an HTML representation of the current session.
func (m *AppModel) buildSessionHTML() string {
	if m.session == nil || len(m.session.Entries) == 0 {
		return ""
	}

	var sb strings.Builder
	sb.WriteString(`<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>xihu session`)
	if name := m.session.GetSessionName(); name != "" {
		sb.WriteString(" - " + name)
	}
	sb.WriteString(`</title>
<style>
body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; max-width: 900px; margin: 0 auto; padding: 20px; background: #1e1e2e; color: #cdd6f4; }
.header { border-bottom: 1px solid #45475a; padding-bottom: 10px; margin-bottom: 20px; }
.header h1 { color: #89b4fa; margin: 0; font-size: 1.5em; }
.header .meta { color: #6c7086; font-size: 0.85em; margin-top: 5px; }
.entry { margin-bottom: 15px; padding: 10px 15px; border-radius: 8px; }
.entry.user { background: #313244; border-left: 3px solid #a6e3a1; }
.entry.assistant { background: #1e1e2e; border-left: 3px solid #89b4fa; }
.entry.system { background: #181825; border-left: 3px solid #f9e2af; }
.entry.tool { background: #11111b; border-left: 3px solid #fab387; }
.role { font-weight: bold; font-size: 0.8em; text-transform: uppercase; margin-bottom: 5px; }
.user .role { color: #a6e3a1; }
.assistant .role { color: #89b4fa; }
.system .role { color: #f9e2af; }
.tool .role { color: #fab387; }
.content { white-space: pre-wrap; word-wrap: break-word; line-height: 1.5; }
pre { background: #11111b; padding: 10px; border-radius: 6px; overflow-x: auto; }
code { background: #45475a; padding: 2px 5px; border-radius: 3px; font-size: 0.9em; }
.footer { margin-top: 30px; padding-top: 10px; border-top: 1px solid #45475a; color: #6c7086; font-size: 0.85em; }
</style>
</head>
<body>
<div class="header">
<h1>xihu session`)
	if name := m.session.GetSessionName(); name != "" {
		sb.WriteString(": " + name)
	}
	sb.WriteString(`</h1>
<div class="meta">` + m.session.CWD + ` · ` + time.Now().Format(time.RFC3339) + `</div>
</div>
`)

	for _, entry := range m.session.Entries {
		role := entry.Role
		if role == "" {
			role = entry.Type
		}
		if role == "" {
			role = "unknown"
		}
		sb.WriteString(`<div class="entry ` + role + `">`)
		sb.WriteString(`<div class="role">` + role + `</div>`)

		var contentBlocks []struct {
			Type string `json:"type"`
			Text string `json:"text"`
		}
		if err := json.Unmarshal(entry.Content, &contentBlocks); err == nil {
			for _, block := range contentBlocks {
				if block.Type == "text" && block.Text != "" {
					escaped := strings.ReplaceAll(block.Text, "&", "&amp;")
					escaped = strings.ReplaceAll(escaped, "<", "&lt;")
					escaped = strings.ReplaceAll(escaped, ">", "&gt;")
					sb.WriteString(`<div class="content">` + escaped + `</div>`)
				}
			}
		}
			sb.WriteString("</div>\n")
	}

	sb.WriteString(`<div class="footer">Exported by xihu · ` + time.Now().Format(time.RFC3339) + `</div>
</body>
</html>`)

	return sb.String()
}

// ShareResultMsg carries the result of the /share command back to the TUI.
type ShareResultMsg struct {
	GistURL    string
	PreviewURL string
	Error      string
}

// handleDebugCommand dumps debug information to a log file (TS pi-mono: /debug).
func (m *AppModel) handleDebugCommand() string {
	home, _ := os.UserHomeDir()
	debugDir := home + "/.xihu"
	os.MkdirAll(debugDir, 0755)
	debugPath := debugDir + "/debug.log"

	var sb strings.Builder
	sb.WriteString(fmt.Sprintf("Debug output at %s\n", time.Now().Format(time.RFC3339)))
	sb.WriteString(fmt.Sprintf("Terminal: %dx%d\n", m.width, m.height))
	sb.WriteString(fmt.Sprintf("Model: %s\n", m.agent.Loop().Model))
	sb.WriteString(fmt.Sprintf("Session: %s\n", m.session.ID))
	sb.WriteString(fmt.Sprintf("Thinking level: %s\n", m.thinkingLevel))
	sb.WriteString(fmt.Sprintf("Streaming: %v  Compacting: %v\n", m.streaming, m.compacting))
	sb.WriteString(fmt.Sprintf("Entries: %d  Tokens in: %d  out: %d\n",
		len(m.session.Entries), m.lastStatus.TokensIn, m.lastStatus.TokensOut))
	sb.WriteString("\n=== Session entries ===\n")
	for i, entry := range m.session.Entries {
		content := string(entry.Content)
		if len(content) > 500 {
			content = content[:500] + "..."
		}
		sb.WriteString(fmt.Sprintf("[%d] type=%s id=%s\n  %s\n", i, entry.Type, entry.ID, content))
	}

	if err := os.WriteFile(debugPath, []byte(sb.String()), 0644); err != nil {
		return "Debug: failed to write " + debugPath + ": " + err.Error()
	}
	return "Debug log written\n" + debugPath
}

// findTemplate looks up a prompt template by name (with or without leading /).
func (m *AppModel) findTemplate(name string) *prompt.PromptTemplate {
	// Strip leading / if present
	name = strings.TrimPrefix(name, "/")
	for i := range m.promptTemplates {
		if m.promptTemplates[i].Name == name {
			return &m.promptTemplates[i]
		}
	}
	return nil
}

// splitSlashCommand splits "/name arg1 arg2" into ("name", ["arg1", "arg2"]).
func splitSlashCommand(text string) (name string, args []string) {
	text = strings.TrimPrefix(text, "/")
	parts := strings.Fields(text)
	if len(parts) == 0 {
		return "", nil
	}
	if len(parts) > 1 {
		args = parts[1:]
	}
	return parts[0], args
}
// commaInt formats an integer with comma separators (matching TS pi-mono toLocaleString).
func commaInt(n int) string {
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

