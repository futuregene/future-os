package components

import (
	"strings"

	"github.com/charmbracelet/lipgloss"
)

// ─── Diff Renderer ─────────────────────────────────────────────────────────

// DiffLine represents a single line in a unified diff.
type DiffLine struct {
	Type    string // "add", "del", "context", "header"
	Content string
}

// DiffStyle holds styles for diff rendering.
type DiffStyle struct {
	Add     lipgloss.Style
	Del     lipgloss.Style
	Context lipgloss.Style
	Header  lipgloss.Style
}

// DefaultDiffStyle returns the default diff color scheme.
func DefaultDiffStyle() DiffStyle {
	return DiffStyle{
		Add:     lipgloss.NewStyle().Foreground(lipgloss.Color("#a6e3a1")),
		Del:     lipgloss.NewStyle().Foreground(lipgloss.Color("#f38ba8")),
		Context: lipgloss.NewStyle().Foreground(lipgloss.Color("#6c7086")),
		Header:  lipgloss.NewStyle().Foreground(lipgloss.Color("#89b4fa")).Bold(true),
	}
}

// RenderDiff renders a unified diff string with colors.
func RenderDiff(text string, style DiffStyle) string {
	lines := strings.Split(text, "\n")
	var sb strings.Builder

	for _, line := range lines {
		switch {
		case strings.HasPrefix(line, "+++") || strings.HasPrefix(line, "---") ||
			strings.HasPrefix(line, "@@") || strings.HasPrefix(line, "diff "):
			sb.WriteString(style.Header.Render(line))
		case strings.HasPrefix(line, "+"):
			sb.WriteString(style.Add.Render(line))
		case strings.HasPrefix(line, "-"):
			sb.WriteString(style.Del.Render(line))
		default:
			sb.WriteString(style.Context.Render(line))
		}
		sb.WriteByte('\n')
	}
	return sb.String()
}

// ─── ToolOutput Renderer ───────────────────────────────────────────────────

// ToolOutput represents the rendered output of a tool execution.
type ToolOutput struct {
	ToolName string
	Args     string
	Output   string
	IsDiff   bool // true for edit/write tools that produce diffs
	IsError  bool
	Expanded bool
	Duration string
}

// RenderToolOutput renders a tool execution result.
func RenderToolOutput(out ToolOutput, width int) string {
	style := DefaultDiffStyle()

	var sb strings.Builder

	// Header: tool name + args summary + duration
	header := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#e5c07b")).
		Render("🔧 " + out.ToolName)
	if out.Args != "" {
		header += " " + lipgloss.NewStyle().
			Foreground(lipgloss.Color("#6c7086")).
			Render(truncate(out.Args, 60))
	}
	if out.Duration != "" {
		header += " " + lipgloss.NewStyle().
			Foreground(lipgloss.Color("#6c7086")).
			Render("(" + out.Duration + ")")
	}
	sb.WriteString(header)
	sb.WriteByte('\n')

	if !out.Expanded {
		sb.WriteString(lipgloss.NewStyle().
			Foreground(lipgloss.Color("#5c6370")).
			Render("  [expand to view output]"))
		sb.WriteByte('\n')
		return sb.String()
	}

	// Render output content
	output := out.Output
	if out.IsDiff {
		output = RenderDiff(output, style)
	} else if out.IsError {
		output = lipgloss.NewStyle().
			Foreground(lipgloss.Color("#e06c75")).
			Render(output)
	}

	// Indent output
	lines := strings.Split(output, "\n")
	maxLines := 50
	if len(lines) > maxLines {
		lines = lines[:maxLines]
		lines = append(lines, lipgloss.NewStyle().
			Foreground(lipgloss.Color("#e5c07b")).
			Render("  ... truncated ("+itoa(len(strings.Split(out.Output, "\n")))+" lines total)"))
	}
	for _, line := range lines {
		sb.WriteString("  ")
		sb.WriteString(line)
		sb.WriteByte('\n')
	}

	return sb.String()
}

func truncate(s string, max int) string {
	if len(s) <= max {
		return s
	}
	return s[:max-3] + "..."
}
