package tools

import (
	"strings"
	"unicode/utf8"

	"golang.org/x/text/unicode/norm"
	"fmt"
)

var smartQuoteReplacer = strings.NewReplacer(
	"\u201c", "\"", // left double quotation mark
	"\u201d", "\"", // right double quotation mark
	"\u2018", "'",  // left single quotation mark
	"\u2019", "'",  // right single quotation mark
	"\uff02", "\"", // fullwidth quotation mark
)

// dashReplacer normalizes various Unicode dash characters to ASCII "-".
// #15 Unicode normalization: dashes.
var dashReplacer = strings.NewReplacer(
	"\u2010", "-", // hyphen
	"\u2011", "-", // non-breaking hyphen
	"\u2012", "-", // figure dash
	"\u2013", "-", // en dash
	"\u2014", "-", // em dash
	"\u2015", "-", // horizontal bar
	"\u2212", "-", // minus sign
)

// spaceReplacer normalizes various Unicode space characters to ASCII " ".
// #15 Unicode normalization: spaces.
var spaceReplacer = strings.NewReplacer(
	"\u00A0", " ", // no-break space
	"\u2002", " ", // en space
	"\u2003", " ", // em space
	"\u2004", " ", // three-per-em space
	"\u2005", " ", // four-per-em space
	"\u2006", " ", // six-per-em space
	"\u2007", " ", // figure space
	"\u2008", " ", // punctuation space
	"\u2009", " ", // thin space
	"\u200A", " ", // hair space
	"\u202F", " ", // narrow no-break space
	"\u205F", " ", // medium mathematical space
	"\u3000", " ", // ideographic space
)

// unicodeDashSet tracks which runes normalize to "-" (for use in buildByteMapper).
var unicodeDashSet = map[rune]bool{
	'\u2010': true, '\u2011': true, '\u2012': true, '\u2013': true,
	'\u2014': true, '\u2015': true, '\u2212': true,
}

// unicodeSpaceSet tracks which runes normalize to " " (for use in buildByteMapper).
var unicodeSpaceSet = map[rune]bool{
	'\u00A0': true, '\u2002': true, '\u2003': true, '\u2004': true,
	'\u2005': true, '\u2006': true, '\u2007': true, '\u2008': true,
	'\u2009': true, '\u200A': true, '\u202F': true, '\u205F': true,
	'\u3000': true,
}

// normalize applies fuzzy matching transformations:
//   - NFKC Unicode normalization (combining characters, fullwidth forms, etc.)
//   - Replace smart quotes with ASCII
//   - Replace Unicode dashes with ASCII "-" (see dashReplacer)
//   - Replace Unicode spaces with ASCII " " (see spaceReplacer)
//   - Strip trailing whitespace from each line
func normalize(s string) string {
	// NFKC normalization: decompose and recompose characters, convert fullwidth
	// to halfwidth, etc. This handles cases like ﬃ (ligature) vs ffi, fullwidth
	// Latin letters, combining diacritics, and more.
	s = norm.NFKC.String(s)

	lines := strings.Split(s, "\n")
	for i, line := range lines {
		line = smartQuoteReplacer.Replace(line)
		line = dashReplacer.Replace(line)
		line = spaceReplacer.Replace(line)
		lines[i] = strings.TrimRight(line, " \t")
	}
	return strings.Join(lines, "\n")
}

// buildByteMapper maps byte indices from normalized content back to original content.
// normalized is derived from original via normalize(), which only removes or shrinks
// characters (never adds), so mapper[normIdx] = corresponding byte position in original.
func buildByteMapper(orig, norm string) []int {
	mapper := make([]int, len(norm)+1)
	oi, ni := 0, 0

	for ni < len(norm) && oi < len(orig) {
		mapper[ni] = oi

		if orig[oi] == norm[ni] {
			oi++
			ni++
			continue
		}

		// Check for smart-quote → ASCII replacement (3-byte UTF-8 → 1-byte ASCII)
		if oi+2 < len(orig) {
			seq := orig[oi : oi+3]
			var repl byte
			switch seq {
			case "\u201c", "\u201d", "\uff02":
				repl = '"'
			case "\u2018", "\u2019":
				repl = '\''
			}
			if repl != 0 && ni < len(norm) && norm[ni] == repl {
				oi += 3
				ni++
				continue
			}
		}

		// Check for Unicode dash/space → ASCII replacement (multi-byte → 1-byte)
		if oi < len(orig) {
			r, size := utf8.DecodeRuneInString(orig[oi:])
			if size > 1 {
				if unicodeDashSet[r] && ni < len(norm) && norm[ni] == '-' {
					oi += size
					ni++
					continue
				}
				if unicodeSpaceSet[r] && ni < len(norm) && norm[ni] == ' ' {
					oi += size
					ni++
					continue
				}
			}
		}

		// Trailing whitespace stripped from line: skip whitespace in original
		if (orig[oi] == ' ' || orig[oi] == '\t') && ni < len(norm) && norm[ni] == '\n' {
			oi++
			continue
		}
		if (orig[oi] == ' ' || orig[oi] == '\t') && ni >= len(norm) {
			oi++
			continue
		}

		// Fallback: advance both (should not normally happen)
		oi++
		ni++
	}

	// Consume any trailing whitespace in original (end-of-file without newline)
	for oi < len(orig) && (orig[oi] == ' ' || orig[oi] == '\t') {
		oi++
	}

	// Fill remaining mapper entries
	for idx := ni; idx <= len(norm); idx++ {
		mapper[idx] = oi
	}

	return mapper
}

