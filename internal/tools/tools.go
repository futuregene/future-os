package tools
import (
	"context"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/huichen/cobalt/pkg/types"
	"golang.org/x/text/unicode/norm"
)

// Bash tool: run shell commands
func BashTool() types.AgentTool {
	return types.AgentTool{
		Def: types.ToolDef{
			Type: "function",
			Function: types.FunctionDef{
				Name:        "bash",
				Description: "Execute a shell command in the project directory",
				Parameters: json.RawMessage(`{
					"type": "object",
					"properties": {
						"command": {"type": "string", "description": "The shell command to execute"},
						"timeout": {"type": "integer", "description": "Optional timeout in seconds. If not specified, runs with no timeout."}
					},
					"required": ["command"]
				}`),
			},
		},
		Guidelines: []string{
			"Prefer one bash command per turn",
			"Check exit codes",
		},
		Handler: func(args json.RawMessage) (string, error) {
			var params struct {
				Command string `json:"command"`
				Timeout int    `json:"timeout"`
			}
			if err := json.Unmarshal(args, &params); err != nil {
				return "", err
			}

			// Timeout is optional; if unspecified or <= 0, run without timeout.
			// The context serves as an AbortSignal that can cancel the command.
			var ctx context.Context
			var cancel context.CancelFunc
			if params.Timeout > 0 {
				ctx, cancel = context.WithTimeout(context.Background(), time.Duration(params.Timeout)*time.Second)
			} else {
				ctx, cancel = context.WithCancel(context.Background())
			}
			defer cancel()

			cmd := exec.CommandContext(ctx, "bash", "-c", params.Command)
			cmd.Dir, _ = os.Getwd()
			out, err := cmd.CombinedOutput()

			// Determine exit code
			exitCode := 0
			if cmd.ProcessState != nil {
				exitCode = cmd.ProcessState.ExitCode()
			} else if err != nil {
				exitCode = -1
			}

			// Handle timeout case
			if ctx.Err() == context.DeadlineExceeded {
				if exitCode == 0 {
					exitCode = -1
				}
			}

			fullOutput := string(out)
			const spillThreshold = 10000
			const tailBytes = 50000

			var result strings.Builder

			// Line 1: exit code
			fmt.Fprintf(&result, "exit code: %d\n", exitCode)

			// Line 2 (optional): temp file path if output exceeds spill threshold
			if len(fullOutput) > spillThreshold {
				tmpFile, tmpErr := os.CreateTemp("", "pi-bash-*.txt")
				if tmpErr != nil {
					fmt.Fprintf(&result, "[failed to spill output: %v]\n", tmpErr)
				} else {
					if _, tmpErr := tmpFile.WriteString(fullOutput); tmpErr != nil {
						tmpFile.Close()
						os.Remove(tmpFile.Name())
						fmt.Fprintf(&result, "[failed to spill output: %v]\n", tmpErr)
					} else {
						tmpFile.Close()
						fmt.Fprintf(&result, "[full output at %s]\n", tmpFile.Name())
					}
				}
			}

			// Line 3+: output — truncated to last tailBytes bytes
			if len(fullOutput) > tailBytes {
				fullOutput = fullOutput[len(fullOutput)-tailBytes:]
			}
			result.WriteString(fullOutput)

			return result.String(), nil
		},
	}
}

