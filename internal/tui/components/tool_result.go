package components

import (
	"strings"

	"github.com/charmbracelet/lipgloss"
)

// ToolResultComponent renders "tool_result" type chat entries.
// Extracted from ChatViewport.View() matching TS pi-mono's ToolResultComponent pattern.
// Handles compact read results, success/error backgrounds, duration display,
// diff rendering, and inline images.
type ToolResultComponent struct {
	base *MessageComponentBase
}

// NewToolResultComponent creates a new tool result component.
func NewToolResultComponent(base *MessageComponentBase) *ToolResultComponent {
	return &ToolResultComponent{base: base}
}

// toggleKey returns the tool toggle key string with fallback.
func (c *ToolResultComponent) toggleKey() string {
	if c.base.ToolToggleKey != nil && *c.base.ToolToggleKey != "" {
		return *c.base.ToolToggleKey
	}
	return "Ctrl+O"
}

// Render renders a tool result entry.
// Handles compact read (system files) with collapsed one-liner.
// Uses success (green) or error (red) background.
// Shows duration when available ("Took X.Xs").
// Renders unified diffs via RenderDiff when content is detected as diff.
// Renders inline images via ImageComponent when ShowImages is enabled.
func (c *ToolResultComponent) Render(entry ChatEntry, width int) string {
	// Compact read result: show compact one-liner when collapsed (TS pi-mono hides result)
	if entry.CompactReadKind != "" {
		return c.renderCompactReadResult(entry, width)
	}

	// Standard tool result rendering
	return c.renderStandardResult(entry, width)
}

// renderCompactReadResult renders a compact read tool result (system files like SKILL.md).
func (c *ToolResultComponent) renderCompactReadResult(entry ChatEntry, width int) string {
	bgStyle := c.base.ToolSuccessBg
	icon := "✓ "
	if entry.IsError {
		bgStyle = c.base.ToolErrorBg
		icon = "✗ "
	}

	var compactLine string
	switch entry.CompactReadKind {
	case "skill":
		compactLine = c.base.CustomLabelStyle.Render("[skill] ") + entry.CompactReadLabel
	case "docs":
		compactLine = c.base.ToolStyle.Render("read docs ") + entry.CompactReadLabel
	case "resource":
		compactLine = c.base.ToolStyle.Render("read resource ") + entry.CompactReadLabel
	}
	if lr := formatLineRange(entry.ToolArgs); lr != "" {
		compactLine += c.base.WarningStyle.Render(lr)
	}
	if !entry.Expanded {
		compactLine += c.base.CustomDimStyle.Render(" (" + c.toggleKey() + " to expand)")
	}

	var sb strings.Builder
	sb.WriteString(applyLineBg(icon+compactLine, width, bgStyle))
	if entry.Expanded && entry.Content != "" {
		sb.WriteByte('\n')
		if isDiffContent(entry.Content) {
			diffStyle := DiffStyle{
				Add:     c.base.DiffAdd,
				Del:     c.base.DiffDel,
				Context: c.base.DiffCtx,
				Header:  c.base.DiffHeader,
				Inverse: lipgloss.NewStyle().Reverse(true),
			}
			rendered := RenderDiff(entry.Content, diffStyle)
			sb.WriteString(prefixedLineBg(" ", rendered, width, bgStyle))
		} else {
			sb.WriteString(prefixedLineBg(" ", wordWrap(entry.Content, width-14), width, bgStyle))
		}
	}
	return sb.String()
}

// renderStandardResult renders a standard (non-compact-read) tool result.
func (c *ToolResultComponent) renderStandardResult(entry ChatEntry, width int) string {
	var bgStyle lipgloss.Style
	var icon string
	if entry.IsError {
		bgStyle = c.base.ToolErrorBg
		icon = "✗ "
	} else {
		bgStyle = c.base.ToolSuccessBg
		icon = "✓ "
	}

	// Duration display (TS pi-mono: "Took X.Xs")
	durPart := ""
	if entry.ToolDuration != "" {
		durPart = "  Took " + entry.ToolDuration
	}

	detail := toolResultDetail(entry)
	header := icon + toolIcon(entry.ToolName) + entry.ToolName + detail + durPart
	if !entry.Expanded && entry.Content != "" {
		header += "  (" + c.toggleKey() + " to expand)"
	}

	var sb strings.Builder
	sb.WriteString(applyLineBg(header, width, bgStyle))

	if entry.Expanded && entry.Content != "" {
		sb.WriteByte('\n')
		if isDiffContent(entry.Content) {
			diffStyle := DiffStyle{
				Add:     c.base.DiffAdd,
				Del:     c.base.DiffDel,
				Context: c.base.DiffCtx,
				Header:  c.base.DiffHeader,
				Inverse: lipgloss.NewStyle().Reverse(true),
			}
			rendered := RenderDiff(entry.Content, diffStyle)
			sb.WriteString(prefixedLineBg(" ", rendered, width, bgStyle))
		} else {
			sb.WriteString(prefixedLineBg(" ", wordWrap(entry.Content, width-14), width, bgStyle))
		}

		// Inline images (TS pi-mono: ImageBlock rendering in tool results)
		showImages := c.base.ShowImages != nil && *c.base.ShowImages
		if showImages && len(entry.ImageBlocks) > 0 {
			imageTheme := ImageTheme{
				FallbackColor: func(s string) string {
					return lipgloss.NewStyle().Foreground(lipgloss.Color("#6c7086")).Render(s)
				},
			}
			imageWidthCells := 0
			if c.base.ImageWidthCells != nil {
				imageWidthCells = *c.base.ImageWidthCells
			}
			for _, ib := range entry.ImageBlocks {
				maxImgWidth := width - 4
				if imageWidthCells > 0 && imageWidthCells < maxImgWidth {
					maxImgWidth = imageWidthCells
				}
				img := NewImage(ib.Base64Data, ib.MimeType, imageTheme, ImageOptions{
					MaxWidthCells: maxImgWidth,
				})
				imageLines := img.Render(width)
				for _, line := range imageLines {
					sb.WriteByte('\n')
					sb.WriteString(applyLineBg("  "+line, width, bgStyle))
				}
			}
		}
	}

	return sb.String()
}

// Ensure ToolResultComponent implements MessageComponent.
var _ MessageComponent = (*ToolResultComponent)(nil)
