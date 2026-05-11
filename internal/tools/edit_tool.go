package tools

import (
"encoding/json"
"fmt"
"os"
"strings"

"github.com/huichen/xihu/pkg/types"
)

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
				Parameters: types.SchemaOf[EditParams](),
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

			// --- Line ending normalization (#14) ---
			// Detect original line endings, then normalize to LF for consistent matching.
			lineEnding := detectLineEnding(originalContent)
			originalContent = normalizeToLF(originalContent)

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

			// --- Restore line endings (#14) ---
			result = restoreLineEndings(result, lineEnding)

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

// Grep tool: search file contents with regex. Uses ripgrep (rg) with --json
