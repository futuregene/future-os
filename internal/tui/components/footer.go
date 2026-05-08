package components

import (
	"fmt"
	"os"
	"sort"
	"strings"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

// Footer shows the status bar at the bottom of the TUI.
// Two-line layout matching pi-mono style:
//
//	Line 1: dim CWD (gitBranch) • sessionName
//	Line 2: dim stats (↑in ↓out Rr Ww $cost ctx:xx%) | right-align (provider) model · thinking
//	Extension line: sorted extension statuses
type Footer struct {
	// Pre-built styles (created once, reused in View)
	baseStyle lipgloss.Style
	dimStyle  lipgloss.Style
	ctxGreen  lipgloss.Style
	ctxYellow lipgloss.Style
	ctxRed    lipgloss.Style

	// Session info
	cwd         string
	gitBranch   string
	sessionName string
	model       string
	thinking    string
	provider    string

	// Stats
	tokensIn     int
	tokensOut    int
	tokensCacheR int
	tokensCacheW int
	totalCost    float64
	contextUsed  float64 // 0.0 ~ 1.0

	// Flags
	compactEnabled     bool
	streaming          bool
	usingSubscription  bool

	// Context usage display
	contextPercent float64 // 0.0 ~ 100.0
	contextMaxTokens int
	autoCompact bool

	// Extensions
	extensionStatuses map[string]string

	// Home directory for ~ abbreviation
	homeDir string
}

// NewFooter creates a new Footer component with pre-built styles.
// ctxGreen, ctxYellow, ctxRed are hex colors for the context percentage bar.
func NewFooter(baseStyle lipgloss.Style, ctxGreen, ctxYellow, ctxRed string) Footer {
	if baseStyle.GetWidth() == 0 {
		baseStyle = lipgloss.NewStyle().
			Background(lipgloss.Color("#21252b")).
			Foreground(lipgloss.Color("#abb2bf"))
	}

	// Derive dim style from the base style
	dimStyle := baseStyle.Copy().Faint(true)

	// Context percentage color styles
	greenStyle := lipgloss.NewStyle().Foreground(lipgloss.Color(ctxGreen))
	yellowStyle := lipgloss.NewStyle().Foreground(lipgloss.Color(ctxYellow))
	redStyle := lipgloss.NewStyle().Foreground(lipgloss.Color(ctxRed))

	// Detect home directory
	homeDir, _ := os.UserHomeDir()

	return Footer{
		baseStyle: baseStyle,
		dimStyle:  dimStyle,
		ctxGreen:  greenStyle,
		ctxYellow: yellowStyle,
		ctxRed:    redStyle,
		homeDir:   homeDir,
	}
}

// SetSession updates session info.
func (f *Footer) SetSession(cwd, gitBranch, sessionName, model, thinking, provider string) {
	f.cwd = cwd
	f.gitBranch = gitBranch
	f.sessionName = sessionName
	f.model = model
	f.thinking = thinking
	f.provider = provider
}

// Update updates footer stats from a StatusMsg equivalent.
func (f *Footer) Update(tokensIn, tokensOut, cacheR, cacheW int, cost, ctxUsed float64, streaming bool) {
	f.tokensIn = tokensIn
	f.tokensOut = tokensOut
	f.tokensCacheR = cacheR
	f.tokensCacheW = cacheW
	f.totalCost = cost
	f.contextUsed = ctxUsed
	f.streaming = streaming
}

// SetContextUsage configures the context percentage display.
func (f *Footer) SetContextUsage(percent float64, maxTokens int, auto bool) {
	f.contextPercent = percent
	f.contextMaxTokens = maxTokens
	f.autoCompact = auto
}

// SetWidth updates the footer width for dynamic resizing.
func (f *Footer) SetWidth(w int) {
	// Subtract padding: baseStyle has Padding(0,1) so we reserve 2 chars
	if w > 2 {
		f.baseStyle = f.baseStyle.Width(w - 2)
	} else {
		f.baseStyle = f.baseStyle.Width(w)
	}
}

// SetUsingSubscription sets whether the current model uses an OAuth subscription.
func (f *Footer) SetUsingSubscription(v bool) {
	f.usingSubscription = v
}

// SetExtensionStatuses sets extension status strings (keyed by extension name).
func (f *Footer) SetExtensionStatuses(statuses map[string]string) {
	f.extensionStatuses = statuses
}

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
			pwd = "…"
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

	// Prepend provider only when provider differs from model
	if f.provider != "" && !strings.Contains(strings.ToLower(f.model), strings.ToLower(f.provider)) {
		rightSide = "(" + f.provider + ") " + modelPart
	}

	// Always show thinking level
	thinkingDisplay := f.thinking
	if thinkingDisplay == "" {
		thinkingDisplay = "off"
	}
	rightSide = rightSide + " · " + thinkingDisplay

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

	// Not enough space: truncate right side
	availableForRight := width - statsLeftWidth - minPadding
		if availableForRight > 0 {
			for lipgloss.Width(rightSide) > availableForRight && len(rightSide) > 0 {
				rightSide = rightSide[:len(rightSide)-1]
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

	// Truncate if too wide
	if lipgloss.Width(rendered) > width {
		for lipgloss.Width(f.dimStyle.Render(line)) > width && len(line) > 3 {
			line = line[:len(line)-1]
		}
		line = line + "..."
		rendered = f.dimStyle.Render(line)
	}

	return rendered
}

// formatContextBar returns a colored context percentage string like "73.2%/128k (auto)".
// Uses TS pi-mono color scheme: >90% red, >70% yellow, ≤70% no color.
func (f *Footer) formatContextBar() string {
	if f.contextPercent <= 0 && f.contextMaxTokens <= 0 {
		return ""
	}

	pct := f.contextPercent
	text := fmt.Sprintf("%.1f%%", pct)

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

// ─── Token formatting (pi-mono style) ──────────────────────────────────────

func formatTokens(n int) string {
	if n < 1000 {
		return fmt.Sprintf("%d", n)
	}
	if n < 10000 {
		return fmt.Sprintf("%.1fk", float64(n)/1000)
	}
	if n < 1000000 {
		return fmt.Sprintf("%dk", (n+500)/1000) // round to nearest k
	}
	if n < 10000000 {
		return fmt.Sprintf("%.1fM", float64(n)/1000000)
	}
	return fmt.Sprintf("%dM", (n+500000)/1000000)
}

var _ = tea.Quit

// ─── Legacy helpers (used by other components in the package) ──────────────

func formatNum(n int) string {
	if n >= 1000000 {
		return itoa(n/1000000) + "." + itoa((n%1000000)/100000) + "M"
	}
	if n >= 1000 {
		return itoa(n/1000) + "." + itoa((n%1000)/100) + "k"
	}
	return itoa(n)
}

func itoa(n int) string {
	if n == 0 {
		return "0"
	}
	var digits []byte
	for n > 0 {
		digits = append([]byte{byte('0' + n%10)}, digits...)
		n /= 10
	}
	return string(digits)
}

func i64toa(n int64) string {
	if n == 0 {
		return "0"
	}
	var digits []byte
	for n > 0 {
		digits = append([]byte{byte('0' + n%10)}, digits...)
		n /= 10
	}
	return string(digits)
}

func floatToStr(v float64) string {
	if v < 0 {
		return "-" + floatToStr(-v)
	}
	intPart := int64(v)
	frac := int64((v - float64(intPart)) * 10000)
	var sb strings.Builder
	sb.WriteString(i64toa(intPart))
	sb.WriteByte('.')
	f := i64toa(frac)
	for len(f) < 4 {
		f = "0" + f
	}
	sb.WriteString(f[:4])
	return sb.String()
}

func fixedStr(format string, v float64) string {
	var sb strings.Builder
	s := floatToStr(v)
	result := format
	for i := 0; i < len(result); i++ {
		if result[i] == '%' && i+1 < len(result) && result[i+1] == 'f' {
			sb.WriteString(s)
			i++
		} else if result[i] == '%' && i+1 < len(result) && result[i+1] == 'd' {
			sb.WriteString(s)
			i++
		} else {
			sb.WriteByte(result[i])
		}
	}
	return sb.String()
}