// Read tool: read file contents (supports text with line numbers and images as base64)
func ReadTool() types.AgentTool {
	return types.AgentTool{
		Def: types.ToolDef{
			Type: "function",
			Function: types.FunctionDef{
				Name:        "read",
				Description: "Read the contents of a file. For text files, returns numbered lines (optionally offset/limit). For image files, returns a base64 data URL.",
				Parameters: json.RawMessage(`{
					"type": "object",
					"properties": {
						"file_path": {"type": "string", "description": "Path to the file to read"},
						"path": {"type": "string", "description": "Alias for file_path"},
						"offset": {"type": "integer", "description": "Line number to start reading from (1-indexed)"},
						"limit": {"type": "integer", "description": "Maximum number of lines to read"}
					},
					"required": []
				}`),
			},
		},
		Guidelines: []string{
			"Read files before editing them",
		},
		Handler: func(args json.RawMessage) (string, error) {
			var params struct {
				FilePath string `json:"file_path"`
				Path     string `json:"path"`
				Offset   int    `json:"offset"`
				Limit    int    `json:"limit"`
			}
			if err := json.Unmarshal(args, &params); err != nil {
				return "", err
			}
			// Accept 'path' as alias for 'file_path'
			if params.FilePath == "" {
				params.FilePath = params.Path
			}
			if params.FilePath == "" {
				return "", fmt.Errorf("file_path is required")
			}
			if params.Limit == 0 {
				params.Limit = 500
			}
			if params.Offset == 0 {
				params.Offset = 1
			}

			data, err := os.ReadFile(params.FilePath)
			if err != nil {
				return "", fmt.Errorf("read file: %w", err)
			}

			// Image detection by extension
			ext := strings.ToLower(filepath.Ext(params.FilePath))
			imageExts := map[string]bool{
				".png": true, ".jpg": true, ".jpeg": true, ".gif": true,
				".webp": true, ".bmp": true, ".svg": true, ".ico": true, ".tiff": true,
			}
			if imageExts[ext] {
				mimeType := http.DetectContentType(data)
				b64 := base64.StdEncoding.EncodeToString(data)
				filename := filepath.Base(params.FilePath)
				return fmt.Sprintf("[Image: %s, size: %d bytes, base64: data:%s;base64,%s]",
					filename, len(data), mimeType, b64), nil
			}

			lines := strings.Split(string(data), "\n")
			if params.Offset > len(lines) {
				return "", nil
			}
			end := params.Offset + params.Limit
			if end > len(lines) {
				end = len(lines)
			}

			var sb strings.Builder
			for i := params.Offset - 1; i < end; i++ {
				fmt.Fprintf(&sb, "%d\t%s\n", i+1, lines[i])
			}

			result := sb.String()
			const maxChars = 50000
			if len(result) > maxChars {
				truncated := result[:maxChars]
				// Walk back to nearest UTF-8 rune boundary
				for len(truncated) > 0 {
					r, size := utf8.DecodeLastRuneInString(truncated)
					if r != utf8.RuneError || size != 1 {
						break
					}
					truncated = truncated[:len(truncated)-1]
				}
				nextOffset := params.Offset + strings.Count(truncated, "\n")
				result = truncated + fmt.Sprintf("\n[truncated — use offset=%d to continue]", nextOffset)
			}
			return result, nil
		},
	}
}

// Write tool: write content to a file
func WriteTool() types.AgentTool {
	return types.AgentTool{
		Def: types.ToolDef{
			Type: "function",
			Function: types.FunctionDef{
				Name:        "write",
				Description: "Write content to a file, overwriting if it exists",
				Parameters: json.RawMessage(`{
					"type": "object",
					"properties": {
						"file_path": {"type": "string", "description": "Path to the file to write"},
						"content": {"type": "string", "description": "Content to write to the file"}
					},
					"required": ["file_path", "content"]
				}`),
			},
		},
		Guidelines: []string{
			"Create parent directories automatically",
		},
		Handler: func(args json.RawMessage) (string, error) {
			var params struct {
				FilePath string `json:"file_path"`
				Content  string `json:"content"`
			}
			if err := json.Unmarshal(args, &params); err != nil {
				return "", err
			}
			dir := filepath.Dir(params.FilePath)
			if err := os.MkdirAll(dir, 0755); err != nil {
				return "", fmt.Errorf("create directories: %w", err)
			}
			if err := os.WriteFile(params.FilePath, []byte(params.Content), 0644); err != nil {
				return "", fmt.Errorf("write file: %w", err)
			}
			return fmt.Sprintf("Wrote %d bytes to %s", len(params.Content), params.FilePath), nil
		},
	}
}

// ---------------------------------------------------------------------------
// Fuzzy matching helpers
// ---------------------------------------------------------------------------

// smartQuoteReplacer replaces Unicode smart quotes with ASCII equivalents.
var smartQuoteReplacer = strings.NewReplacer(
	"\u201c", "\"", // left double quotation mark
	"\u201d", "\"", // right double quotation mark
	"\u2018", "'",  // left single quotation mark
	"\u2019", "'",  // right single quotation mark
	"\uff02", "\"", // fullwidth quotation mark
)

