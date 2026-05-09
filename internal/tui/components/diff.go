package components

import (
	"strings"
	"unicode/utf8"

	"github.com/charmbracelet/lipgloss"
)

// ─── Diff Renderer ─────────────────────────────────────────────────────────

// DiffLine represents a single line in a unified diff.
type DiffLine struct {
	Type    string // "add", "del", "context", "header"
	Content string
}

// DiffStyle holds styles for diff rendering.
type DiffStyle struct {
	Add     lipgloss.Style
	Del     lipgloss.Style
	Context lipgloss.Style
	Header  lipgloss.Style
	Inverse lipgloss.Style
}

// DefaultDiffStyle returns the default diff color scheme.
func DefaultDiffStyle() DiffStyle {
	return DiffStyle{
		Add:     lipgloss.NewStyle().Foreground(lipgloss.Color("#a6e3a1")),
		Del:     lipgloss.NewStyle().Foreground(lipgloss.Color("#f38ba8")),
		Context: lipgloss.NewStyle().Foreground(lipgloss.Color("#6c7086")),
		Header:  lipgloss.NewStyle().Foreground(lipgloss.Color("#89b4fa")).Bold(true),
		Inverse: lipgloss.NewStyle().Reverse(true),
	}
}

// parseDiffLine extracts prefix, line number, and content from a diff line.
// Format: "+123 content" or "-123 content" or " 123 content" or "     ..."
func parseDiffLine(line string) (prefix, lineNum, content string, ok bool) {
	if len(line) < 2 {
		return "", "", "", false
	}
	prefix = string(line[0])
	if prefix != "+" && prefix != "-" && prefix != " " {
		return "", "", "", false
	}
	rest := line[1:]
	// Find the first space after the line number
	spaceIdx := strings.IndexByte(rest, ' ')
	if spaceIdx < 0 {
		return "", "", "", false
	}
	return prefix, rest[:spaceIdx], rest[spaceIdx+1:], true
}

// renderIntraLineDiff computes a word-level diff between old and new content,
// highlighting changed words with inverse colors (matching TS pi-mono diffWords).
func renderIntraLineDiff(oldContent, newContent string, inverse lipgloss.Style) (removedLine, addedLine string) {
	oldTokens := splitWords(oldContent)
	newTokens := splitWords(newContent)

	// Compute LCS to find common tokens
	lcsSet := lcsTokenSet(oldTokens, newTokens)

	var removed, added strings.Builder
	oi, ni := 0, 0
	for _, match := range lcsSet {
		// Output removed-only tokens before the match
		for oi < len(oldTokens) && !match.old {
			_ = match
			if oi == match.oldIdx {
				break
			}
			removed.WriteString(inverse.Render(oldTokens[oi]))
			oi++
		}
		// Output added-only tokens before the match
		for ni < len(newTokens) && ni < match.newIdx {
			added.WriteString(inverse.Render(newTokens[ni]))
			ni++
		}
		// Output the common token
		if oi < len(oldTokens) {
			// Strip leading whitespace inverse from first removed part
			s := oldTokens[oi]
			if removed.Len() == 0 {
				s = stripLeadingWsInverse(s, inverse)
			}
			removed.WriteString(s)
			oi++
		}
		if ni < len(newTokens) {
			s := newTokens[ni]
			if added.Len() == 0 {
				s = stripLeadingWsInverse(s, inverse)
			}
			added.WriteString(s)
			ni++
		}
	}
	// Remaining tokens
	for oi < len(oldTokens) {
		removed.WriteString(inverse.Render(oldTokens[oi]))
		oi++
	}
	for ni < len(newTokens) {
		added.WriteString(inverse.Render(newTokens[ni]))
		ni++
	}

	return removed.String(), added.String()
}

type tokenMatch struct {
	oldIdx int
	newIdx int
	old    bool
}

