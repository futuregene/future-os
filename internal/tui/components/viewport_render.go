package components

import (
	"strings"
	"unicode/utf8"

	"github.com/charmbracelet/lipgloss"
)

func (c *ChatViewport) RenderBorder(width int) string {
	if width < 2 {
		width = 2
	}
	return c.borderStyle.Render(strings.Repeat("─", width))
}

// hasToolCalls checks if the text entry at idx has tool calls in the same message block.
// In pi-mono, OSC 133 zones are only applied when the message has no tool calls.
func (c *ChatViewport) hasToolCalls(idx int) bool {
	for j := idx + 1; j < len(c.entries); j++ {
		switch c.entries[j].Type {
		case "user_message":
			return false // reached next message, no tool calls found
		case "tool_call":
			return true
		}
	}
	return false
}

// AppendText adds a text chunk (or appends to the last text entry).
func visibleWidth(s string) int {
	w := 0
	inAnsi := false
	for i := 0; i < len(s); {
		if inAnsi {
			if s[i] >= '@' && s[i] <= '~' {
				inAnsi = false
			}
			i++
			continue
		}
		if s[i] == '\x1b' && i+1 < len(s) && s[i+1] == '[' {
			inAnsi = true
			i += 2
			continue
		}
		r, size := utf8.DecodeRuneInString(s[i:])
		if isWideRune(r) {
			w += 2
		} else {
			w += 1
		}
		i += size
	}
	return w
}

// isWideRune returns true for CJK ideographs, hangul, kana, and emoji that
// occupy two terminal columns.
func isWideRune(r rune) bool {
	if r >= 0x1100 && r <= 0x115f ||
		r == 0x2329 || r == 0x232a ||
		r >= 0x2e80 && r <= 0xa4cf ||
		r >= 0xac00 && r <= 0xd7a3 ||
		r >= 0xf900 && r <= 0xfaff ||
		r >= 0xfe10 && r <= 0xfe19 ||
		r >= 0xfe30 && r <= 0xfe6f ||
		r >= 0xff00 && r <= 0xff60 ||
		r >= 0xffe0 && r <= 0xffe6 ||
		r >= 0x1f300 && r <= 0x1f64f ||
		r >= 0x1f680 && r <= 0x1f6ff ||
		r >= 0x1f900 && r <= 0x1f9ff {
		return true
	}
	return false
}

// needsBlankLine returns true if a blank line should separate two consecutive entries (TS pi-mono: Spacer(1)).
func needsBlankLine(current, next ChatEntry) bool {
	// Blank line between distinct message blocks: user→assistant, assistant→next message,
	// tool results→next, bash→next, compaction→next, etc.
	// Blank line between: user_msg→anything, anything→next distinct block
	if current.Type == "user_message" {
		return true
	}
	if current.Type == "text" || current.Type == "thinking" {
		return next.Type != "thinking" // no blank between consecutive thinking blocks
	}
	if current.Type == "tool_result" || current.Type == "tool_call" {
		return next.Type != "tool_result" && next.Type != "tool_call"
	}
	if current.Type == "bash" {
		return next.Type != "bash" // TS pi-mono: trailing \n in bash component provides spacing
	}
	if current.Type == "custom_message" || current.Type == "system" ||
		current.Type == "error" || current.Type == "warning" {
		return true
	}
	return false
}

// normalizeTerminalOutput decomposes Thai/Lao AM vowels to avoid stale-cell
// artifacts in terminal renderers during differential repaint (TS pi-mono).
func normalizeTerminalOutput(s string) string {
	if !strings.ContainsRune(s, 'ำ') && !strings.ContainsRune(s, 'ຳ') {
		return s
	}
	s = strings.ReplaceAll(s, "ำ", "ํา")
	s = strings.ReplaceAll(s, "ຳ", "ໍາ")
	return s
}

