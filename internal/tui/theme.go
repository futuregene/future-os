package tui

import (
	"encoding/json"
	"os"
	"path/filepath"

	"github.com/charmbracelet/lipgloss"
)

// Theme defines the color palette and derived styles for the TUI.
// Aligned with TS pi-mono theme-schema.json color categories.
type Theme struct {
	Name string `json:"name"`

	// Base colors
	Background string `json:"background"`
	Foreground string `json:"foreground"`
	Border     string `json:"border"`
	BorderAccent string `json:"border_accent"`
	BorderMuted  string `json:"border_muted"`
	Accent     string `json:"accent"`

	// Semantic colors
	UserColor      string `json:"user_color"`
	AssistantColor string `json:"assistant_color"`
	ThinkingColor  string `json:"thinking_color"`
	ThinkingText   string `json:"thinking_text"`
	ToolColor      string `json:"tool_color"`
	ToolPendingBg  string `json:"tool_pending_bg"`
	ToolSuccessBg  string `json:"tool_success_bg"`
	ToolErrorBg    string `json:"tool_error_bg"`
	ErrorColor     string `json:"error_color"`
	SystemColor    string `json:"system_color"`
	DiffAddColor   string `json:"diff_add_color"`
	DiffDelColor   string `json:"diff_del_color"`
	Success        string `json:"success"`
	Warning        string `json:"warning"`
	Muted          string `json:"muted"`
	Dim            string `json:"dim"`

	// Selection / message backgrounds
	SelectedBg string `json:"selected_bg"`

	// Context bar colors
	ContextGreen  string `json:"context_green"`
	ContextYellow string `json:"context_yellow"`
	ContextRed    string `json:"context_red"`

	// Footer
	FooterBackground string `json:"footer_background"`
	FooterForeground string `json:"footer_foreground"`

	// Input border modes
	InputBorder string `json:"input_border"`
	BashMode    string `json:"bash_mode"`

	// Per-level thinking border colors (TS pi-mono: thinkingOff..thinkingXhigh)
	ThinkingOff     string `json:"thinking_off"`
	ThinkingMinimal string `json:"thinking_minimal"`
	ThinkingLow     string `json:"thinking_low"`
	ThinkingMedium  string `json:"thinking_medium"`
	ThinkingHigh    string `json:"thinking_high"`
	ThinkingXhigh   string `json:"thinking_xhigh"`
}

// DefaultTheme returns the built-in dark theme.
func DefaultTheme() *Theme {
	return &Theme{
		Name:             "dark",
		Background:       "#1e1e2e",
		Foreground:       "#cdd6f4",
		Border:           "#45475a",
		BorderAccent:     "#89b4fa",
		BorderMuted:      "#313244",
		Accent:           "#89b4fa",
		UserColor:        "#89b4fa",
		AssistantColor:   "#cdd6f4",
		ThinkingColor:    "#6c7086",
		ThinkingText:     "#a6adc8",
		ToolColor:        "#f9e2af",
		ToolPendingBg:    "#282832",
		ToolSuccessBg:    "#283228",
		ToolErrorBg:      "#3c2828",
		ErrorColor:       "#f38ba8",
		SystemColor:      "#a6e3a1",
		DiffAddColor:     "#a6e3a1",
		DiffDelColor:     "#f38ba8",
		Success:          "#a6e3a1",
		Warning:          "#f9e2af",
		Muted:            "#a6adc8",
		Dim:              "#585b70",
		SelectedBg:       "#45475a",
		ContextGreen:     "#a6e3a1",
		ContextYellow:    "#f9e2af",
		ContextRed:       "#f38ba8",
		FooterBackground: "#181825",
		FooterForeground: "#a6adc8",
		InputBorder:      "#89b4fa",
		BashMode:         "#f9e2af",
		ThinkingOff:      "#5c6370",
		ThinkingMinimal:  "#6e6e6e",
		ThinkingLow:      "#5f87af",
		ThinkingMedium:   "#81a2be",
		ThinkingHigh:     "#b294bb",
		ThinkingXhigh:    "#d183e8",
	}
}

