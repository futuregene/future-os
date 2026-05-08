package tui

import (
	"encoding/json"
	"os"
	"path/filepath"

	"github.com/charmbracelet/lipgloss"
)

// Theme defines the color palette and derived styles for the TUI.
type Theme struct {
	Name string `json:"name"`

	// Base colors
	Background string `json:"background"`
	Foreground string `json:"foreground"`
	Border     string `json:"border"`
	Accent     string `json:"accent"`

	// Semantic colors
	UserColor      string `json:"user_color"`
	AssistantColor string `json:"assistant_color"`
	ThinkingColor  string `json:"thinking_color"`
	ToolColor      string `json:"tool_color"`
	ErrorColor     string `json:"error_color"`
	SystemColor    string `json:"system_color"`
	DiffAddColor   string `json:"diff_add_color"`
	DiffDelColor   string `json:"diff_del_color"`

	// Context bar colors
	ContextGreen  string `json:"context_green"`
	ContextYellow string `json:"context_yellow"`
	ContextRed    string `json:"context_red"`

	// Footer
	FooterBackground string `json:"footer_background"`
	FooterForeground string `json:"footer_foreground"`

	// Input
	InputBorder string `json:"input_border"`
}

// DefaultTheme returns the built-in dark theme.
func DefaultTheme() *Theme {
	return &Theme{
		Name:            "dark",
		Background:      "#1e1e2e",
		Foreground:      "#cdd6f4",
		Border:          "#45475a",
		Accent:          "#89b4fa",
		UserColor:       "#89b4fa",
		AssistantColor:  "#cdd6f4",
		ThinkingColor:   "#6c7086",
		ToolColor:       "#f9e2af",
		ErrorColor:      "#f38ba8",
		SystemColor:     "#a6e3a1",
		DiffAddColor:    "#a6e3a1",
		DiffDelColor:    "#f38ba8",
		ContextGreen:    "#a6e3a1",
		ContextYellow:   "#f9e2af",
		ContextRed:      "#f38ba8",
		FooterBackground: "#181825",
		FooterForeground: "#a6adc8",
		InputBorder:     "#89b4fa",
	}
}

// LightTheme returns the built-in light theme.
func LightTheme() *Theme {
	return &Theme{
		Name:             "light",
		Background:       "#eff1f5",
		Foreground:       "#4c4f69",
		Border:           "#ccd0da",
		Accent:           "#1e66f5",
		UserColor:        "#1e66f5",
		AssistantColor:   "#4c4f69",
		ThinkingColor:    "#9ca0b0",
		ToolColor:        "#df8e1d",
		ErrorColor:       "#d20f39",
		SystemColor:      "#40a02b",
		DiffAddColor:     "#40a02b",
		DiffDelColor:     "#d20f39",
		ContextGreen:     "#40a02b",
		ContextYellow:    "#df8e1d",
		ContextRed:       "#d20f39",
		FooterBackground: "#dce0e8",
		FooterForeground: "#5c5f77",
		InputBorder:      "#1e66f5",
	}
}

