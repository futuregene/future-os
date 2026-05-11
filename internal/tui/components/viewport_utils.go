package components

import (
	"fmt"
	"strconv"
	"strings"
	"unicode/utf8"

)

func wrapURLsOSC8(s string) string {
	if !strings.Contains(s, "http://") && !strings.Contains(s, "https://") {
		return s
	}

	var result strings.Builder
	i := 0
	for i < len(s) {
		// Look for next URL start
		rem := s[i:]
		httpIdx := strings.Index(rem, "http://")
		httpsIdx := strings.Index(rem, "https://")
		urlStart := -1
		if httpIdx >= 0 && (httpsIdx < 0 || httpIdx < httpsIdx) {
			urlStart = httpIdx
		} else if httpsIdx >= 0 {
			urlStart = httpsIdx
		}
		if urlStart < 0 {
			result.WriteString(rem)
			break
		}

		// Write everything before the URL
		result.WriteString(rem[:urlStart])
		urlRem := rem[urlStart:]

		// Extract URL characters (stop at whitespace, ANSI end, or special chars)
		urlEnd := 0
		inAnsi := false
		cleanURL := ""
		for j := 0; j < len(urlRem); j++ {
			ch := urlRem[j]
			if inAnsi {
				cleanURL += string(ch)
				if ch >= '@' && ch <= '~' {
					inAnsi = false
				}
				urlEnd = j + 1
				continue
			}
			if ch == '\x1b' && j+1 < len(urlRem) && urlRem[j+1] == '[' {
				inAnsi = true
				urlEnd = j + 1
				continue
			}
			if ch == ' ' || ch == '\t' || ch == '\n' || ch == '\r' ||
				ch == '"' || ch == '\'' || ch == '<' || ch == '>' ||
				ch == ')' || ch == ']' || ch == '}' {
				break
			}
			urlEnd = j + 1
		}

		if urlEnd > 0 {
			rawURL := urlRem[:urlEnd]
			// Strip ANSI codes to get the clean URL text
			clean := stripAnsiCodes(rawURL)
			// Wrap in OSC 8
			result.WriteString(fmt.Sprintf("\x1b]8;;%s\x1b\\%s\x1b]8;;\x1b\\", clean, rawURL))
		}
		i += urlStart + urlEnd
	}
	return result.String()
}

// stripAnsiCodes removes ANSI escape sequences from a string.
func stripAnsiCodes(s string) string {
	var b strings.Builder
	inAnsi := false
	for i := 0; i < len(s); i++ {
		if inAnsi {
			if s[i] >= '@' && s[i] <= '~' {
				inAnsi = false
			}
			continue
		}
		if s[i] == '\x1b' && i+1 < len(s) && s[i+1] == '[' {
			inAnsi = true
			i++ // skip '['
			continue
		}
		b.WriteByte(s[i])
	}
	return b.String()
}

// TruncateByWidth truncates a string to fit within a visual width,
// preserving ANSI escape sequences. Returns the truncated string.
func TruncateByWidth(s string, maxWidth int) string {
	if maxWidth <= 0 {
		return ""
	}
	visualWidth := 0
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
		w := 1
		if isWideRune(r) {
			w = 2
		}
		if visualWidth+w > maxWidth {
			return s[:i]
		}
		visualWidth += w
		i += size
	}
	return s
}
// toolArgsPreview returns a human-readable preview of tool arguments.

// toolIcon returns a per-tool icon matching TS pi-mono tool icons.
func toolIcon(name string) string {
	switch name {
	case "read", "read_file":
		return "📖 "
	case "edit", "patch":
		return "✏️ "
	case "write", "write_file":
		return "📝 "
	case "bash":
		return "💻 "
	case "grep":
		return "🔍 "
	case "ls":
		return "📂 "
	case "find":
		return "🔎 "
	case "web_search", "websearch":
		return "🌐 "
	case "web_fetch", "webfetch":
		return "📥 "
	case "notebook_edit", "notebookedit":
		return "📓 "
	default:
		return "🔧 "
	}
}