// lcsTokenSet computes LCS of two token sequences, returning matches
// with their indices for reconstruction.
func lcsTokenSet(oldTokens, newTokens []string) []tokenMatch {
	m, n := len(oldTokens), len(newTokens)
	if m == 0 || n == 0 {
		return nil
	}

	// 1D DP: use two rows to save memory
	prev := make([]int, n+1)
	curr := make([]int, n+1)
	// Store backpointers for reconstruction
	type bp struct{ i, j int }
	back := make([][]bp, m+1)
	for i := range back {
		back[i] = make([]bp, n+1)
	}

	for i := 1; i <= m; i++ {
		prev, curr = curr, prev
		for j := 1; j <= n; j++ {
			if oldTokens[i-1] == newTokens[j-1] {
				curr[j] = prev[j-1] + 1
				back[i][j] = bp{i - 1, j - 1}
			} else if prev[j] >= curr[j-1] {
				curr[j] = prev[j]
				back[i][j] = bp{i - 1, j}
			} else {
				curr[j] = curr[j-1]
				back[i][j] = bp{i, j - 1}
			}
		}
	}

	// Backtrack
	var matches []tokenMatch
	i, j := m, n
	for i > 0 && j > 0 {
		b := back[i][j]
		if b.i == i-1 && b.j == j-1 && oldTokens[i-1] == newTokens[j-1] {
			matches = append(matches, tokenMatch{oldIdx: i - 1, newIdx: j - 1})
		}
		i, j = b.i, b.j
	}
	// Reverse to get forward order
	for left, right := 0, len(matches)-1; left < right; left, right = left+1, right-1 {
		matches[left], matches[right] = matches[right], matches[left]
	}
	return matches
}

// splitWords splits a string into words and whitespace tokens for diffing.
func splitWords(s string) []string {
	var tokens []string
	var current []byte
	inWord := false

	for i := 0; i < len(s); {
		r, size := utf8.DecodeRuneInString(s[i:])
		isSpace := r == ' ' || r == '\t'
		if len(current) > 0 && isSpace != inWord {
			tokens = append(tokens, string(current))
			current = nil
		}
		current = append(current, s[i:i+size]...)
		inWord = !isSpace
		i += size
	}
	if len(current) > 0 {
		tokens = append(tokens, string(current))
	}
	return tokens
}

// stripLeadingWsInverse strips leading whitespace from the inverse-rendered
// text (matching TS behavior: don't highlight indentation changes).
func stripLeadingWsInverse(s string, inverse lipgloss.Style) string {
	trimmed := strings.TrimLeft(s, " \t")
	if trimmed == s {
		return s
	}
	leading := s[:len(s)-len(trimmed)]
	return leading + inverse.Render(trimmed)
}

// isDiffHeader returns true for unified diff header lines.
func isDiffHeader(line string) bool {
	return strings.HasPrefix(line, "+++") || strings.HasPrefix(line, "---") ||
		strings.HasPrefix(line, "@@") || strings.HasPrefix(line, "diff ") ||
		strings.HasPrefix(line, "index ") || strings.HasPrefix(line, "new file ") ||
		strings.HasPrefix(line, "deleted file ")
}