// LoadTheme loads a theme from a JSON file.
func LoadTheme(path string) (*Theme, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	var t Theme
	if err := json.Unmarshal(data, &t); err != nil {
		return nil, err
	}
	// Apply defaults for missing fields
	d := DefaultTheme()
	if t.Background == "" {
		t.Background = d.Background
	}
	if t.Foreground == "" {
		t.Foreground = d.Foreground
	}
	if t.Border == "" {
		t.Border = d.Border
	}
	if t.Accent == "" {
		t.Accent = d.Accent
	}
	if t.UserColor == "" {
		t.UserColor = d.UserColor
	}
	if t.AssistantColor == "" {
		t.AssistantColor = d.AssistantColor
	}
	if t.ThinkingColor == "" {
		t.ThinkingColor = d.ThinkingColor
	}
	if t.ToolColor == "" {
		t.ToolColor = d.ToolColor
	}
	if t.ErrorColor == "" {
		t.ErrorColor = d.ErrorColor
	}
	if t.SystemColor == "" {
		t.SystemColor = d.SystemColor
	}
	if t.DiffAddColor == "" {
		t.DiffAddColor = d.DiffAddColor
	}
	if t.DiffDelColor == "" {
		t.DiffDelColor = d.DiffDelColor
	}
	if t.ContextGreen == "" {
		t.ContextGreen = d.ContextGreen
	}
	if t.ContextYellow == "" {
		t.ContextYellow = d.ContextYellow
	}
	if t.ContextRed == "" {
		t.ContextRed = d.ContextRed
	}
	if t.FooterBackground == "" {
		t.FooterBackground = d.FooterBackground
	}
	if t.FooterForeground == "" {
		t.FooterForeground = d.FooterForeground
	}
	if t.InputBorder == "" {
		t.InputBorder = d.InputBorder
	}
	return &t, nil
}

// DiscoverThemes scans the themes directory for .json files.
func DiscoverThemes(dir string) ([]string, error) {
	if dir == "" {
		home, err := os.UserHomeDir()
		if err != nil {
			return nil, err
		}
		dir = filepath.Join(home, ".pi", "themes")
	}

	entries, err := os.ReadDir(dir)
	if err != nil {
		return nil, nil // directory not found is not an error
	}

	var themes []string
	for _, e := range entries {
		if !e.IsDir() && filepath.Ext(e.Name()) == ".json" {
			themes = append(themes, filepath.Join(dir, e.Name()))
		}
	}
	return themes, nil
}

// ─── Derived Styles ────────────────────────────────────────────────────────

// FooterStyle returns the lipgloss style for the footer.
func (t *Theme) FooterStyle() lipgloss.Style {
	return lipgloss.NewStyle().
		Background(lipgloss.Color(t.FooterBackground)).
		Foreground(lipgloss.Color(t.FooterForeground)).
		Padding(0, 1).
		Width(120)
}

// InputStyle returns the lipgloss style for the input area.
func (t *Theme) InputStyle() lipgloss.Style {
	return lipgloss.NewStyle().
		Border(lipgloss.NormalBorder(), true).
		BorderForeground(lipgloss.Color(t.InputBorder)).
		Padding(0, 1)
}

// AppStyle returns the base app style.
func (t *Theme) AppStyle() lipgloss.Style {
	return lipgloss.NewStyle().
		Background(lipgloss.Color(t.Background)).
		Foreground(lipgloss.Color(t.Foreground))
}

// UserStyle returns the style for user messages.
func (t *Theme) UserStyle() lipgloss.Style {
	return lipgloss.NewStyle().
		Foreground(lipgloss.Color(t.UserColor))
}

// AssistantStyle returns the style for assistant messages.
func (t *Theme) AssistantStyle() lipgloss.Style {
	return lipgloss.NewStyle().
		Foreground(lipgloss.Color(t.AssistantColor))
}

// ThinkingStyle returns the style for thinking blocks.
func (t *Theme) ThinkingStyle() lipgloss.Style {
	return lipgloss.NewStyle().
		Foreground(lipgloss.Color(t.ThinkingColor)).
		Italic(true)
}

// ToolStyle returns the style for tool calls.
func (t *Theme) ToolStyle() lipgloss.Style {
	return lipgloss.NewStyle().
		Foreground(lipgloss.Color(t.ToolColor))
}

// ErrorStyle returns the style for errors.
func (t *Theme) ErrorStyle() lipgloss.Style {
	return lipgloss.NewStyle().
		Foreground(lipgloss.Color(t.ErrorColor))
}

// SystemStyle returns the style for system messages.
func (t *Theme) SystemStyle() lipgloss.Style {
	return lipgloss.NewStyle().
		Foreground(lipgloss.Color(t.SystemColor))
}
