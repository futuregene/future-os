package components

import (
	"time"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

// ─── BranchSummaryMessage ────────────────────────────────────────────────

// BranchSummaryMsg is sent to control the BranchSummaryMessage component.
type BranchSummaryMsg struct {
	// Action: "set", "clear"
	Action     string
	Summary    string
	BranchName string
}

// BranchSummaryMessage is a tea.Model that renders a branch summary entry
// in a bordered box with a "Branch: {name}" header and summary content.
// It uses a muted/blue tone distinct from regular messages.
type BranchSummaryMessage struct {
	summary    string
	branchName string
	timestamp  time.Time
	active     bool

	// styles
	borderColor lipgloss.Color
	headerColor lipgloss.Color
	textColor   lipgloss.Color
	mutedColor  lipgloss.Color
}

// NewBranchSummaryMessage creates a new BranchSummaryMessage with default styles.
func NewBranchSummaryMessage() BranchSummaryMessage {
	return BranchSummaryMessage{
		borderColor: "#313244",
		headerColor: "#89b4fa",
		textColor:   "#a6adc8",
		mutedColor:  "#585b70",
	}
}

// SetBorderColor sets the border color (hex string).
func (b *BranchSummaryMessage) SetBorderColor(hex string) {
	b.borderColor = lipgloss.Color(hex)
}

// SetHeaderColor sets the header text color (hex string).
func (b *BranchSummaryMessage) SetHeaderColor(hex string) {
	b.headerColor = lipgloss.Color(hex)
}

// SetTextColor sets the body text color (hex string).
func (b *BranchSummaryMessage) SetTextColor(hex string) {
	b.textColor = lipgloss.Color(hex)
}

// SetMutedColor sets the muted/timestamp color (hex string).
func (b *BranchSummaryMessage) SetMutedColor(hex string) {
	b.mutedColor = lipgloss.Color(hex)
}

// SetSummary activates the component with a summary and branch name.
func (b *BranchSummaryMessage) SetSummary(summary, branchName string) {
	b.summary = summary
	b.branchName = branchName
	b.timestamp = time.Now()
	b.active = true
}

// Clear deactivates the component.
func (b *BranchSummaryMessage) Clear() {
	b.active = false
	b.summary = ""
	b.branchName = ""
	b.timestamp = time.Time{}
}

// Init is a no-op for this component.
func (b BranchSummaryMessage) Init() tea.Cmd {
	return nil
}

// Update handles BranchSummaryMsg control messages.
func (b BranchSummaryMessage) Update(msg tea.Msg) (BranchSummaryMessage, tea.Cmd) {
	switch m := msg.(type) {
	case BranchSummaryMsg:
		switch m.Action {
		case "set":
			b.SetSummary(m.Summary, m.BranchName)
		case "clear":
			b.Clear()
		}
	}
	return b, nil
}

// View renders the branch summary in a bordered box.
func (b BranchSummaryMessage) View() string {
	if !b.active {
		return ""
	}

	headerStyle := lipgloss.NewStyle().
		Foreground(b.headerColor).
		Bold(true)

	mutedStyle := lipgloss.NewStyle().
		Foreground(b.mutedColor)

	bodyStyle := lipgloss.NewStyle().
		Foreground(b.textColor)

	// Build header line: "Branch: {name}  •  {timestamp}"
	header := headerStyle.Render("Branch: " + b.branchName)
	if !b.timestamp.IsZero() {
		ts := b.timestamp.Format("15:04:05")
		header += "  " + mutedStyle.Render("• "+ts)
	}

	content := header
	if b.summary != "" {
		content += "\n" + bodyStyle.Render(b.summary)
	}

	boxStyle := lipgloss.NewStyle().
		Border(lipgloss.RoundedBorder()).
		BorderForeground(b.borderColor).
		Padding(1, 2)

	return boxStyle.Render(content)
}
