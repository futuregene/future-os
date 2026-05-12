// Package tui provides the interactive terminal UI for xihu using Bubble Tea.
package tui

import (
	"os"
	"time"

	tea "github.com/charmbracelet/bubbletea"

	agentsession "github.com/huichen/xihu/internal/agentsession"
	"github.com/huichen/xihu/internal/events"
	"github.com/huichen/xihu/internal/extensions"
	"github.com/huichen/xihu/internal/modelregistry"
	"github.com/huichen/xihu/internal/prompt"
	"github.com/huichen/xihu/internal/session"
	"github.com/huichen/xihu/internal/settings"
	"github.com/huichen/xihu/internal/skills"
	"github.com/huichen/xihu/internal/tui/components"
	"github.com/huichen/xihu/internal/utils"


)

// ─── Message Types ─────────────────────────────────────────────────────────

// StreamTextMsg is a chunk of streamed text from the LLM.

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
	as.Loop().SteeringQueue.Mode = app.steeringMode

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
	app.footer.SetHasReasoning(supportsThinking(modelName) || (thinkingLevel != "" && thinkingLevel != "off"))
	app.input.SetBorderColor(app.theme.ThinkingBorderColor(thinkingLevel))
	app.input.SetBashBorderColor("#98c379")  // green (TS pi-mono: bashMode)
	app.input.SetSlashBorderColor("#61afef") // blue (default)
	app.input.SetFileBorderColor("#e5c07b")  // yellow/amber (TS pi-mono: @ file mode)
	app.input.SetSymbolBorderColor("#c678dd") // magenta/purple (TS pi-mono: # symbol mode)
	if sess != nil {
		app.footer.SetEntryCount(len(sess.Entries))
	}

	// Look up model context window from registry (TS pi-mono: shows context window in footer)
	contextWindow := 0
	for _, mi := range modelregistry.BuiltinModels() {
		if mi.ID == modelName {
			contextWindow = mi.ContextWindow
			break
		}
	}
	app.footer.SetContextUsage(0, contextWindow, app.autoCompact)

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
		// tea.EnterAltScreen, // disabled: allows scrolling up to pre-launch terminal output
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
