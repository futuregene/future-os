package components

import (
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

// ─── SkillInvocationMessage ──────────────────────────────────────────────

// SkillMsg is sent to control the SkillInvocationMessage component.
type SkillMsg struct {
	// Action: "set", "clear"
	Action      string
	SkillName   string
	Description string
}

// SkillInvocationMessage is a tea.Model that renders a skill invocation display.
// It shows a loading line while the skill is being invoked, then the completed
// description once the skill has loaded.
type SkillInvocationMessage struct {
	skillName   string
	description string
	active      bool

	// Whether the skill has completed loading (description is final).
	completed bool

	// styles
	accentColor lipgloss.Color
	textColor   lipgloss.Color
	mutedColor  lipgloss.Color
}

// NewSkillInvocationMessage creates a new SkillInvocationMessage with default styles.
func NewSkillInvocationMessage() SkillInvocationMessage {
	return SkillInvocationMessage{
		accentColor: "#89b4fa",
		textColor:   "#cdd6f4",
		mutedColor:  "#6c7086",
	}
}

// SetAccentColor sets the accent color (hex string).
func (s *SkillInvocationMessage) SetAccentColor(hex string) {
	s.accentColor = lipgloss.Color(hex)
}

// SetTextColor sets the text color (hex string).
func (s *SkillInvocationMessage) SetTextColor(hex string) {
	s.textColor = lipgloss.Color(hex)
}

// SetMutedColor sets the muted/description color (hex string).
func (s *SkillInvocationMessage) SetMutedColor(hex string) {
	s.mutedColor = lipgloss.Color(hex)
}

// SetSkill activates the component with a skill name and description.
// If description is non-empty, the skill is treated as completed.
func (s *SkillInvocationMessage) SetSkill(name, description string) {
	s.skillName = name
	s.description = description
	s.active = true
	s.completed = description != ""
}

// Clear deactivates the component.
func (s *SkillInvocationMessage) Clear() {
	s.active = false
	s.skillName = ""
	s.description = ""
	s.completed = false
}

// Init is a no-op for this component.
func (s SkillInvocationMessage) Init() tea.Cmd {
	return nil
}

// Update handles SkillMsg control messages.
func (s SkillInvocationMessage) Update(msg tea.Msg) (SkillInvocationMessage, tea.Cmd) {
	switch m := msg.(type) {
	case SkillMsg:
		switch m.Action {
		case "set":
			s.SetSkill(m.SkillName, m.Description)
		case "clear":
			s.Clear()
		}
	}
	return s, nil
}

// View renders the skill invocation display.
func (s SkillInvocationMessage) View() string {
	if !s.active {
		return ""
	}

	accent := lipgloss.NewStyle().
		Foreground(s.accentColor).
		Bold(true)

	italic := lipgloss.NewStyle().
		Foreground(s.accentColor).
		Italic(true)

	muted := lipgloss.NewStyle().
		Foreground(s.mutedColor).
		Italic(true)

	if s.completed {
		// Completed skill: show the description
		header := accent.Render("🔧[skill] ") + italic.Render(s.skillName)
		return header + "\n" + muted.Render("    "+s.description)
	}

	// Loading skill
	return accent.Render("🔧[skill] ") + italic.Render("Loading skill: "+s.skillName+"...")
}
