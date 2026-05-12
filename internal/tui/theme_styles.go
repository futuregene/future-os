package tui

import (
	"github.com/alecthomas/chroma/v2"
	"github.com/charmbracelet/lipgloss"
)

// ─── Derived Styles ────────────────────────────────────────────────────────

// FooterStyle returns the lipgloss style for the footer.
func (t *Theme) FooterStyle() lipgloss.Style {
	return lipgloss.NewStyle().
		Foreground(lipgloss.Color(t.FooterForeground)).
		Padding(0, 1).
		Width(120)
}

// InputStyle returns the lipgloss style for the input area.
// TS pi-mono: borderless single-line inline prompt
func (t *Theme) InputStyle() lipgloss.Style {
	return lipgloss.NewStyle().
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