// wordWrap wraps text at word boundaries (spaces), never breaking mid-word
// unless a single word exceeds the width. ANSI escape sequences are skipped
// when measuring width.
func wordWrap(s string, width int) string {
	if width <= 0 {
		return s
	}
	var result strings.Builder

	writeBreak := func(cur *int) {
		result.WriteByte('\n')
		*cur = 0
	}

	lines := strings.Split(s, "\n")
	for li, line := range lines {
		if li > 0 {
			result.WriteByte('\n')
		}
		vw := visibleWidth(line)
		if vw <= width {
			result.WriteString(line)
			continue
		}
		// Tokenise into (word, whitespace) pairs so we can wrap at spaces.
		cur := 0
		var tok strings.Builder
		flush := func() {
			if tok.Len() == 0 {
				return
			}
			t := tok.String()
			tw := visibleWidth(t)
			tok.Reset()
			if tw > width {
				// Word longer than line — force-break by character.
				breakLongWord(&result, t, width, &cur)
				return
			}
			if cur+tw > width && cur > 0 {
				writeBreak(&cur)
			}
			result.WriteString(t)
			cur += tw
		}
		for i := 0; i < len(line); {
			if line[i] == ' ' {
				flush()
				if cur >= width {
					writeBreak(&cur)
					// skip leading space on fresh line
					i++
					continue
				}
				result.WriteByte(' ')
				cur++
				i++
				continue
			}
			// Collect ANSI codes (attach to current token)
			if line[i] == '\x1b' && i+1 < len(line) && line[i+1] == '[' {
				end := i + 2
				for end < len(line) && !(line[end] >= '@' && line[end] <= '~') {
					end++
				}
				if end < len(line) {
					end++
				}
				tok.WriteString(line[i:end])
				i = end
				continue
			}
			r, size := utf8.DecodeRuneInString(line[i:])
			tok.WriteRune(r)
			i += size
		}
		flush()
	}
	return result.String()
}

// breakLongWord breaks a single token that exceeds width by inserting newlines,
// preserving any embedded ANSI codes.
func breakLongWord(rb *strings.Builder, token string, width int, cur *int) {
	col := *cur
	for i := 0; i < len(token); {
		// Collect any ANSI prefix
		ansiPrefix := ""
		for i < len(token) && token[i] == '\x1b' && i+1 < len(token) && token[i+1] == '[' {
			end := i + 2
			for end < len(token) && !(token[end] >= '@' && token[end] <= '~') {
				end++
			}
			if end < len(token) {
				end++
			}
			ansiPrefix += token[i:end]
			i = end
		}
		if i >= len(token) {
			break
		}
		r, size := utf8.DecodeRuneInString(token[i:])
		rw := 1
		if isWideRune(r) {
			rw = 2
		}
		if col+rw > width && col > 0 {
			rb.WriteByte('\n')
			col = 0
			if ansiPrefix != "" {
				rb.WriteString(ansiPrefix)
			}
		}
		if ansiPrefix != "" {
			rb.WriteString(ansiPrefix)
		}
		rb.WriteRune(r)
		col += rw
		i += size
	}
	*cur = col
}

// applyLineBg pads each line to the given width with spaces then wraps it in a
// background style, matching TS pi-mono applyBackgroundToLine. Multi-line input
// (separated by \n) is processed line-by-line.
func applyLineBg(s string, width int, style lipgloss.Style) string {
	var sb strings.Builder
	for _, line := range strings.Split(s, "\n") {
		vw := visibleWidth(line)
		if vw < width {
			line += strings.Repeat(" ", width-vw)
		}
		sb.WriteString(style.Render(line))
		sb.WriteByte('\n')
	}
	return strings.TrimSuffix(sb.String(), "\n")
}

// prefixedLineBg applies a prefix to every line before calling applyLineBg.
// Useful for content margins inside background-colored blocks.
func prefixedLineBg(prefix, content string, width int, style lipgloss.Style) string {
	lines := strings.Split(content, "\n")
	for i, l := range lines {
		lines[i] = prefix + l
	}
	return applyLineBg(strings.Join(lines, "\n"), width, style)
}

// wrapURLsOSC8 wraps bare http/https URLs in OSC 8 hyperlink sequences (TS pi-mono).
// Handles ANSI escape codes that may be interleaved within URLs.
func lineCount(s string) int {
	if s == "" {
		return 0
	}
	n := strings.Count(s, "\n") + 1
	// Don't count trailing empty line
	if strings.HasSuffix(s, "\n") {
		n--
	}
	return n
}

// isDiffContent detects whether content looks like a unified diff.
func padLineToWidth(line string, width int, bg lipgloss.Style) string {
	vw := lipgloss.Width(line)
	if vw < width {
		line += bg.Render(strings.Repeat(" ", width-vw))
	}
	return line
}

// formatTokenCount formats a token count with comma separators (matching TS pi-mono toLocaleString).
