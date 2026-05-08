package components

import (
	"strings"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

// Footer shows the status bar at the bottom of the TUI.
type Footer struct {
	style lipgloss.Style

	cwd        string
	gitBranch  string
	sessionName string
	model      string
	thinking   string

	tokensIn     int
	tokensOut    int
	tokensCacheR int
	tokensCacheW int
	totalCost    float64
	contextUsed  float64 // 0.0 ~ 1.0

	compacting bool
	streaming  bool
}

// NewFooter creates a new Footer component.
func NewFooter(style lipgloss.Style) Footer {
	if style.GetWidth() == 0 {
		style = lipgloss.NewStyle().
			Background(lipgloss.Color("#21252b")).
			Foreground(lipgloss.Color("#abb2bf"))
	}
	return Footer{style: style}
}

// Update updates footer state from a StatusMsg equivalent.
func (f *Footer) Update(tokensIn, tokensOut, cacheR, cacheW int, cost, ctxUsed float64, compacting, streaming bool) {
	f.tokensIn = tokensIn
	f.tokensOut = tokensOut
	f.tokensCacheR = cacheR
	f.tokensCacheW = cacheW
	f.totalCost = cost
	f.contextUsed = ctxUsed
	f.compacting = compacting
	f.streaming = streaming
}

// SetSession updates session info.
func (f *Footer) SetSession(cwd, gitBranch, sessionName, model, thinking string) {
	f.cwd = cwd
	f.gitBranch = gitBranch
	f.sessionName = sessionName
	f.model = model
	f.thinking = thinking
}

// View renders the footer.
func (f *Footer) View() string {
	// Left section: CWD + git branch + session name
	left := f.cwd
	if f.gitBranch != "" {
		left += "  ⎇ " + f.gitBranch
	}
	if f.sessionName != "" {
		left += "  " + f.sessionName
	}

	// Center section: tokens + cost + context %
	var center strings.Builder
	if f.streaming || f.compacting {
		status := "● streaming"
		if f.compacting {
			status = "◎ compacting"
		}
		center.WriteString(status + "  ")
	}
	center.WriteString(formatTokens(f.tokensIn, f.tokensOut, f.tokensCacheR, f.tokensCacheW))
	if f.totalCost > 0 {
		center.WriteString(fixedStr("  $%.4f", f.totalCost))
	}
	center.WriteString("  ")
	center.WriteString(formatContextBar(f.contextUsed))

	// Right section: model + thinking level
	right := f.model
	if f.thinking != "" && f.thinking != "off" {
		right += " · " + f.thinking
	}

	// Pad to fill
	width := f.style.GetWidth()
	if width == 0 {
		width = 120
	}
	total := left + " " + center.String() + " " + right
	if len(total) < width {
		pad := width - len(total)
		_ = pad // reserved for future centering
		// Simple approach: just pad center
		if pad > 3 {
			center.WriteString(strings.Repeat(" ", pad))
		}
	}

	line1 := f.style.Render(left)
	line2 := f.style.Render(center.String())

	return lipgloss.JoinVertical(lipgloss.Top, line1, line2)
}

func formatTokens(in, out, cacheR, cacheW int) string {
	var parts []string
	if in > 0 {
		parts = append(parts, "↑"+formatNum(in))
	}
	if out > 0 {
		parts = append(parts, "↓"+formatNum(out))
	}
	if cacheR > 0 {
		parts = append(parts, "R"+formatNum(cacheR))
	}
	if cacheW > 0 {
		parts = append(parts, "W"+formatNum(cacheW))
	}
	return strings.Join(parts, " ")
}

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

func fixedStr(format string, v float64) string {
	// Simple sprintf that doesn't import fmt
	var sb strings.Builder
	// Just format the float as string
	s := floatToStr(v)
	// Replace %f with the value
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

func floatToStr(v float64) string {
	// Quick and dirty float formatting without fmt
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

func formatContextBar(used float64) string {
	if used <= 0 {
		return "ctx: --"
	}
	pct := int(used * 100)
	color := lipgloss.Color("#98c379") // green
	if used > 0.9 {
		color = lipgloss.Color("#e06c75") // red
	} else if used > 0.7 {
		color = lipgloss.Color("#e5c07b") // yellow
	}
	return lipgloss.NewStyle().Foreground(color).Render(fixedStr("ctx: %d%%", float64(pct)))
}

var _ = tea.Quit
