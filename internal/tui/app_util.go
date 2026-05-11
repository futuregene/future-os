// Package tui provides the interactive terminal UI for xihu using Bubble Tea.
package tui

import (

	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"time"




)

// ─── Message Types ─────────────────────────────────────────────────────────

// StreamTextMsg is a chunk of streamed text from the LLM.

func getGitBranch(cwd string) string {
	cmd := exec.Command("git", "rev-parse", "--abbrev-ref", "HEAD")
	cmd.Dir = cwd
	out, err := cmd.Output()
	if err != nil {
		return ""
	}
	branch := strings.TrimSpace(string(out))
	// "HEAD" means we're in detached HEAD state
	if branch == "HEAD" {
		return ""
	}
	return branch
}

// updateTerminalTitle sets the terminal window title to "xihu - sessionName - cwd" or "xihu - cwd".
func updateTerminalTitle(sessionName, cwd string) {
	basename := filepath.Base(cwd)
	var title string
	if sessionName != "" {
		title = fmt.Sprintf("xihu - %s - %s", sessionName, basename)
	} else {
		title = fmt.Sprintf("xihu - %s", basename)
	}
	fmt.Fprintf(os.Stdout, "\033]0;%s\007", title)
}

// ─── Tool Duration Formatting ────────────────────────────────────────────────

// formatDuration formats a duration in milliseconds as a human-readable string.
func formatDuration(ms int64) string {
	if ms <= 0 {
		return ""
	}
	if ms < 1000 {
		return fmt.Sprintf("%dms", ms)
	}
	return fmt.Sprintf("%.1fs", float64(ms)/1000)
}

// formatContextPath formats a context file path for display (TS pi-mono: formatContextPath).
func formatContextPath(fp string) string {
	home, _ := os.UserHomeDir()
	if home != "" && strings.HasPrefix(fp, home) {
		return "~" + fp[len(home):]
	}
	return filepath.Base(fp)
}

// openExternalEditor opens $EDITOR (or nano/vi) on a temp file and returns the content.
func formatRelativeDate(t time.Time) string {
	diff := time.Since(t)
	switch {
	case diff < time.Minute:
		return "now"
	case diff < time.Hour:
		return fmt.Sprintf("%dm", int(diff.Minutes()))
	case diff < 24*time.Hour:
		return fmt.Sprintf("%dh", int(diff.Hours()))
	case diff < 7*24*time.Hour:
		return fmt.Sprintf("%dd", int(diff.Hours()/24))
	case diff < 30*24*time.Hour:
		return fmt.Sprintf("%dw", int(diff.Hours()/(24*7)))
	case diff < 365*24*time.Hour:
		return fmt.Sprintf("%dmo", int(diff.Hours()/(24*30)))
	default:
		return fmt.Sprintf("%dy", int(diff.Hours()/(24*365)))
	}
}

// parseModelString splits a model string like "deepseek/deepseek-chat"
// into (modelName, provider). Returns the original string as modelName
// and empty provider if no "/" separator is found.
func parseModelString(modelStr string) (modelName, provider string) {
	parts := strings.SplitN(modelStr, "/", 2)
	if len(parts) == 2 {
		return parts[1], parts[0]
	}
	return modelStr, ""
}

// Ensure imports are used.
var _ = fmt.Sprintf
var _ = context.Background

// copyToClipboard copies text to the system clipboard using platform-specific commands.
func copyToClipboard(text string) error {
	var cmd *exec.Cmd
	switch runtime.GOOS {
	case "darwin":
		cmd = exec.Command("pbcopy")
	case "linux":
		// Try wl-copy (Wayland) first, fall back to xclip (X11)
		if _, err := exec.LookPath("wl-copy"); err == nil {
			cmd = exec.Command("wl-copy")
		} else if _, err := exec.LookPath("xclip"); err == nil {
			cmd = exec.Command("xclip", "-selection", "clipboard")
		} else {
			return fmt.Errorf("no clipboard tool found (install wl-copy or xclip)")
		}
	case "windows":
		cmd = exec.Command("clip.exe")
	default:
		return fmt.Errorf("unsupported platform: %s", runtime.GOOS)
	}
	cmd.Stdin = strings.NewReader(text)
	return cmd.Run()
}

// pasteFromClipboard reads text from the system clipboard using platform-specific commands.
func pasteFromClipboard() (string, error) {
	var cmd *exec.Cmd
	switch runtime.GOOS {
	case "darwin":
		cmd = exec.Command("pbpaste")
	case "linux":
		if _, err := exec.LookPath("wl-paste"); err == nil {
			cmd = exec.Command("wl-paste")
		} else if _, err := exec.LookPath("xclip"); err == nil {
			cmd = exec.Command("xclip", "-selection", "clipboard", "-o")
		} else {
			return "", fmt.Errorf("no clipboard tool found (install wl-paste or xclip)")
		}
	case "windows":
		cmd = exec.Command("powershell", "-Command", "Get-Clipboard")
	default:
		return "", fmt.Errorf("unsupported platform: %s", runtime.GOOS)
	}
	out, err := cmd.Output()
	if err != nil {
		return "", err
	}
	return string(out), nil
}

// startProgress writes OSC 9;4;3 to show an indeterminate progress bar.
// (TS pi-mono: terminal.ts setProgress(true) with keepalive timer)
func splitSlashCommand(text string) (name string, args []string) {
	text = strings.TrimPrefix(text, "/")
	parts := strings.Fields(text)
	if len(parts) == 0 {
		return "", nil
	}
	if len(parts) > 1 {
		args = parts[1:]
	}
	return parts[0], args
}
// commaInt formats an integer with comma separators (matching TS pi-mono toLocaleString).
func commaInt(n int) string {
	s := fmt.Sprintf("%d", n)
	var result []byte
	for i := len(s) - 1; i >= 0; i-- {
		result = append([]byte{s[i]}, result...)
		if (len(s)-i)%3 == 0 && i > 0 {
			result = append([]byte{','}, result...)
		}
	}
	return string(result)
}

func fuzzyMatch(pattern, s string) bool {
	j := 0
	for i := 0; i < len(s) && j < len(pattern); i++ {
		if s[i] == pattern[j] {
			j++
		}
	}
	return j == len(pattern)
}

func boolToStr(v bool) string {
	if v {
		return "true"
	}
	return "false"
}

// showWarningsSelector opens a warning settings submenu (TS pi-mono: settings submenu).
