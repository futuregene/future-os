package tools

import (
"encoding/json"
"fmt"
"os/exec"
"strings"

"github.com/huichen/xihu/pkg/types"
)

func GrepTool() types.AgentTool {
	return types.AgentTool{
		Def: types.ToolDef{
			Type: "function",
			Function: types.FunctionDef{
				Name:        "grep",
				Description: "Search for a pattern in file contents. Uses ripgrep with structured JSON output if available; falls back to system grep.",
				Parameters: types.SchemaOf[grepParams](),
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
