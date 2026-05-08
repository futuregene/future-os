package components

import (
	"strings"

	"github.com/charmbracelet/lipgloss"
)

// Header displays startup information: changelog, version, resource status.
type Header struct {
	visible     bool
	changelog   string   // latest changelog entries
	version     string   // current version
	newVersion  string   // new version available
	resources   []ResourceStatus
	diagnostics []string
	expanded    bool
}

// ResourceStatus tracks a loaded resource.
type ResourceStatus struct {
	Type   string // "skill", "prompt", "extension", "theme"
	Name   string
	Count  int
	Error  string
}

// NewHeader creates a new header component.
func NewHeader() Header {
	return Header{
		visible:  true,
		expanded: false,
	}
}

// SetChangelog sets the changelog content.
func (h *Header) SetChangelog(text string) {
	h.changelog = text
}

// SetVersion sets version info.
func (h *Header) SetVersion(current, available string) {
	h.version = current
	h.newVersion = available
}

// AddResource adds a loaded resource.
func (h *Header) AddResource(typ, name string, count int, err string) {
	h.resources = append(h.resources, ResourceStatus{Type: typ, Name: name, Count: count, Error: err})
}

// AddDiagnostic adds a diagnostic message.
func (h *Header) AddDiagnostic(msg string) {
	h.diagnostics = append(h.diagnostics, msg)
}

// Toggle expands/collapses the header.
func (h *Header) Toggle() {
	h.expanded = !h.expanded
}

// View renders the header.
func (h Header) View() string {
	if !h.visible {
		return ""
	}

	style := lipgloss.NewStyle().
		Background(lipgloss.Color("#21252b")).
		Padding(1, 2)

	var sb strings.Builder

	// Version line
	if h.version != "" {
		sb.WriteString(lipgloss.NewStyle().
			Foreground(lipgloss.Color("#61afef")).Bold(true).
			Render("cobalt " + h.version))
		if h.newVersion != "" {
			sb.WriteString("  " + lipgloss.NewStyle().
				Foreground(lipgloss.Color("#e5c07b")).
				Render("(update available: "+h.newVersion+")"))
		}
		sb.WriteByte('\n')
	}

	if !h.expanded {
		sb.WriteString(lipgloss.NewStyle().
			Foreground(lipgloss.Color("#6c7086")).
			Render("Press Enter to expand…"))
		return style.Render(sb.String())
	}

	// Changelog
	if h.changelog != "" {
		sb.WriteString(lipgloss.NewStyle().
			Foreground(lipgloss.Color("#abb2bf")).
			Render("Changelog:"))
		sb.WriteByte('\n')
		for _, line := range strings.Split(h.changelog, "\n") {
			if line == "" {
				continue
			}
			sb.WriteString("  " + lipgloss.NewStyle().
				Foreground(lipgloss.Color("#98c379")).
				Render("• "+strings.TrimSpace(line)))
			sb.WriteByte('\n')
		}
	}

	// Resources
	if len(h.resources) > 0 {
		sb.WriteString(lipgloss.NewStyle().
			Foreground(lipgloss.Color("#abb2bf")).
			Render("Loaded:"))
		sb.WriteByte('\n')
		for _, r := range h.resources {
			icon := "✓"
			color := lipgloss.Color("#98c379")
			if r.Error != "" {
				icon = "✗"
				color = lipgloss.Color("#e06c75")
			}
			sb.WriteString("  " + lipgloss.NewStyle().Foreground(color).Render(
				icon+" "+r.Type+": "+r.Name))
			if r.Count > 1 {
				sb.WriteString(" (x" + itoa(r.Count) + ")")
			}
			if r.Error != "" {
				sb.WriteString(" — " + r.Error)
			}
			sb.WriteByte('\n')
		}
	}

	// Diagnostics
	if len(h.diagnostics) > 0 {
		for _, d := range h.diagnostics {
			sb.WriteString(lipgloss.NewStyle().
				Foreground(lipgloss.Color("#e5c07b")).
				Render("⚠ " + d))
			sb.WriteByte('\n')
		}
	}

	return style.Render(sb.String())
}
