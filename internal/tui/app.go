// Package tui provides the interactive terminal UI for xihu using Bubble Tea.
package tui

import (
	"context"
	"encoding/json"
	"fmt"
	"os/exec"
	"strings"
	"sync/atomic"
	"time"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"

	"github.com/huichen/xihu/internal/agent"
	"github.com/huichen/xihu/internal/commands"
	"github.com/huichen/xihu/internal/events"
	"github.com/huichen/xihu/internal/session"
	"github.com/huichen/xihu/internal/tui/components"
	"github.com/huichen/xihu/pkg/types"
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

// ToolResultMsg delivers the result of a tool execution.
type ToolResultMsg struct {
	ID     string
	Output string
	Error  string
}

// AgentDoneMsg signals the agent has finished processing.
type AgentDoneMsg struct {
	FinalText string
}

// AgentErrorMsg signals an error from the agent.
type AgentErrorMsg struct {
	Error error
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

// ResizeMsg indicates terminal size change.
type ResizeMsg struct {
	Width  int
	Height int
}

// WelcomeMsg signals the app to display the startup banner.
type WelcomeMsg struct {
	ThemeAccent string
	CWD         string
	Skills      []string
	Extensions  []string
}

// ─── App Model ─────────────────────────────────────────────────────────────

// AppModel is the root Bubble Tea model for the xihu TUI.
type AppModel struct {
	width  int
	height int

	// Sub-components
	chat         *components.ChatViewport
	footer       *components.Footer
	input        *components.Editor
	overlay      *components.Overlay
	autocomplete *components.Autocomplete

	// Agent state
	agent   *agent.Loop
	session *session.Session
	sessMgr *session.Manager

	// EventBus bridges agent streaming events to Bubble Tea messages
	eventBus *events.EventBus

	// Program reference for sending messages from goroutines
	program *tea.Program

	// Loaded resources
	Skills        []string
	Extensions    []string
	thinkingLevel string

	// Theme
	theme *Theme

	// Derived state
	streaming  bool
	compacting bool
	quitting   bool

	// Accumulated stats across agent runs
	lastStatus StatusMsg

	// streamID is an atomic counter that changes on each new submission / interrupt.
	// The EventBus forwarding goroutine checks it to discard events from stale streams.
	streamID int32

	// Help overlay state
	showHelp        bool
	welcomeExpanded bool
}

// NewAppModel creates a new AppModel.
func NewAppModel(agt *agent.Loop, sessMgr *session.Manager, sess *session.Session, theme *Theme, modelStr string, skills []string, extensions []string, thinkingLevel string) AppModel {
	chat := components.NewChatViewport()
	footer := components.NewFooter(theme.FooterStyle(), theme.ContextGreen, theme.ContextYellow, theme.ContextRed)
	input := components.NewEditor(theme.InputStyle())
	overlay := components.NewOverlay()
	ac := components.NewAutocomplete()

	app := AppModel{
		chat:          &chat,
		footer:        &footer,
		input:         &input,
		overlay:       &overlay,
		autocomplete:  &ac,
		agent:         agt,
		session:       sess,
		sessMgr:       sessMgr,
		theme:         theme,
		thinkingLevel: thinkingLevel,
	}

	// Set skills and extensions
	if len(skills) > 0 {
		app.Skills = skills
	}
	if len(extensions) > 0 {
		app.Extensions = extensions
	}

	// Wire footer with session info + parsed model/provider
	cwd := ""
	if sess != nil {
		cwd = sess.CWD
	}
	gitBranch := getGitBranch(cwd)
	modelName, provider := parseModelString(modelStr)
	// Use explicit thinkingLevel parameter (not extracted from modelStr)
	app.footer.SetSession(cwd, gitBranch, "", modelName, thinkingLevel, provider)

	// Create EventBus and attach to agent for fine-grained events
	app.eventBus = events.NewEventBus()
	agt.EventBus = app.eventBus

	return app
}

// Init is the first command run when the program starts.
func (m AppModel) Init() tea.Cmd {
	return tea.Batch(
		tea.EnterAltScreen,
		m.input.Focus(),
		func() tea.Msg {
			return WelcomeMsg{
				ThemeAccent: m.theme.Accent,
				CWD:         m.session.CWD,
				Skills:      m.Skills,
				Extensions:  m.Extensions,
			}
		},
	)
}

// Update handles messages and updates the model.
func (m AppModel) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.WindowSizeMsg:
		m.width = msg.Width
		m.height = msg.Height
		m.chat.SetSize(msg.Width, msg.Height-6)
		m.input.SetWidth(msg.Width - 4)
		m.footer.SetWidth(msg.Width)
		return m, nil

	case WelcomeMsg:
		m.showWelcome(msg)
		return m, nil

	case tea.MouseMsg:
		// Route mouse events to chat viewport for native scroll handling
		_, _ = m.chat.Update(msg)
		return m, nil

	case tea.KeyMsg:
		switch msg.String() {
		case "ctrl+c":
			if m.input.Empty() && !m.streaming {
				m.quitting = true
				return m, tea.Quit
			}
			// If input has text, let it fall through to textarea (handles copy)
		case "ctrl+d":
			if !m.streaming && !m.compacting && m.input.Empty() {
				m.quitting = true
				return m, tea.Quit
			}
		case "ctrl+o":
			// Cycle: collapsed welcome → expanded welcome → inline help → collapsed welcome
			if m.showHelp {
				m.showHelp = false
				m.welcomeExpanded = false
			} else if m.welcomeExpanded {
				m.welcomeExpanded = false
				m.showHelp = true
				m.chat.AppendSystem(m.buildHelpOverlay())
			} else {
				m.welcomeExpanded = true
			}
			return m, nil
		case "esc":
			// TS pi-mono: Escape during streaming = abort current LLM call
			if m.streaming {
				m.agent.Abort()
				m.chat.AppendSystem("⏹ Aborted")
				return m, nil
			}
		case "shift+tab":
			// Cycle thinking level: off → low → medium → high → xhigh → off
			m.cycleThinking()
			return m, nil
		case "ctrl+t":
			// Toggle thinking visibility (TS pi-mono: hideThinkingBlock)
			m.chat.HideAllThinking = !m.chat.HideAllThinking
			if m.chat.HideAllThinking {
				m.chat.AppendSystem("Thinking: hidden")
			} else {
				m.chat.AppendSystem("Thinking: visible")
			}
			return m, nil
		}

		// Route to overlay if active (non-help overlays only; help is now inline)
		if m.overlay.Active() {
			cmd := m.overlay.Update(msg)
			return m, cmd
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
			case "enter", "tab":
				selected := m.autocomplete.Selected()
				if selected != "" {
					m.input.SetValue(selected)
					m.autocomplete.Hide()
				}
				return m, nil
			}
		}

		// Route scroll/chat keys to chat viewport (handles pgup/pgdown/ctrl+u/ctrl+d/mouse wheel natively via bubbles/viewport)
		switch msg.String() {
		case "pgup", "pgdown", "ctrl+u", "ctrl+d", "home", "end":
			_, cmd := m.chat.Update(msg)
			return m, cmd
		}

	case components.SubmitMsg:
		text := string(msg)
		if m.streaming {
			// TS-style steer: inject message without aborting current stream
			m.chat.AppendSystem("⏎ " + text)
			m.agent.Steer(text)
			return m, nil
		}
		if m.compacting {
			m.chat.AppendSystem("⏳ " + text + " (queued)")
			return m, nil
		}
		{
			atomic.AddInt32(&m.streamID, 1)
			if strings.HasPrefix(text, "/") {
				m.chat.AppendSystem("Cmd: " + text)
				result := m.handleSlashCmd(text)
				m.chat.AppendSystem(result)
			} else {
				m.chat.AppendSystem("You: " + text)
				go m.runAgent(text, m.streamID)
			}
		}
		return m, nil

	case components.FollowUpMsg:
		// TS pi-mono: Alt+Enter queues message for after agent finishes
		text := string(msg)
		m.chat.AppendSystem("⏩ " + text + " (queued)")
		m.agent.Steer(text) // Uses SteeringQueue → processed on next turn
		return m, nil

	case StreamTextMsg:
		m.chat.AppendText(string(msg))
		return m, nil

	case ThinkingMsg:
		m.chat.AppendThinking(string(msg))
		return m, nil

	case ToolCallMsg:
		m.chat.AddToolCall(msg.ID, msg.Name, msg.Arguments)
		return m, nil

	case ToolResultMsg:
		if msg.Error != "" {
			m.chat.UpdateToolResult(msg.ID, msg.Error, true)
		} else {
			m.chat.UpdateToolResult(msg.ID, msg.Output, false)
		}
		return m, nil

	case AgentDoneMsg:
		m.streaming = false
		return m, nil

	case AgentErrorMsg:
		m.streaming = false
		m.chat.AppendError(msg.Error.Error())
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

	case ResizeMsg:
		m.width = msg.Width
		m.height = msg.Height
		return m, nil
	}

	// Route to input editor
	var cmd tea.Cmd
	*m.input, cmd = m.input.Update(msg)

	// After editor update, check for slash mode and update autocomplete
	if m.input.IsSlashMode() {
		prefix := m.input.GetSlashPrefix()
		candidates := m.filterSlashCandidates(prefix)
		names := make([]string, len(candidates))
		for i, c := range candidates {
			names[i] = c.Name
		}
		m.input.SetSlashCandidates(names)
		// Update autocomplete overlay with formatted candidates
		m.updateAutocomplete(candidates, prefix)
	} else {
		m.autocomplete.Hide()
	}

	return m, cmd
}

