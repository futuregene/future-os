package components

import (
	"fmt"
	"sort"
	"strings"

	"github.com/charmbracelet/lipgloss"
)

// View renders the footer.
func (f *Footer) View() string {
	width := f.baseStyle.GetWidth()
	if width == 0 {
		width = 120
	}

	lines := make([]string, 0, 3)

	// ── Line 1: dim CWD + git branch + session name ──────────────────────
	line1 := f.buildLine1(width)
	lines = append(lines, line1)

	// ── Line 2: stats (left) + model info (right) ────────────────────────
	line2 := f.buildLine2(width)
	lines = append(lines, line2)

	// ── Extension status line ────────────────────────────────────────────
	if len(f.extensionStatuses) > 0 {
		extLine := f.buildExtensionLine(width)
		lines = append(lines, extLine)
	}

	return lipgloss.JoinVertical(lipgloss.Top, lines...)
}

// buildLine1 constructs the dimmed CWD line with ~ abbreviation, git branch, and session name.
func (f *Footer) buildLine1(width int) string {
	pwd := f.cwd

	// Replace home directory with ~
	if f.homeDir != "" && strings.HasPrefix(pwd, f.homeDir) {
		pwd = "~" + pwd[len(f.homeDir):]
	}
	if pwd == "" {
		pwd = "~"
	}

	// Add git branch in parentheses
	if f.gitBranch != "" {
		pwd = pwd + " (" + f.gitBranch + ")"
	}

	// Add session name with bullet separator
	if f.sessionName != "" {
		pwd = pwd + " • " + f.sessionName
	}

	// Truncate if too wide (accounting for dim ANSI codes)
	rendered := f.dimStyle.Render(pwd)
	if lipgloss.Width(rendered) > width && width > 0 {
		// Truncate the plain text, not the rendered version
		for lipgloss.Width(f.dimStyle.Render(pwd)) > width && len(pwd) > 3 {
			pwd = pwd[:len(pwd)-1]
		}
		// Safety: ensure we don't produce zero-length
		if len(pwd) <= 3 {
			pwd = "..."
		} else {
			pwd = pwd + "..."
		}
		rendered = f.dimStyle.Render(pwd)
	}

	return rendered
}

// buildLine2 constructs the stats + model line with left/right layout.
// TS pi-mono style: dim stats | padding | dim (provider) model · thinking
func (f *Footer) buildLine2(width int) string {
	// ── Left stats ───────────────────────────────────────────────────────
	var statsParts []string

	// Streaming indicator moved to statusLine() (TS pi-mono: statusContainer).
	// Footer shows only stats + model during streaming, matching pi-mono.

	if f.tokensIn > 0 {
		statsParts = append(statsParts, "↑"+formatTokens(f.tokensIn))
	}
	if f.tokensOut > 0 {
		statsParts = append(statsParts, "↓"+formatTokens(f.tokensOut))
	}
	if f.tokensCacheR > 0 {
		statsParts = append(statsParts, "R"+formatTokens(f.tokensCacheR))
	}
	if f.tokensCacheW > 0 {
		statsParts = append(statsParts, "W"+formatTokens(f.tokensCacheW))
	}

	// Cost with optional (sub) indicator
	if f.totalCost > 0 || f.usingSubscription {
		costStr := fmt.Sprintf("$%.3f", f.totalCost)
		if f.usingSubscription {
			costStr += " (sub)"
		}
		statsParts = append(statsParts, costStr)
	}

	// Context percentage (colored: >90% red, >70% yellow, ≤70% default)
	ctxStr := f.formatContextBar()
	if ctxStr != "" {
		statsParts = append(statsParts, ctxStr)
	}

	statsLeft := strings.Join(statsParts, " ")

	// ── Right side: (provider) model · thinking ──────── dim gray (same as stats) ──
	modelPart := f.model
	if modelPart == "" {
		modelPart = "no-model"
	}

	rightSide := modelPart

	// Prepend provider only when multiple providers configured (TS pi-mono style)
	if f.provider != "" && f.availableProviderCount > 1 {
		rightSide = "(" + f.provider + ") " + modelPart
	}

	// Only show thinking when model supports reasoning (TS pi-mono)
	// Format: "model • thinking off" or "model • low" (TS pi-mono format)
	if f.hasReasoning {
		thinkingDisplay := f.thinking
		if thinkingDisplay == "" {
			thinkingDisplay = "off"
		}
		if thinkingDisplay == "off" {
			rightSide = rightSide + " • thinking off"
		} else {
			rightSide = rightSide + " • " + thinkingDisplay
		}
	}

	// ── Layout: both sides rendered in dim gray ─────────────────────────
	// TS wraps statsLeft and rightSide in separate dim calls to prevent
	// ANSI reset codes in context% from bleeding into the dim.
	// IMPORTANT: use a no-width copy of dimStyle for rendering; the parent
	// style's Width() inherited from baseStyle would pad to full width.
	renderDim := f.dimStyle.Copy().Width(0)
	dimStatsLeft := renderDim.Render(statsLeft)
	statsLeftWidth := lipgloss.Width(dimStatsLeft)

	dimRight := renderDim.Render(rightSide)
	rightSideWidth := lipgloss.Width(dimRight)
	minPadding := 2
	totalNeeded := statsLeftWidth + minPadding + rightSideWidth

	if totalNeeded <= width {
		space := width - statsLeftWidth - rightSideWidth
		if space < 0 {
			space = 0
		}
		padding := strings.Repeat(" ", space)
		return dimStatsLeft + padding + dimRight
	}

	// Not enough space: try dropping provider first, then truncate (TS pi-mono)
	availableForRight := width - statsLeftWidth - minPadding
	if availableForRight > 0 {
		// Try full right side first
		if lipgloss.Width(renderDim.Render(rightSide)) <= availableForRight {
			// fits
		} else if f.provider != "" && f.availableProviderCount > 1 {
			// Try dropping provider prefix
			tryRight := modelPart
			if f.hasReasoning {
				thinkingDisplay := f.thinking
				if thinkingDisplay == "" {
					thinkingDisplay = "off"
				}
				if thinkingDisplay == "off" {
					tryRight = tryRight + " • thinking off"
				} else {
					tryRight = tryRight + " • " + thinkingDisplay
				}
			}
			if lipgloss.Width(renderDim.Render(tryRight)) <= availableForRight {
				rightSide = tryRight
			} else {
				for lipgloss.Width(rightSide) > availableForRight && len(rightSide) > 0 {
					rightSide = rightSide[:len(rightSide)-1]
				}
			}
		} else {
			for lipgloss.Width(rightSide) > availableForRight && len(rightSide) > 0 {
				rightSide = rightSide[:len(rightSide)-1]
			}
		}
		dimRight = renderDim.Render(rightSide)
		rightSideWidth = lipgloss.Width(dimRight)
		space := width - statsLeftWidth - rightSideWidth
		if space < 0 {
			space = 0
		}
		padding := strings.Repeat(" ", space)
		return dimStatsLeft + padding + dimRight
	}

	// Not enough space for right side at all, just show stats
	return dimStatsLeft
}

