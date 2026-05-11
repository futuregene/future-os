package components

import (
	"strings"

	"github.com/charmbracelet/lipgloss"
)

// CustomMessageComponent renders "custom_message" type chat entries.
// Extracted from ChatViewport.renderCustomMessageEntry() matching TS pi-mono's
// CompactionSummaryMessageComponent, BranchSummaryMessageComponent,
// SkillInvocationMessageComponent, and generic CustomMessageComponent patterns.
type CustomMessageComponent struct {
	base *MessageComponentBase
}

// NewCustomMessageComponent creates a new custom message component.
func NewCustomMessageComponent(base *MessageComponentBase) *CustomMessageComponent {
	return &CustomMessageComponent{base: base}
}

// toggleKey returns the tool toggle key string with fallback.
func (c *CustomMessageComponent) toggleKey() string {
	if c.base.ToolToggleKey != nil && *c.base.ToolToggleKey != "" {
		return *c.base.ToolToggleKey
	}
	return "Ctrl+O"
}

// Render renders a custom message entry.
// Handles CustomType: "compaction", "branch", "skill", and generic fallback.
// Compaction: token count display + markdown content when expanded.
// Branch: branch summary markdown.
// Skill: name + markdown content.
// Generic: first line preview + expand hint.
func (c *CustomMessageComponent) Render(entry ChatEntry, width int) string {
	labelOnBg := c.base.CustomMessageBg.Copy().Inherit(c.base.CustomMessageLabel)
	textOnBg := c.base.CustomMessageBg.Copy()
	dimOnBg := c.base.CustomMessageBg.Copy().Foreground(lipgloss.Color("#5c6370"))

	label := labelOnBg.Render("[" + entry.CustomType + "]")
	toggleKey := c.toggleKey()

	switch entry.CustomType {
	case "compaction":
		return c.renderCompaction(entry, width, label, textOnBg, dimOnBg, toggleKey)
	case "branch":
		return c.renderBranch(entry, width, label, textOnBg, dimOnBg, toggleKey)
	case "skill":
		return c.renderSkill(entry, width, label, textOnBg, dimOnBg, toggleKey)
	default:
		return c.renderGeneric(entry, width, label, textOnBg, dimOnBg, toggleKey)
	}
}

// renderCompaction renders a compaction summary (TS pi-mono: CompactionSummaryMessageComponent).
func (c *CustomMessageComponent) renderCompaction(entry ChatEntry, width int, label string, textOnBg, dimOnBg lipgloss.Style, toggleKey string) string {
	tokenStr := formatTokenCount(entry.TokensBefore)

	if entry.Expanded && entry.Content != "" {
		labelLine := padLineToWidth(label, width, c.base.CustomMessageBg)
		spacerLine := padLineToWidth(textOnBg.Render(""), width, c.base.CustomMessageBg)
		mdContent := "**Compacted from " + tokenStr + " tokens**\n\n" + entry.Content
		rendered, err := c.base.MdRenderer.Render(mdContent)
		if err != nil {
			rendered = wordWrap(mdContent, width-10)
		}
		rendered = strings.TrimSuffix(rendered, "\n")
		return labelLine + "\n" + spacerLine + "\n" + prefixedLineBg(" ", wrapURLsOSC8(rendered), width, c.base.CustomMessageBg)
	}

	msg := textOnBg.Render("Compacted from " + tokenStr + " tokens (")
	keyHint := dimOnBg.Render(toggleKey)
	suffix := textOnBg.Render(" to expand)")
	labelLine := padLineToWidth(label, width, c.base.CustomMessageBg)
	contentLine := padLineToWidth(textOnBg.Render(" "+msg+keyHint+suffix), width, c.base.CustomMessageBg)
	return labelLine + "\n" + contentLine
}

