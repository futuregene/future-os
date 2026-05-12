package components

import (
	"fmt"
	"os"
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
	hasReasoning       bool // only show thinking when model supports reasoning

	// Context usage display
	contextPercent float64 // 0.0 ~ 100.0
	contextMaxTokens int
	autoCompact bool

	// Spinner animation
	spinnerFrame int

	// Working indicator customization
	workingMessage string
	workingVisible bool
	customFrames    []string
	customIntervalMs int

	// Extensions
	extensionStatuses map[string]string

	// Provider count (TS pi-mono: only show provider when >1)
	availableProviderCount int

	// Session stats
	entryCount int

	// Home directory for ~ abbreviation
	homeDir string
}

// NewFooter creates a new Footer component with pre-built styles.
// ctxGreen, ctxYellow, ctxRed are hex colors for the context percentage bar.
func NewFooter(baseStyle lipgloss.Style, ctxGreen, ctxYellow, ctxRed string) Footer {
	if baseStyle.GetWidth() == 0 {
		baseStyle = lipgloss.NewStyle().
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

// SetStyle updates the footer's base style and context bar colors (for live theme reload).
func (f *Footer) SetStyle(baseStyle lipgloss.Style, ctxGreen, ctxYellow, ctxRed string) {
	f.baseStyle = baseStyle.Copy().UnsetBackground()
	if baseStyle.GetWidth() == 0 {
		f.baseStyle = f.baseStyle.Copy().UnsetWidth()
	} else {
		f.baseStyle = baseStyle.Copy().UnsetBackground()
	}
	f.dimStyle = f.baseStyle.Copy().Faint(true)
	f.ctxGreen = lipgloss.NewStyle().Foreground(lipgloss.Color(ctxGreen))
	f.ctxYellow = lipgloss.NewStyle().Foreground(lipgloss.Color(ctxYellow))
	f.ctxRed = lipgloss.NewStyle().Foreground(lipgloss.Color(ctxRed))
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

// SetGitBranch updates only the git branch (for live branch watching).
func (f *Footer) SetGitBranch(branch string) {
	f.gitBranch = branch
}

// Model returns the current model string.
func (f Footer) Model() string { return f.model }

// Provider returns the current provider string.
func (f Footer) Provider() string { return f.provider }

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

// spinner frames for animated streaming indicator
var spinnerFrames = []string{"⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"}

// SetSpinnerFrame sets the current spinner animation frame.
func (f *Footer) SetSpinnerFrame(n int) {
	f.spinnerFrame = n
}

// SetWorkingMessage sets the working message shown during streaming.
func (f *Footer) SetWorkingMessage(msg string) {
	f.workingMessage = msg
}

// SetWorkingVisible sets whether the working loader is shown during streaming.
func (f *Footer) SetWorkingVisible(visible bool) {
	f.workingVisible = visible
}

// SetWorkingIndicator sets custom spinner frames and interval for the streaming loader.
// Pass nil/empty frames to restore the default spinner.
func (f *Footer) SetWorkingIndicator(frames []string, intervalMs int) {
	f.customFrames = frames
	f.customIntervalMs = intervalMs
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

// SetHasReasoning sets whether the current model supports reasoning/thinking.
func (f *Footer) SetHasReasoning(v bool) {
	f.hasReasoning = v
}

// SetEntryCount sets the number of entries in the current session.
func (f *Footer) SetEntryCount(n int) {
	f.entryCount = n
}

// SetAvailableProviders sets the number of configured providers.
// When >1, the provider label is shown in the footer (TS pi-mono style).
func (f *Footer) SetAvailableProviders(count int) {
	f.availableProviderCount = count
}

// SetExtensionStatuses sets extension status strings (keyed by extension name).
func (f *Footer) SetExtensionStatuses(statuses map[string]string) {
	f.extensionStatuses = statuses
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