func toolArgsPreview(name, args string) string {
	switch name {
	case "read", "edit", "write":
		if path := extractJSONField(args, "file_path"); path != "" {
			return path + formatLineRange(args)
		}
	case "bash":
		if cmd := extractJSONField(args, "command"); cmd != "" {
			if len(cmd) > 60 {
				cmd = cmd[:60] + "..."
			}
			return cmd
		}
	case "grep":
		if pat := extractJSONField(args, "pattern"); pat != "" {
			return pat
		}
	case "ls":
		if dir := extractJSONField(args, "path"); dir != "" {
			return dir
		}
	}
	return ""
}

// toolResultDetail returns a tool-specific detail string for the collapsed view.
func toolResultDetail(e ChatEntry) string {
	switch e.ToolName {
	case "read", "edit", "write":
		if path := extractJSONField(e.ToolArgs, "file_path"); path != "" {
			detail := " " + path + formatLineRange(e.ToolArgs)
			if lc := lineCount(e.Content); lc > 0 {
				detail += fmt.Sprintf(" (%d lines)", lc)
			}
			return detail
		}
	case "bash":
		if cmd := extractJSONField(e.ToolArgs, "command"); cmd != "" {
			if len(cmd) > 60 {
				cmd = cmd[:60] + "..."
			}
			detail := " " + cmd
			if lc := lineCount(e.Content); lc > 0 {
				detail += fmt.Sprintf(" (%d lines)", lc)
			}
			return detail
		}
	case "grep":
		if pat := extractJSONField(e.ToolArgs, "pattern"); pat != "" {
			return " " + pat
		}
	case "ls", "find", "glob":
		if dir := extractJSONField(e.ToolArgs, "path"); dir != "" {
			return " " + dir
		}
	case "web_search":
		if query := extractJSONField(e.ToolArgs, "query"); query != "" {
			if len(query) > 60 {
				query = query[:60] + "..."
			}
			return " " + query
		}
	case "web_fetch":
		if url := extractJSONField(e.ToolArgs, "url"); url != "" {
			if len(url) > 60 {
				url = url[:60] + "..."
			}
			return " " + url
		}
	}
	return ""
}

// lineCount returns the number of lines in a string.
func isDiffContent(content string) bool {
	return strings.HasPrefix(content, "diff ") ||
		strings.Contains(content, "\n--- ") ||
		strings.Contains(content, "\n+++ ") ||
		strings.Contains(content, "\n@@ ")
}

// extractJSONField extracts a string field from a JSON object string.
func extractJSONField(jsonStr, field string) string {
	needle := `"` + field + `": "`
	idx := strings.Index(jsonStr, needle)
	if idx < 0 {
		return ""
	}
	start := idx + len(needle)
	end := strings.IndexByte(jsonStr[start:], '"')
	if end < 0 {
		return ""
	}
	return jsonStr[start : start+end]
}

// extractJSONIntField extracts an integer field from a JSON object string (TS pi-mono: line range parsing).
func extractJSONIntField(jsonStr, field string) (int, bool) {
	needle := `"` + field + `": `
	idx := strings.Index(jsonStr, needle)
	if idx < 0 {
		return 0, false
	}
	start := idx + len(needle)
	// Read until comma, whitespace, or closing brace
	end := start
	for end < len(jsonStr) && jsonStr[end] != ',' && jsonStr[end] != ' ' && jsonStr[end] != '\n' && jsonStr[end] != '}' {
		end++
	}
	val, err := strconv.Atoi(strings.TrimSpace(jsonStr[start:end]))
	if err != nil {
		return 0, false
	}
	return val, true
}

// formatLineRange returns a line range string like ":10-20" or ":51" matching TS pi-mono formatReadLineRange.
func formatLineRange(args string) string {
	offset, hasOffset := extractJSONIntField(args, "offset")
	limit, hasLimit := extractJSONIntField(args, "limit")
	if !hasOffset && !hasLimit {
		return ""
	}
	if !hasOffset {
		offset = 1
	}
	if hasLimit {
		return fmt.Sprintf(":%d-%d", offset, offset+limit-1)
	}
	return fmt.Sprintf(":%d", offset)
}

// padLineToWidth pads a line (which may contain ANSI codes) to the given visual width
// using background-colored spaces so the entire line has a uniform background.
func formatTokenCount(n int) string {
	if n <= 0 {
		return "?"
	}
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

// AppendCompactionSummary adds a compaction summary custom message with token count
// matching TS pi-mono CompactionSummaryMessageComponent.