// renderBranch renders a branch summary (TS pi-mono: BranchSummaryMessageComponent).
func (c *CustomMessageComponent) renderBranch(entry ChatEntry, width int, label string, textOnBg, dimOnBg lipgloss.Style, toggleKey string) string {
	if entry.Expanded && entry.Content != "" {
		labelLine := padLineToWidth(label, width, c.base.CustomMessageBg)
		spacerLine := padLineToWidth(textOnBg.Render(""), width, c.base.CustomMessageBg)
		mdContent := "**Branch Summary**\n\n" + entry.Content
		rendered, err := c.base.MdRenderer.Render(mdContent)
		if err != nil {
			rendered = wordWrap(mdContent, width-10)
		}
		rendered = strings.TrimSuffix(rendered, "\n")
		return labelLine + "\n" + spacerLine + "\n" + prefixedLineBg(" ", wrapURLsOSC8(rendered), width, c.base.CustomMessageBg)
	}

	msg := textOnBg.Render("Branch summary (")
	keyHint := dimOnBg.Render(toggleKey)
	suffix := textOnBg.Render(" to expand)")
	labelLine := padLineToWidth(label, width, c.base.CustomMessageBg)
	contentLine := padLineToWidth(textOnBg.Render(" "+msg+keyHint+suffix), width, c.base.CustomMessageBg)
	return labelLine + "\n" + contentLine
}

// renderSkill renders a skill invocation (TS pi-mono: SkillInvocationMessageComponent).
// Content format: "name\n\nfullSkillContent"
func (c *CustomMessageComponent) renderSkill(entry ChatEntry, width int, label string, textOnBg, dimOnBg lipgloss.Style, toggleKey string) string {
	name := entry.Content
	fullContent := ""
	if idx := strings.Index(entry.Content, "\n\n"); idx != -1 {
		name = entry.Content[:idx]
		fullContent = entry.Content[idx+2:]
	}

	if entry.Expanded && fullContent != "" {
		labelLine := padLineToWidth(label, width, c.base.CustomMessageBg)
		spacerLine := padLineToWidth(textOnBg.Render(""), width, c.base.CustomMessageBg)
		mdContent := "**" + name + "**\n\n" + fullContent
		rendered, err := c.base.MdRenderer.Render(mdContent)
		if err != nil {
			return labelLine + "\n" + spacerLine + "\n" +
				prefixedLineBg(" ", wordWrap(mdContent, width-14), width, c.base.CustomMessageBg)
		}
		rendered = strings.TrimSuffix(rendered, "\n")
		return labelLine + "\n" + spacerLine + "\n" +
			prefixedLineBg(" ", wrapURLsOSC8(rendered), width, c.base.CustomMessageBg)
	}

	hint := dimOnBg.Render(" (" + toggleKey + " to expand)")
	return padLineToWidth(label+" "+textOnBg.Render(name)+hint, width, c.base.CustomMessageBg)
}

// renderGeneric renders a generic custom message (TS pi-mono: CustomMessageComponent).
// Shows first line preview + expand hint when collapsed.
func (c *CustomMessageComponent) renderGeneric(entry ChatEntry, width int, label string, textOnBg, dimOnBg lipgloss.Style, toggleKey string) string {
	if entry.Expanded && entry.Content != "" {
		labelLine := padLineToWidth(label, width, c.base.CustomMessageBg)
		spacerLine := padLineToWidth(textOnBg.Render(""), width, c.base.CustomMessageBg)
		rendered, err := c.base.MdRenderer.Render(entry.Content)
		if err != nil {
			return labelLine + "\n" + spacerLine + "\n" +
				prefixedLineBg(" ", wordWrap(entry.Content, width-14), width, c.base.CustomMessageBg)
		}
		rendered = strings.TrimSuffix(rendered, "\n")
		return labelLine + "\n" + spacerLine + "\n" +
			prefixedLineBg(" ", wrapURLsOSC8(rendered), width, c.base.CustomMessageBg)
	}

	if entry.Content != "" {
		firstLine := entry.Content
		if idx := strings.IndexByte(firstLine, '\n'); idx != -1 {
			firstLine = firstLine[:idx]
		}
		if lipgloss.Width(firstLine) > 80 {
			firstLine = TruncateByWidth(firstLine, 77) + "..."
		}
		hint := dimOnBg.Render(" (" + toggleKey + " to expand)")
		return padLineToWidth(label+" "+textOnBg.Render(firstLine)+hint, width, c.base.CustomMessageBg)
	}

	return padLineToWidth(label, width, c.base.CustomMessageBg)
}

// Ensure CustomMessageComponent implements MessageComponent.
var _ MessageComponent = (*CustomMessageComponent)(nil)
