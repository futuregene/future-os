package tui

import (
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"strings"
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

	// Markdown colors (TS pi-mono: 10 dedicated slots)
	MdHeading         string `json:"md_heading"`
	MdLink            string `json:"md_link"`
	MdLinkUrl         string `json:"md_link_url"`
	MdCode            string `json:"md_code"`
	MdCodeBlock       string `json:"md_code_block"`
	MdCodeBlockBorder string `json:"md_code_block_border"`
	MdQuote           string `json:"md_quote"`
	MdQuoteBorder     string `json:"md_quote_border"`
	MdHr              string `json:"md_hr"`
	MdListBullet      string `json:"md_list_bullet"`

	// Syntax highlight colors (TS pi-mono: 9 slots)
	SyntaxComment  string `json:"syntax_comment"`
	SyntaxKeyword  string `json:"syntax_keyword"`
	SyntaxFunction string `json:"syntax_function"`
	SyntaxString   string `json:"syntax_string"`
	SyntaxNumber   string `json:"syntax_number"`
	SyntaxType     string `json:"syntax_type"`
	SyntaxVariable string `json:"syntax_variable"`
	SyntaxConstant string `json:"syntax_constant"`
	SyntaxOperator string `json:"syntax_operator"`

	// Additional UI colors
	UserMessageBg   string `json:"user_message_bg"`
	UserMessageText string `json:"user_message_text"`
	ToolTitle       string `json:"tool_title"`
	ToolOutput      string `json:"tool_output"`

	// Theme variables (TS pi-mono: vars section with indirect references)
	Vars map[string]string `json:"vars,omitempty"`
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
	// Apply defaults for missing fields using reflection-based fallback
	d := DefaultTheme()
	fallbackName := strings.TrimSuffix(filepath.Base(path), filepath.Ext(path))
	t.applyDefaults(d, fallbackName)
	return &t, nil
}

// applyDefaults fills zero-value string fields from a default theme.
// Uses reflection to avoid 30+ if-statements per field.
// If fallbackName is non-empty, sets it when Name is empty.
func (t *Theme) applyDefaults(d *Theme, fallbackName string) {
	tv := reflect.ValueOf(t).Elem()
	dv := reflect.ValueOf(d).Elem()
	for i := range tv.NumField() {
		f := tv.Field(i)
		if f.Kind() == reflect.String && f.String() == "" {
			df := dv.Field(i)
			if df.String() != "" {
				f.SetString(df.String())
			}
		}
	}
	if t.Name == "" && fallbackName != "" {
		t.Name = fallbackName
	}
}

// ResolveVar resolves a variable reference like "$primary" or "${primary}"
// from the theme's Vars map. Returns the resolved value, or the original
// string if it's not a variable reference or can't be resolved.
func (t *Theme) ResolveVar(v string) string {
	if t.Vars == nil {
		return v
	}
	// Strip $ prefix and optional ${} wrapping
	key := v
	if strings.HasPrefix(key, "${") && strings.HasSuffix(key, "}") {
		key = key[2 : len(key)-1]
	} else if strings.HasPrefix(key, "$") {
		key = key[1:]
	}
	if resolved, ok := t.Vars[key]; ok {
		return resolved
	}
	return v
}

// GetAllThemes returns names of all known themes: built-in (default, light) + discovered.
func GetAllThemes(dir string) []string {
	names := []string{"default", "light"}
	seen := map[string]bool{"default": true, "light": true}

	paths, err := DiscoverThemes(dir)
	if err != nil {
		return names
	}
	for _, p := range paths {
		t, err := LoadTheme(p)
		if err != nil {
			continue
		}
		name := t.Name
		if name == "" {
			base := filepath.Base(p)
			name = strings.TrimSuffix(base, filepath.Ext(base))
		}
		if !seen[name] {
			names = append(names, name)
			seen[name] = true
		}
	}
	return names
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