// View renders the entire UI.
func (m AppModel) View() string {
	if m.quitting {
		return "Goodbye.\n"
	}

	chatView := m.chat.View()
	inputView := m.input.View()
	footerView := m.footer.View()

	main := lipgloss.JoinVertical(
		lipgloss.Top,
		chatView,
		inputView,
		footerView,
	)

	// Show autocomplete popover above the input
	if m.autocomplete.Active() {
		acView := m.autocomplete.View()
		return lipgloss.JoinVertical(lipgloss.Top, chatView, acView, inputView, footerView)
	}

	// Show generic overlay
	if m.overlay.Active() {
		overlayView := m.overlay.View()
		return lipgloss.Place(
			m.width, m.height,
			lipgloss.Center, lipgloss.Center,
			overlayView,
			lipgloss.WithWhitespaceChars(" "),
			lipgloss.WithWhitespaceForeground(lipgloss.Color("#000000")),
		)
	}

	return main
}

// ─── Startup Banner ────────────────────────────────────────────────────────

func (m *AppModel) showWelcome(msg WelcomeMsg) {
	accentStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color(msg.ThemeAccent)).
		Bold(true)
	dimStyle := lipgloss.NewStyle().
		Faint(true)

	m.chat.AppendSystem(accentStyle.Render("xihu v0.1.0"))

	if !m.welcomeExpanded {
		// Collapsed: one-line summary
		m.chat.AppendSystem(dimStyle.Render("  Esc interrupt · ctrl+c clear/exit · / commands · ctrl+o more"))
		return
	}

	// Expanded: full keybinding guide
	m.chat.AppendSystem("  Enter=submit · Ctrl+J=newline · / commands · ! bash")
	m.chat.AppendSystem("  Esc=abort · Ctrl+C=clear/exit · Ctrl+D=exit")
	m.chat.AppendSystem("  Shift+Tab=cycle thinking · Ctrl+O=toggle help · PgUp/PgDn/Ctrl+U/Ctrl+D=scroll")

	// Show loaded skills
	if len(msg.Skills) > 0 {
		m.chat.AppendSystem("[Skills] " + strings.Join(msg.Skills, ", "))
	}

	// Show loaded extensions
	if len(msg.Extensions) > 0 {
		m.chat.AppendSystem("[Extensions] " + strings.Join(msg.Extensions, ", "))
	}
}