// buildExtensionLine builds a single line of sorted extension statuses, truncated.
func (f *Footer) buildExtensionLine(width int) string {
	// Sort extension names alphabetically
	names := make([]string, 0, len(f.extensionStatuses))
	for name := range f.extensionStatuses {
		names = append(names, name)
	}
	sort.Strings(names)

	var parts []string
	for _, name := range names {
		text := f.extensionStatuses[name]
		// Sanitize: collapse whitespace
		text = strings.TrimSpace(strings.Join(strings.Fields(text), " "))
		if text != "" {
			parts = append(parts, text)
		}
	}

	line := strings.Join(parts, " ")
	rendered := f.dimStyle.Render(line)

	// Width-aware truncation with dim ellipsis (matching TS pi-mono truncateToWidth)
	if lipgloss.Width(rendered) > width {
		ellipsis := f.dimStyle.Render("...")
		ellipsisWidth := lipgloss.Width(ellipsis)
		runes := []rune(line)
		for len(runes) > 0 && lipgloss.Width(f.dimStyle.Render(string(runes)))+ellipsisWidth > width {
			runes = runes[:len(runes)-1]
		}
		return f.dimStyle.Render(string(runes)) + ellipsis
	}

	return rendered
}

// formatContextBar returns a colored context percentage string like "73.2%/128k (auto)".
// Uses TS pi-mono color scheme: >90% red, >70% yellow, ≤70% no color.
// Always shows context% at startup (TS pi-mono always pushes to statsParts).
func (f *Footer) formatContextBar() string {
	// Show "?" at startup when context is unknown (TS pi-mono: always visible)
	if f.contextPercent <= 0 && f.contextMaxTokens <= 0 {
		if f.autoCompact {
			return "? (auto)"
		}
		return "?"
	}

	var text string
	pct := f.contextPercent
	if pct <= 0 {
		text = "?" // TS pi-mono: show "?" after compaction when context unknown
	} else {
		text = fmt.Sprintf("%.1f%%", pct)
	}

	// Add max tokens
	if f.contextMaxTokens > 0 {
		text += "/" + formatTokens(f.contextMaxTokens)
	}

	// Add (auto) if auto-compact is enabled
	if f.autoCompact {
		text += " (auto)"
	}

	if pct > 90 {
		return f.ctxRed.Render(text)
	}
	if pct > 70 {
		return f.ctxYellow.Render(text)
	}
	return text
}
