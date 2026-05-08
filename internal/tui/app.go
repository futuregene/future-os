// Package tui provides the interactive terminal UI for cobalt using Bubble Tea.
package tui

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"

	"github.com/huichen/cobalt/internal/agent"
	"github.com/huichen/cobalt/internal/session"
	"github.com/huichen/cobalt/internal/tui/components"
	"github.com/huichen/cobalt/pkg/types"
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
	Compacting   bool
	Streaming    bool
}

// ResizeMsg indicates terminal size change.
type ResizeMsg struct {
	Width  int
	Height int
}

// ─── App Model ─────────────────────────────────────────────────────────────

// AppModel is the root Bubble Tea model for the cobalt TUI.
type AppModel struct {
	width  int
	height int

	// Sub-components
	chat    *components.ChatViewport
	footer  *components.Footer
	input   *components.Editor
	overlay *components.Overlay

	// Agent state
	agent   *agent.Loop
	session *session.Session
	sessMgr *session.Manager

	// Program reference for sending messages from goroutines
	program *tea.Program

	// Derived state
	streaming  bool
	compacting bool
	quitting   bool
}

// NewAppModel creates a new AppModel.
func NewAppModel(agt *agent.Loop, sessMgr *session.Manager, sess *session.Session, theme *Theme) AppModel {
	chat := components.NewChatViewport()
	footer := components.NewFooter(theme.FooterStyle())
	input := components.NewEditor(theme.InputStyle())
	overlay := components.NewOverlay()

	return AppModel{
		chat:    &chat,
		footer:  &footer,
		input:   &input,
		overlay: &overlay,
		agent:   agt,
		session: sess,
		sessMgr: sessMgr,
	}
}

// Init is the first command run when the program starts.
func (m AppModel) Init() tea.Cmd {
	return tea.Batch(
		tea.EnterAltScreen,
		m.input.Focus(),
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
		return m, nil

	case tea.KeyMsg:
		switch msg.String() {
		case "ctrl+c":
			m.quitting = true
			return m, tea.Quit
		case "ctrl+d":
			if !m.streaming && !m.compacting && m.input.Empty() {
				m.quitting = true
				return m, tea.Quit
			}
		}

		// Route to overlay if active
		if m.overlay.Active() {
			cmd := m.overlay.Update(msg)
			return m, cmd
		}

	case components.SubmitMsg:
		if !m.streaming && !m.compacting {
			text := string(msg)
			if strings.HasPrefix(text, "/") {
				m.chat.AppendSystem("Cmd: " + text)
			} else {
				go m.runAgent(text)
			}
		}
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
		m.footer.Update(
			msg.TokensIn, msg.TokensOut,
			msg.TokensCacheR, msg.TokensCacheW,
			msg.TotalCost, msg.ContextUsed,
			msg.Compacting, msg.Streaming,
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

// ─── Agent Integration ─────────────────────────────────────────────────────

// runAgent sends user input to the agent loop in a goroutine.
func (m *AppModel) runAgent(text string) {
	m.streaming = true
	m.chat.AppendSystem("You: " + text)

	var messages []types.Message
	if m.session != nil && len(m.session.Entries) > 0 {
		messages = session.BuildContext(m.session.Entries)
	}
	userMsg := types.Message{
		Role:    "user",
		Content: jsonMarshalContent(text),
	}
	messages = append(messages, userMsg)

	ctx := context.Background()
	_, _, err := m.agent.RunStreamingWithMessages(ctx, messages, func(chunk string) {
		// Send chunks as Bubble Tea messages via Program.Send
		if m.program != nil {
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

// Ensure imports are used.
var _ = fmt.Sprintf
var _ = context.Background