// ─── Help Overlay ──────────────────────────────────────────────────────────

func (m *AppModel) buildHelpOverlay() string {
	var sb strings.Builder

	// Title
	titleStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color(m.theme.Accent)).
		Bold(true)
	sb.WriteString(titleStyle.Render("xihu v0.1.0 — Help"))
	sb.WriteString("\n\n")

	// Keybindings by category
	km := DefaultKeyMap()
	groups := km.ByCategory()

	categoryOrder := []string{"global", "editor", "chat", "tools"}
	categoryTitles := map[string]string{
		"global": "Global",
		"editor": "Editor",
		"chat":   "Chat",
		"tools":  "Tools",
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
			sb.WriteString("  ")
			sb.WriteString(keyStyle.Render(b.Key))
			sb.WriteString(descStyle.Render(b.Description))
			sb.WriteByte('\n')
		}
		sb.WriteByte('\n')
	}

	// Loaded resources
	sb.WriteString(titleStyle.Render("▸ Loaded Resources"))
	sb.WriteByte('\n')

	// Skills
	if len(m.Skills) > 0 {
		sb.WriteString("  [Skills] " + strings.Join(m.Skills, ", "))
	} else {
		sb.WriteString("  [Skills] none")
	}
	sb.WriteByte('\n')

	// Extensions
	if len(m.Extensions) > 0 {
		sb.WriteString("  [Extensions] " + strings.Join(m.Extensions, ", "))
	} else {
		sb.WriteString("  [Extensions] none")
	}
	sb.WriteByte('\n')

	// Prompts & themes (placeholder for future)
	sb.WriteString("  [Prompts] user custom prompts directory")
	sb.WriteByte('\n')
	sb.WriteString("  [Themes] default, light + custom themes")
	sb.WriteByte('\n')

	sb.WriteByte('\n')
	sb.WriteString("Press Esc or Enter to close.")

	return sb.String()
}

// ─── Slash Command Autocomplete ────────────────────────────────────────────