// LightTheme returns the built-in light theme.
func LightTheme() *Theme {
	return &Theme{
		Name:             "light",
		Background:       "#eff1f5",
		Foreground:       "#4c4f69",
		Border:           "#ccd0da",
		BorderAccent:     "#1e66f5",
		BorderMuted:      "#e6e9ef",
		Accent:           "#1e66f5",
		UserColor:        "#1e66f5",
		AssistantColor:   "#4c4f69",
		ThinkingColor:    "#9ca0b0",
		ThinkingText:     "#6c6f85",
		ToolColor:        "#df8e1d",
		ToolPendingBg:    "#e6e9ef",
		ToolSuccessBg:    "#dcedd6",
		ToolErrorBg:      "#f0d6dc",
		ErrorColor:       "#d20f39",
		SystemColor:      "#40a02b",
		DiffAddColor:     "#40a02b",
		DiffDelColor:     "#d20f39",
		Success:          "#40a02b",
		Warning:          "#df8e1d",
		Muted:            "#8c8fa1",
		Dim:              "#bcc0cc",
		SelectedBg:       "#ccd0da",
		ContextGreen:     "#40a02b",
		ContextYellow:    "#df8e1d",
		ContextRed:       "#d20f39",
		FooterBackground: "#dce0e8",
		FooterForeground: "#5c5f77",
		InputBorder:      "#1e66f5",
		BashMode:         "#df8e1d",
		ThinkingOff:      "#9ca0b0",
		ThinkingMinimal:  "#8c8fa1",
		ThinkingLow:      "#1e66f5",
		ThinkingMedium:   "#6c6f85",
		ThinkingHigh:     "#7c3aed",
		ThinkingXhigh:    "#d946ef",
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
	if t.ToolPendingBg == "" {
		t.ToolPendingBg = d.ToolPendingBg
	}
	if t.ToolSuccessBg == "" {
		t.ToolSuccessBg = d.ToolSuccessBg
	}
	if t.ToolErrorBg == "" {
		t.ToolErrorBg = d.ToolErrorBg
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
	if t.BorderAccent == "" {
		t.BorderAccent = d.BorderAccent
	}
	if t.BorderMuted == "" {
		t.BorderMuted = d.BorderMuted
	}
	if t.ThinkingText == "" {
		t.ThinkingText = d.ThinkingText
	}
	if t.Success == "" {
		t.Success = d.Success
	}
	if t.Warning == "" {
		t.Warning = d.Warning
	}
	if t.Muted == "" {
		t.Muted = d.Muted
	}
	if t.Dim == "" {
		t.Dim = d.Dim
	}
	if t.SelectedBg == "" {
		t.SelectedBg = d.SelectedBg
	}
	if t.BashMode == "" {
		t.BashMode = d.BashMode
	}
	if t.ThinkingOff == "" {
		t.ThinkingOff = d.ThinkingOff
	}
	if t.ThinkingMinimal == "" {
		t.ThinkingMinimal = d.ThinkingMinimal
	}
	if t.ThinkingLow == "" {
		t.ThinkingLow = d.ThinkingLow
	}
	if t.ThinkingMedium == "" {
		t.ThinkingMedium = d.ThinkingMedium
	}
	if t.ThinkingHigh == "" {
		t.ThinkingHigh = d.ThinkingHigh
	}
	if t.ThinkingXhigh == "" {
		t.ThinkingXhigh = d.ThinkingXhigh
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
		dir = filepath.Join(home, ".xihu", "themes")
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

// ThinkingBorderColor returns the editor border color for a thinking level.
func (t *Theme) ThinkingBorderColor(level string) string {
	switch level {
	case "off":
		return t.ThinkingOff
	case "minimal":
		return t.ThinkingMinimal
	case "low":
		return t.ThinkingLow
	case "medium":
		return t.ThinkingMedium
	case "high":
		return t.ThinkingHigh
	case "xhigh":
		return t.ThinkingXhigh
	default:
		return t.Accent
	}
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