// normalize applies fuzzy matching transformations:
//   - NFKC Unicode normalization (combining characters, fullwidth forms, etc.)
//   - Replace smart quotes with ASCII
//   - Strip trailing whitespace from each line
func normalize(s string) string {
	// NFKC normalization: decompose and recompose characters, convert fullwidth
	// to halfwidth, etc. This handles cases like ﬃ (ligature) vs ffi, fullwidth
	// Latin letters, combining diacritics, and more.
	s = norm.NFKC.String(s)

	lines := strings.Split(s, "\n")
	for i, line := range lines {
		lines[i] = strings.TrimRight(smartQuoteReplacer.Replace(line), " \t")
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

// editOp represents a single edit operation.
type editOp struct {
	OldString string `json:"old_string"`
	NewString string `json:"new_string"`
}

// matchRegion records the byte range of a match in the original content.
type matchRegion struct {
	editIdx  int
	oldStart int
	oldEnd   int
}

// EditTool returns the enhanced Edit tool.
func EditTool() types.AgentTool {
	return types.AgentTool{
		Def: types.ToolDef{
			Type: "function",
			Function: types.FunctionDef{
				Name: "edit",
				Description: "Make targeted edits to a file. Supports single edit (old_string/new_string) or multi-edit (edits array). Uses fuzzy matching for smart quotes and trailing whitespace.",
				Parameters: json.RawMessage(`{
					"type": "object",
					"properties": {
						"file_path": {"type": "string", "description": "Path to the file to edit"},
						"old_string": {"type": "string", "description": "Exact text to find and replace. Must be unique unless replace_all is true."},
						"new_string": {"type": "string", "description": "Replacement text. Can be empty to delete the matched text."},
						"oldText": {"type": "string", "description": "Alias for old_string (legacy compatibility)"},
						"newText": {"type": "string", "description": "Alias for new_string (legacy compatibility)"},
						"replace_all": {"type": "boolean", "description": "Replace all occurrences instead of requiring uniqueness (default: false)"},
						"edits": {"type": "array", "items": {"type": "object", "properties": {"old_string": {"type": "string"}, "new_string": {"type": "string"}}, "required": ["old_string", "new_string"]}, "description": "Array of edits for multi-edit mode"}
					},
					"required": ["file_path"]
				}`),
			},
		},
		Guidelines: []string{
			"Include enough context lines for unique matching",
			"Use replace_all for global changes",
		},
		Handler: func(args json.RawMessage) (string, error) {
			// --- Parse parameters ---
			var params struct {
				FilePath   string          `json:"file_path"`
				OldString  string          `json:"old_string"`
				NewString  string          `json:"new_string"`
				OldText    string          `json:"oldText"`
				NewText    string          `json:"newText"`
				ReplaceAll bool            `json:"replace_all"`
				Edits      json.RawMessage `json:"edits"`
			}
			if err := json.Unmarshal(args, &params); err != nil {
				return "", err
			}
			if params.FilePath == "" {
				return "", fmt.Errorf("file_path is required")
			}

			// Legacy alias fallback
			if params.OldString == "" && params.OldText != "" {
				params.OldString = params.OldText
			}
			if params.NewString == "" && params.NewText != "" {
				params.NewString = params.NewText
			}

			// --- Build edits list ---
			var edits []editOp
			if len(params.Edits) > 0 {
				if err := json.Unmarshal(params.Edits, &edits); err != nil {
					return "", fmt.Errorf("invalid edits: %w", err)
				}
				if len(edits) == 0 {
					return "", fmt.Errorf("edits array is empty")
				}
			} else {
				if params.OldString == "" {
					return "", fmt.Errorf("old_string (or oldText) is required in single-edit mode")
				}
				edits = []editOp{{OldString: params.OldString, NewString: params.NewString}}
			}

			// --- Read file ---
			data, err := os.ReadFile(params.FilePath)
			if err != nil {
				return "", fmt.Errorf("read file: %w", err)
			}

			// --- BOM handling ---
			bom := []byte{0xEF, 0xBB, 0xBF}
			hasBOM := len(data) >= 3 && data[0] == bom[0] && data[1] == bom[1] && data[2] == bom[2]
			originalContent := string(data)
			if hasBOM {
				originalContent = string(data[3:])
			}

			// --- Normalize content + build mapper ---
			normalizedContent := normalize(originalContent)
			mapper := buildByteMapper(originalContent, normalizedContent)

			// --- Find all matches ---
			var matches []matchRegion
			for ei, edit := range edits {
				normOld := normalize(edit.OldString)
				if normOld == "" {
					return "", fmt.Errorf("edit[%d]: old_string normalizes to empty string", ei)
				}

				searchStart := 0
				for {
					pos := strings.Index(normalizedContent[searchStart:], normOld)
					if pos < 0 {
						break
					}
					absPos := searchStart + pos
					origStart := mapper[absPos]
					origEnd := mapper[absPos+len(normOld)]

					matches = append(matches, matchRegion{
						editIdx:  ei,
						oldStart: origStart,
						oldEnd:   origEnd,
					})

					// Advance past this match
					searchStart = absPos + len(normOld)
					if !params.ReplaceAll && len(edits) == 1 {
						// Single-edit non-replace_all: only first match
						break
					}
				}
			}

			// --- Verify matches found ---
			if len(matches) == 0 {
				return "", fmt.Errorf("no matches found for any edit in %s", params.FilePath)
			}

			// --- Single-edit uniqueness check ---
			if len(edits) == 1 && !params.ReplaceAll {
				if len(matches) > 1 {
					return "", fmt.Errorf("old_string matches %d times in %s — use replace_all=true or add more context lines", len(matches), params.FilePath)
				}
			}

			// --- Overlap detection (multi-edit) ---
			if len(edits) > 1 {
				for i := 0; i < len(matches); i++ {
					for j := i + 1; j < len(matches); j++ {
						a, b := matches[i], matches[j]
						// Check if [a.start, a.end) overlaps [b.start, b.end)
						if a.oldStart < b.oldEnd && b.oldStart < a.oldEnd {
							return "", fmt.Errorf(
								"overlapping edits: edit[%d] at bytes [%d,%d) and edit[%d] at bytes [%d,%d)",
								a.editIdx, a.oldStart, a.oldEnd,
								b.editIdx, b.oldStart, b.oldEnd,
							)
						}
					}
				}
			}

			// --- Apply replacements (reverse order to preserve positions) ---
			// Sort matches by start position descending
			sortMatches(matches)

			result := originalContent
			totalReplacements := 0
			skippedNoChange := 0
			for _, m := range matches {
				edit := edits[m.editIdx]

				// No-change detection: if the matched text is identical to new_string,
				// skip this replacement to avoid unnecessary file writes.
				matchedText := originalContent[m.oldStart:m.oldEnd]
				if matchedText == edit.NewString {
					skippedNoChange++
					continue
				}

				result = result[:m.oldStart] + edit.NewString + result[m.oldEnd:]
				totalReplacements++
			}

			// If all replacements were skipped due to no-change, report it
			if totalReplacements == 0 && skippedNoChange > 0 {
				return fmt.Sprintf("No changes needed: all %d edit(s) already match the target content in %s", skippedNoChange, params.FilePath), nil
			}

			// --- Restore BOM ---
			output := result
			if hasBOM {
				output = string(bom) + result
			}

			// --- Write file ---
			if err := os.WriteFile(params.FilePath, []byte(output), 0644); err != nil {
				return "", fmt.Errorf("write file: %w", err)
			}

			// --- Generate unified diff ---
			diff := generateUnifiedDiff(params.FilePath, originalContent, result)

			// --- Build response ---
			var sb strings.Builder
			if len(edits) == 1 {
				fmt.Fprintf(&sb, "Edited %s: %d replacement(s)\n", params.FilePath, totalReplacements)
			} else {
				fmt.Fprintf(&sb, "Multi-edited %s: %d edit(s) applied, %d total replacement(s)\n",
					params.FilePath, len(edits), totalReplacements)
			}
			if diff != "" {
				sb.WriteString(diff)
			}

			return sb.String(), nil
		},
	}
}

// sortMatches sorts match regions by start position descending (for reverse-order application).
func sortMatches(matches []matchRegion) {
	for i := 0; i < len(matches)-1; i++ {
		for j := i + 1; j < len(matches); j++ {
			if matches[j].oldStart > matches[i].oldStart {
				matches[i], matches[j] = matches[j], matches[i]
			}
		}
	}
}

// grepParams holds parameters for the Grep tool.
type grepParams struct {
	Pattern    string `json:"pattern"`
	Path       string `json:"path"`
	Glob       string `json:"glob"`
	IgnoreCase bool   `json:"ignoreCase"`
	Literal    bool   `json:"literal"`
	Context    int    `json:"context"`
	Limit      int    `json:"limit"`
}

// Grep tool: search file contents with regex. Uses ripgrep (rg) with --json
// if available for structured output; falls back to system grep otherwise.
func GrepTool() types.AgentTool {
	return types.AgentTool{
		Def: types.ToolDef{
			Type: "function",
			Function: types.FunctionDef{
				Name:        "grep",
				Description: "Search for a pattern in file contents. Uses ripgrep with structured JSON output if available; falls back to system grep.",
				Parameters: json.RawMessage(`{
					"type": "object",
					"properties": {
						"pattern": {"type": "string", "description": "Regex pattern to search for"},
						"path": {"type": "string", "description": "Directory or file to search in (default: current directory)"},
						"glob": {"type": "string", "description": "File pattern filter (e.g., '*.go')"},
						"ignoreCase": {"type": "boolean", "description": "Case-insensitive search (default: false)"},
						"literal": {"type": "boolean", "description": "Treat pattern as a literal string, not a regex (default: false)"},
						"context": {"type": "integer", "description": "Number of context lines before and after each match (default: 0)"},
						"limit": {"type": "integer", "description": "Maximum number of matching lines to return (default: 100)"}
					},
					"required": ["pattern"]
				}`),
			},
		},
		Guidelines: []string{
			"Use grep before reading to find the right file",
		},
		Handler: func(args json.RawMessage) (string, error) {
			var params grepParams
			if err := json.Unmarshal(args, &params); err != nil {
				return "", err
			}
			if params.Path == "" {
				params.Path = "."
			}
			if params.Limit <= 0 {
				params.Limit = 100
			}

			// Try ripgrep first
			if rgPath, err := exec.LookPath("rg"); err == nil {
				return grepViaRipgrep(rgPath, params)
			}
			return grepViaSystem(params)
		},
	}
}

// grepViaRipgrep runs rg --json and parses structured output.
func grepViaRipgrep(rgPath string, params grepParams) (string, error) {
	args := []string{"--json", "--no-heading", "--color", "never",
		"--max-count", fmt.Sprintf("%d", params.Limit+1)} // +1 to detect overflow
	if params.IgnoreCase {
		args = append(args, "-i")
	}
	if params.Literal {
		args = append(args, "-F")
	}
	if params.Context > 0 {
		args = append(args, "-C", fmt.Sprintf("%d", params.Context))
	}
	if params.Glob != "" {
		args = append(args, "-g", params.Glob)
	}
	args = append(args, params.Pattern, params.Path)

	cmd := exec.Command(rgPath, args...)
	out, err := cmd.CombinedOutput()
	exitCode := 0
	if cmd.ProcessState != nil {
		exitCode = cmd.ProcessState.ExitCode()
	}
	// rg exits 1 when no matches
	if exitCode == 1 {
		return "No matches found\n", nil
	}
	// rg exits 2 on error
	if exitCode == 2 {
		return "", fmt.Errorf("ripgrep error: %s", string(out))
	}
	// Ignore other non-zero exits (e.g. broken pipe)
	_ = err

	return parseRipgrepJSON(string(out), params.Limit)
}

// parseRipgrepJSON parses rg --json output lines into formatted output.
func parseRipgrepJSON(output string, limit int) (string, error) {
	type submatch struct {
		Match struct {
			Text string `json:"text"`
		} `json:"match"`
		Start int `json:"start"`
		End   int `json:"end"`
	}
	type lineData struct {
		Path struct {
			Text string `json:"text"`
		} `json:"path"`
		Lines struct {
			Text string `json:"text"`
		} `json:"lines"`
		LineNumber int        `json:"line_number"`
		Submatches []submatch `json:"submatches"`
	}
	type jsonLine struct {
		Type string   `json:"type"`
		Data lineData `json:"data"`
	}

	rawLines := strings.Split(strings.TrimSpace(output), "\n")
	var sb strings.Builder
	matchCount := 0

	// Buffer to collect context + match lines per file group
	type outLine struct {
		prefix   string // ":" for match, "-" for context
		path     string
		lineNum  int
		text     string
	}
	var group []outLine

	flushGroup := func() {
		for _, ol := range group {
			fmt.Fprintf(&sb, "%s%s:%d:%s\n", ol.prefix, ol.path, ol.lineNum, ol.text)
		}
		group = nil
	}

	for _, raw := range rawLines {
		if raw == "" {
			continue
		}
		var jl jsonLine
		if err := json.Unmarshal([]byte(raw), &jl); err != nil {
			continue
		}

		switch jl.Type {
		case "begin":
			// Start of a new file — flush previous group
			flushGroup()
		case "context":
			text := strings.TrimSuffix(jl.Data.Lines.Text, "\n")
			text = truncateLine(text, 500)
			group = append(group, outLine{"-", jl.Data.Path.Text, jl.Data.LineNumber, text})
		case "match":
			if matchCount >= limit {
				continue
			}
			text := strings.TrimSuffix(jl.Data.Lines.Text, "\n")
			text = truncateLine(text, 500)
			group = append(group, outLine{":", jl.Data.Path.Text, jl.Data.LineNumber, text})
			matchCount++
		}
	}
	flushGroup()

	// Build result
	var result strings.Builder
	if matchCount == 0 {
		result.WriteString("No matches found\n")
	} else {
		fmt.Fprintf(&result, "Found %d matches\n", matchCount)
		result.WriteString(sb.String())
		if matchCount >= limit {
			fmt.Fprintf(&result, "[results limited to %d matches]\n", limit)
		}
	}
	return result.String(), nil
}

// grepViaSystem falls back to traditional grep.
func grepViaSystem(params grepParams) (string, error) {
	args := []string{"-rn", "--color=never"}
	if params.IgnoreCase {
		args = append(args, "-i")
	}
	if params.Literal {
		args = append(args, "-F")
	} else {
		args = append(args, "-E")
	}
	if params.Context > 0 {
		args = append(args, "-C", fmt.Sprintf("%d", params.Context))
	}
	if params.Limit > 0 {
		args = append(args, "-m", fmt.Sprintf("%d", params.Limit+1)) // +1 to detect overflow
	}
	if params.Glob != "" {
		args = append(args, "--include="+params.Glob)
	}
	args = append(args, params.Pattern, params.Path)

	cmd := exec.Command("grep", args...)
	out, err := cmd.CombinedOutput()
	exitCode := 0
	if cmd.ProcessState != nil {
		exitCode = cmd.ProcessState.ExitCode()
	}
	if exitCode == 1 {
		return "No matches found\n", nil
	}
	if exitCode > 1 {
		return "", fmt.Errorf("grep error (exit %d): %s", exitCode, string(out))
	}
	_ = err

	return parseSystemGrepOutput(string(out), params.Limit)
}

// parseSystemGrepOutput parses traditional grep -C output.
// Context lines: file-line-text; match lines: file:line:text; groups separated by --
func parseSystemGrepOutput(output string, limit int) (string, error) {
	lines := strings.Split(strings.TrimSpace(output), "\n")
	var result strings.Builder
	matchCount := 0

	for _, line := range lines {
		if line == "--" {
			continue
		}

		// Determine if this is a match line or context line.
		// Format: "file:line:text" for matches, "file-line-text" for context (grep -C)
		// We need to distinguish. Grep emits "filename-line-text" for context and "filename:line:text" for matches.
		colonIdx := strings.Index(line, ":")
		if colonIdx < 0 {
			continue
		}

		// Heuristic: after the file path, if there's a colon then a number then a colon, it's a match.
		// If there's a hyphen then a number then a colon/hyphen, it's context.
		// Check: find the first colon. Before it is the file path.
		// Then check if the next char after colon is a digit.
		filePart := line[:colonIdx]
		rest := line[colonIdx+1:]

		if len(rest) == 0 {
			continue
		}

		// Find the line number: digits after the separator
		isMatch := true // colon-separated -> match by default in grep -rn output
		var lineNumStr string
		for i, c := range rest {
			if c >= '0' && c <= '9' {
				lineNumStr += string(c)
			} else {
				// The separator after line number determines match vs context
				if c == '-' {
					isMatch = false
				}
				rest = rest[i+1:] // skip past the separator
				break
			}
		}

		if lineNumStr == "" {
			continue
		}
		lineNum := 0
		fmt.Sscanf(lineNumStr, "%d", &lineNum)

		if isMatch {
			if matchCount >= limit {
				continue
			}
			matchCount++
		}

		prefix := ":"
		if !isMatch {
			prefix = "-"
		}

		text := truncateLine(rest, 500)
		fmt.Fprintf(&result, "%s%s:%d:%s\n", prefix, filePart, lineNum, text)
	}

	var header strings.Builder
	if matchCount == 0 {
		return "No matches found\n", nil
	}
	fmt.Fprintf(&header, "Found %d matches\n", matchCount)
	header.WriteString(result.String())
	if matchCount >= limit {
		fmt.Fprintf(&header, "[results limited to %d matches]\n", limit)
	}
	return header.String(), nil
}

// truncateLine truncates s to maxLen chars, appending "[truncated]" if needed.
func truncateLine(s string, maxLen int) string {
	if len(s) <= maxLen {
		return s
	}
	return s[:maxLen] + "[truncated]"
}

// AllTools returns all available tools
func AllTools() []types.AgentTool {
	return []types.AgentTool{
		BashTool(),
		ReadTool(),
		WriteTool(),
		EditTool(),
		GrepTool(),
		LsTool(),
		FindTool(),
	}
}