// filterSlashCandidates filters SlashCommandsWithDesc by the given prefix.
func (m *AppModel) filterSlashCandidates(prefix string) []components.SlashCommand {
	all := components.SlashCommandsWithDesc()
	if prefix == "" {
		return all
	}
	lower := strings.ToLower(prefix)
	var filtered []components.SlashCommand
	for _, sc := range all {
		if strings.HasPrefix(strings.ToLower(sc.Name), "/"+lower) ||
			strings.HasPrefix(strings.ToLower(sc.Name), lower) {
			filtered = append(filtered, sc)
		}
	}
	return filtered
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
		messages = session.BuildContext(m.session.Entries)
	}
	userMsg := types.Message{
		Role:    "user",
		Content: jsonMarshalContent(text),
	}
	messages = append(messages, userMsg)

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
			case "toolcall_end":
				id, _ := evt.Data["tool_id"].(string)
				name, _ := evt.Data["tool_name"].(string)
				args, _ := evt.Data["args"].(string)
				if m.program != nil {
					m.program.Send(ToolCallMsg{ID: id, Name: name, Arguments: args})
				}
			case "tool_end":
				name, _ := evt.Data["tool_name"].(string)
				result, _ := evt.Data["result"].(string)
				errStr, _ := evt.Data["error"].(string)
				// Use tool_name as ID for UpdateToolResult matching
				if m.program != nil {
					m.program.Send(ToolResultMsg{ID: name, Output: result, Error: errStr})
				}
			case "usage":
				if in, ok := evt.Data["input_tokens"].(float64); ok {
					tokensIn += int(in)
				}
				if out, ok := evt.Data["output_tokens"].(float64); ok {
					tokensOut += int(out)
				}
				if cr, ok := evt.Data["cache_read_tokens"].(float64); ok {
					cacheR += int(cr)
				}
				if cw, ok := evt.Data["cache_write_tokens"].(float64); ok {
					cacheW += int(cw)
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
	_, _, err := m.agent.RunStreamingWithMessages(ctx, messages, func(chunk string) {
		if m.program != nil && atomic.LoadInt32(&m.streamID) == myID {
			m.program.Send(StreamTextMsg(chunk))
		}
	})

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

// handleSlashCmd processes a slash command and returns the result string.
// Local commands (model, thinking, quit, hotkeys) are handled here;
// everything else is forwarded to the commands.Handle() subsystem.
func (m *AppModel) handleSlashCmd(text string) string {
	parts := strings.Fields(text)
	if len(parts) == 0 {
		return ""
	}
	cmd := strings.ToLower(parts[0])

	// Local TUI-only commands
	switch cmd {
	case "/help":
		return "xihu — AI coding assistant. Type /hotkeys for shortcuts, /model to switch models."
	case "/hotkeys":
		m.chat.AppendSystem(m.buildHelpOverlay())
		return "Keybindings shown above"
	case "/model":
		if len(parts) > 1 {
			m.agent.Model = parts[1]
			modelName, provider := parseModelString(parts[1])
			m.footer.SetSession(m.session.CWD, getGitBranch(m.session.CWD), "", modelName, "", provider)
			return "Model set to: " + parts[1]
		}
		return "Current model: " + m.agent.Model
	case "/quit":
		m.quitting = true
		return "Goodbye."
	case "/thinking":
		m.cycleThinking()
		return "Thinking: " + m.thinkingLevel
	}

	// Forward to commands.Handle for all other commands
	ctx := &commands.Context{
		CWD:              m.session.CWD,
		SessionDir:       m.sessMgr.Dir,
		SettingsDir:      m.sessMgr.Dir, // approximate
		CurrentSessionID: m.session.ID,
		Model:            m.agent.Model,
		SystemPrompt:     m.agent.SystemPrompt,
	}
	result, err := commands.Handle(text, ctx)
	if err != nil {
		return "Error: " + err.Error()
	}
	return result
}

// ─── Thinking Level Cycling ─────────────────────────────────────────────────

var thinkingLevels = []string{"off", "low", "medium", "high", "xhigh"}

// cycleThinking cycles through thinking levels: off → low → medium → high → xhigh → off.
func (m *AppModel) cycleThinking() {
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
	_, provider := parseModelString(m.agent.Model)
	modelName := m.agent.Model
	if idx := strings.Index(modelName, "/"); idx >= 0 {
		modelName = modelName[idx+1:]
	}
	m.footer.SetSession(m.session.CWD, getGitBranch(m.session.CWD), "", modelName, next, provider)

	// Also update the agent's thinking budget
	if m.agent.Provider != nil {
		// The thinking level is passed to the LLM client via the engine
		// For now, we just show it in the footer - the actual model thinking
		// is controlled by the LLM client's ThinkingBudget field
	}

	m.chat.AppendSystem("Thinking: " + next)
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

// ─── Model Parsing ─────────────────────────────────────────────────────────

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
