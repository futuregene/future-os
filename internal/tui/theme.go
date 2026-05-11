package tui

import (
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"strings"

	"github.com/alecthomas/chroma/v2"
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
		// Markdown colors
		MdHeading:         "#cba6f7",
		MdLink:            "#89b4fa",
		MdLinkUrl:         "#a6adc8",
		MdCode:            "#f9e2af",
		MdCodeBlock:       "#45475a",
		MdCodeBlockBorder: "#585b70",
		MdQuote:           "#a6adc8",
		MdQuoteBorder:     "#585b70",
		MdHr:              "#585b70",
		MdListBullet:      "#89b4fa",
		// Syntax colors
		SyntaxComment:  "#6c7086",
		SyntaxKeyword:  "#cba6f7",
		SyntaxFunction: "#89b4fa",
		SyntaxString:   "#a6e3a1",
		SyntaxNumber:   "#fab387",
		SyntaxType:     "#f9e2af",
		SyntaxVariable: "#cdd6f4",
		SyntaxConstant: "#fab387",
		SyntaxOperator: "#89dceb",
		// Additional UI
		UserMessageBg:   "#313244",
		UserMessageText: "#cdd6f4",
		ToolTitle:       "#f9e2af",
		ToolOutput:      "#a6adc8",
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
		// Markdown colors
		MdHeading:         "#8839ef",
		MdLink:            "#1e66f5",
		MdLinkUrl:         "#6c6f85",
		MdCode:            "#fe640b",
		MdCodeBlock:       "#ccd0da",
		MdCodeBlockBorder: "#bcc0cc",
		MdQuote:           "#8c8fa1",
		MdQuoteBorder:     "#bcc0cc",
		MdHr:              "#bcc0cc",
		MdListBullet:      "#1e66f5",
		// Syntax colors
		SyntaxComment:  "#9ca0b0",
		SyntaxKeyword:  "#8839ef",
		SyntaxFunction: "#1e66f5",
		SyntaxString:   "#40a02b",
		SyntaxNumber:   "#fe640b",
		SyntaxType:     "#df8e1d",
		SyntaxVariable: "#4c4f69",
		SyntaxConstant: "#fe640b",
		SyntaxOperator: "#04a5e5",
		// Additional UI
		UserMessageBg:   "#dce0e8",
		UserMessageText: "#4c4f69",
		ToolTitle:       "#df8e1d",
		ToolOutput:      "#6c6f85",
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

// ─── Markdown Styles ───────────────────────────────────────────────────────

// MdHeadingStyle returns the style for markdown headings.
func (t *Theme) MdHeadingStyle() lipgloss.Style {
	return lipgloss.NewStyle().
		Foreground(lipgloss.Color(t.MdHeading)).
		Bold(true)
}

// MdLinkStyle returns the style for markdown links.
func (t *Theme) MdLinkStyle() lipgloss.Style {
	return lipgloss.NewStyle().
		Foreground(lipgloss.Color(t.MdLink)).
		Underline(true)
}

// MdLinkUrlStyle returns the style for markdown link URLs.
func (t *Theme) MdLinkUrlStyle() lipgloss.Style {
	return lipgloss.NewStyle().
		Foreground(lipgloss.Color(t.MdLinkUrl))
}

// MdCodeStyle returns the style for inline code.
func (t *Theme) MdCodeStyle() lipgloss.Style {
	return lipgloss.NewStyle().
		Foreground(lipgloss.Color(t.MdCode))
}

// MdCodeBlockStyle returns the style for code block backgrounds.
func (t *Theme) MdCodeBlockStyle() lipgloss.Style {
	return lipgloss.NewStyle().
		Background(lipgloss.Color(t.MdCodeBlock))
}

// MdCodeBlockBorderStyle returns the style for code block borders.
func (t *Theme) MdCodeBlockBorderStyle() lipgloss.Style {
	return lipgloss.NewStyle().
		Foreground(lipgloss.Color(t.MdCodeBlockBorder))
}

// MdQuoteStyle returns the style for markdown blockquotes.
func (t *Theme) MdQuoteStyle() lipgloss.Style {
	return lipgloss.NewStyle().
		Foreground(lipgloss.Color(t.MdQuote)).
		Italic(true)
}

// MdQuoteBorderStyle returns the style for blockquote borders.
func (t *Theme) MdQuoteBorderStyle() lipgloss.Style {
	return lipgloss.NewStyle().
		Foreground(lipgloss.Color(t.MdQuoteBorder))
}

// MdHrStyle returns the style for horizontal rules.
func (t *Theme) MdHrStyle() lipgloss.Style {
	return lipgloss.NewStyle().
		Foreground(lipgloss.Color(t.MdHr))
}

// MdListBulletStyle returns the style for list bullets.
func (t *Theme) MdListBulletStyle() lipgloss.Style {
	return lipgloss.NewStyle().
		Foreground(lipgloss.Color(t.MdListBullet))
}

// ─── Syntax Highlight Styles ───────────────────────────────────────────────

// SyntaxCommentStyle returns the style for code comments.
func (t *Theme) SyntaxCommentStyle() lipgloss.Style {
	return lipgloss.NewStyle().
		Foreground(lipgloss.Color(t.SyntaxComment)).
		Italic(true)
}

// SyntaxKeywordStyle returns the style for syntax keywords.
func (t *Theme) SyntaxKeywordStyle() lipgloss.Style {
	return lipgloss.NewStyle().
		Foreground(lipgloss.Color(t.SyntaxKeyword)).
		Bold(true)
}

// SyntaxFunctionStyle returns the style for syntax function names.
func (t *Theme) SyntaxFunctionStyle() lipgloss.Style {
	return lipgloss.NewStyle().
		Foreground(lipgloss.Color(t.SyntaxFunction))
}

// SyntaxStringStyle returns the style for syntax strings.
func (t *Theme) SyntaxStringStyle() lipgloss.Style {
	return lipgloss.NewStyle().
		Foreground(lipgloss.Color(t.SyntaxString))
}

// SyntaxNumberStyle returns the style for syntax numbers.
func (t *Theme) SyntaxNumberStyle() lipgloss.Style {
	return lipgloss.NewStyle().
		Foreground(lipgloss.Color(t.SyntaxNumber))
}

// SyntaxTypeStyle returns the style for syntax types.
func (t *Theme) SyntaxTypeStyle() lipgloss.Style {
	return lipgloss.NewStyle().
		Foreground(lipgloss.Color(t.SyntaxType))
}

// SyntaxVariableStyle returns the style for syntax variables.
func (t *Theme) SyntaxVariableStyle() lipgloss.Style {
	return lipgloss.NewStyle().
		Foreground(lipgloss.Color(t.SyntaxVariable))
}

// SyntaxConstantStyle returns the style for syntax constants.
func (t *Theme) SyntaxConstantStyle() lipgloss.Style {
	return lipgloss.NewStyle().
		Foreground(lipgloss.Color(t.SyntaxConstant))
}

// SyntaxOperatorStyle returns the style for syntax operators.
func (t *Theme) SyntaxOperatorStyle() lipgloss.Style {
	return lipgloss.NewStyle().
		Foreground(lipgloss.Color(t.SyntaxOperator))
}

// ─── Additional UI Styles ──────────────────────────────────────────────────

// UserMessageBgStyle returns the background style for user messages.
func (t *Theme) UserMessageBgStyle() lipgloss.Style {
	return lipgloss.NewStyle().
		Background(lipgloss.Color(t.UserMessageBg))
}

// UserMessageTextStyle returns the text style for user messages.
func (t *Theme) UserMessageTextStyle() lipgloss.Style {
	return lipgloss.NewStyle().
		Foreground(lipgloss.Color(t.UserMessageText))
}

// ToolTitleStyle returns the style for tool call titles.
func (t *Theme) ToolTitleStyle() lipgloss.Style {
	return lipgloss.NewStyle().
		Foreground(lipgloss.Color(t.ToolTitle)).
		Bold(true)
}

// ToolOutputStyle returns the style for tool output text.
func (t *Theme) ToolOutputStyle() lipgloss.Style {
	return lipgloss.NewStyle().
		Foreground(lipgloss.Color(t.ToolOutput))
}

// ─── Chroma Style Generation ──────────────────────────────────────────────

// BuildChromaStyle creates a chroma.Style from the Theme's syntax highlight colors.
// This is available for custom renderers or future glamour integration.
// Currently glamour uses its built-in dark/light styles with "terminal16m" formatter;
// the Theme syntax slots are available for a future custom markdown renderer.
func (t *Theme) BuildChromaStyle() *chroma.Style {
	entries := chroma.StyleEntries{
		chroma.Comment:         t.SyntaxComment,
		chroma.CommentPreproc:  t.SyntaxComment,
		chroma.Keyword:         t.SyntaxKeyword,
		chroma.KeywordReserved: t.SyntaxKeyword,
		chroma.KeywordNamespace: t.SyntaxKeyword,
		chroma.KeywordType:     t.SyntaxType,
		chroma.Operator:        t.SyntaxOperator,
		chroma.Punctuation:     t.Muted,
		chroma.Name:            t.Foreground,
		chroma.NameBuiltin:     t.SyntaxFunction,
		chroma.NameTag:         t.SyntaxFunction,
		chroma.NameAttribute:   t.SyntaxVariable,
		chroma.NameClass:       t.SyntaxType,
		chroma.NameFunction:    t.SyntaxFunction,
		chroma.NameDecorator:   t.SyntaxFunction,
		chroma.LiteralNumber:   t.SyntaxNumber,
		chroma.LiteralString:   t.SyntaxString,
		chroma.LiteralStringEscape: t.SyntaxString,
		chroma.GenericDeleted:  t.ErrorColor,
		chroma.GenericInserted: t.Success,
		chroma.Background:      "bg:" + t.MdCodeBlock,
	}
	style, err := chroma.NewStyle("xihu-"+t.Name, entries)
	if err != nil {
		return nil
	}
	return style
}