// ---------------------------------------------------------------------------
// CRLF handling (#14)
// ---------------------------------------------------------------------------

// detectLineEnding detects whether content uses "\r\n" or "\n" line endings.
func detectLineEnding(content string) string {
	if strings.Contains(content, "\r\n") {
		return "\r\n"
	}
	return "\n"
}

// normalizeToLF replaces CRLF line endings with LF.
func normalizeToLF(content string) string {
	return strings.ReplaceAll(content, "\r\n", "\n")
}

// restoreLineEndings restores original line endings in content.
func restoreLineEndings(content string, lineEnding string) string {
	if lineEnding == "\r\n" {
		return strings.ReplaceAll(content, "\n", "\r\n")
	}
	return content
}

// ---------------------------------------------------------------------------
// Unified diff helpers
// ---------------------------------------------------------------------------

// generateUnifiedDiff produces a unified diff with 3 context lines.
func generateUnifiedDiff(filePath string, oldContent, newContent string) string {
	oldLines := strings.Split(oldContent, "\n")
	newLines := strings.Split(newContent, "\n")

	// Find first differing line
	first := 0
	for first < len(oldLines) && first < len(newLines) && oldLines[first] == newLines[first] {
		first++
	}

	// Find last differing line (scan backwards)
	lastOld := len(oldLines) - 1
	lastNew := len(newLines) - 1
	for lastOld >= first && lastNew >= first && oldLines[lastOld] == newLines[lastNew] {
		lastOld--
		lastNew--
	}

	// If nothing changed, return empty diff
	if first > lastOld && first > lastNew {
		return ""
	}

	const ctx = 3
	hunkStart := first - ctx
	if hunkStart < 0 {
		hunkStart = 0
	}
	hunkOldEnd := lastOld + ctx
	if hunkOldEnd >= len(oldLines) {
		hunkOldEnd = len(oldLines) - 1
	}
	hunkNewEnd := lastNew + ctx
	if hunkNewEnd >= len(newLines) {
		hunkNewEnd = len(newLines) - 1
	}

	oldHunkCount := hunkOldEnd - hunkStart + 1
	newHunkCount := hunkNewEnd - hunkStart + 1

	var sb strings.Builder
	fmt.Fprintf(&sb, "--- a/%s\n", filePath)
	fmt.Fprintf(&sb, "+++ b/%s\n", filePath)
	fmt.Fprintf(&sb, "@@ -%d,%d +%d,%d @@\n", hunkStart+1, oldHunkCount, hunkStart+1, newHunkCount)

	oi, ni := hunkStart, hunkStart
	for oi <= hunkOldEnd || ni <= hunkNewEnd {
		oldLine := ""
		newLine := ""
		if oi <= hunkOldEnd {
			oldLine = oldLines[oi]
		}
		if ni <= hunkNewEnd {
			newLine = newLines[ni]
		}

		if oi <= lastOld && ni <= lastNew && oldLine == newLine {
			// In the changed region but lines happen to be equal
			fmt.Fprintf(&sb, " %s\n", oldLine)
			oi++
			ni++
		} else if oi <= lastOld && (ni > lastNew || oi <= hunkOldEnd) {
			// Remove old line (but not if we're past the diff and in trailing context)
			if oi >= first && oi <= lastOld {
				fmt.Fprintf(&sb, "-%s\n", oldLine)
				oi++
			} else if ni >= first && ni <= lastNew {
				fmt.Fprintf(&sb, "+%s\n", newLine)
				ni++
			} else {
				fmt.Fprintf(&sb, " %s\n", oldLine)
				oi++
				if ni <= hunkNewEnd && oi-1 == ni {
					ni++
				}
			}
		} else if ni <= lastNew {
			fmt.Fprintf(&sb, "+%s\n", newLine)
			ni++
		} else {
			fmt.Fprintf(&sb, " %s\n", oldLine)
			oi++
			ni++
		}
	}

	return sb.String()
}

// ---------------------------------------------------------------------------
// Edit tool: targeted find-and-replace with fuzzy matching, multi-edit, etc.
// ---------------------------------------------------------------------------