// RenderDiff renders a unified diff string with colors and word-level
// intra-line change highlighting (matching TS pi-mono diff rendering).
func RenderDiff(text string, style DiffStyle) string {
	lines := strings.Split(text, "\n")
	var result []string

	i := 0
	for i < len(lines) {
		line := lines[i]

		// Render diff headers specially
		if isDiffHeader(line) {
			result = append(result, style.Header.Render(line))
			i++
			continue
		}

		prefix, lineNum, content, ok := parseDiffLine(line)

		if !ok {
			result = append(result, style.Context.Render(line))
			i++
			continue
		}

		if prefix == "-" {
			// Collect consecutive removed lines
			var removedLines []struct{ lineNum, content string }
			for i < len(lines) {
				p, ln, ct, ok2 := parseDiffLine(lines[i])
				if !ok2 || p != "-" {
					break
				}
				removedLines = append(removedLines, struct{ lineNum, content string }{ln, ct})
				i++
			}

			// Collect consecutive added lines
			var addedLines []struct{ lineNum, content string }
			for i < len(lines) {
				p, ln, ct, ok2 := parseDiffLine(lines[i])
				if !ok2 || p != "+" {
					break
				}
				addedLines = append(addedLines, struct{ lineNum, content string }{ln, ct})
				i++
			}

			// Word-level intra-line diff for single removed+added pair
			if len(removedLines) == 1 && len(addedLines) == 1 {
				removed, added := renderIntraLineDiff(
					replaceTabs(removedLines[0].content),
					replaceTabs(addedLines[0].content),
					style.Inverse,
				)
				result = append(result, style.Del.Render("-"+removedLines[0].lineNum+" "+removed))
				result = append(result, style.Add.Render("+"+addedLines[0].lineNum+" "+added))
			} else {
				for _, rl := range removedLines {
					result = append(result, style.Del.Render("-"+rl.lineNum+" "+replaceTabs(rl.content)))
				}
				for _, al := range addedLines {
					result = append(result, style.Add.Render("+"+al.lineNum+" "+replaceTabs(al.content)))
				}
			}
		} else if prefix == "+" {
			result = append(result, style.Add.Render("+"+lineNum+" "+replaceTabs(content)))
			i++
		} else {
			result = append(result, style.Context.Render(" "+lineNum+" "+replaceTabs(content)))
			i++
		}
	}
	return strings.Join(result, "\n")
}

// replaceTabs replaces tab characters with spaces for consistent rendering.
func replaceTabs(s string) string {
	return strings.ReplaceAll(s, "\t", "   ")
}

// ─── ToolOutput Renderer ───────────────────────────────────────────────────

// ToolOutput represents the rendered output of a tool execution.
type ToolOutput struct {
	ToolName string
	Args     string
	Output   string
	IsDiff   bool // true for edit/write tools that produce diffs
	IsError  bool
	Expanded bool
	Duration string
}

// RenderToolOutput renders a tool execution result.
func RenderToolOutput(out ToolOutput, width int) string {
	style := DefaultDiffStyle()

	var sb strings.Builder

	// Header: tool name + args summary + duration
	header := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#e5c07b")).
		Render("🔧 " + out.ToolName)
	if out.Args != "" {
		header += " " + lipgloss.NewStyle().
			Foreground(lipgloss.Color("#6c7086")).
			Render(truncate(out.Args, 60))
	}
	if out.Duration != "" {
		header += " " + lipgloss.NewStyle().
			Foreground(lipgloss.Color("#6c7086")).
			Render("(" + out.Duration + ")")
	}
	sb.WriteString(header)
	sb.WriteByte('\n')

	if !out.Expanded {
		sb.WriteString(lipgloss.NewStyle().
			Foreground(lipgloss.Color("#5c6370")).
			Render("  [expand to view output]"))
		sb.WriteByte('\n')
		return sb.String()
	}

	// Render output content
	output := out.Output
	if out.IsDiff {
		output = RenderDiff(output, style)
	} else if out.IsError {
		output = lipgloss.NewStyle().
			Foreground(lipgloss.Color("#e06c75")).
			Render(output)
	}

	// Indent output
	lines := strings.Split(output, "\n")
	maxLines := 50
	if len(lines) > maxLines {
		lines = lines[:maxLines]
		lines = append(lines, lipgloss.NewStyle().
			Foreground(lipgloss.Color("#e5c07b")).
			Render("  ... truncated ("+itoa(len(strings.Split(out.Output, "\n")))+" lines total)"))
	}
	for _, line := range lines {
		sb.WriteString("  ")
		sb.WriteString(line)
		sb.WriteByte('\n')
	}

	return sb.String()
}

func truncate(s string, max int) string {
	if len(s) <= max {
		return s
	}
	return s[:max-3] + "..."
}
