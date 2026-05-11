package tools

import (
	"encoding/json"
	"fmt"
	"os"
	"strings"

	"github.com/huichen/xihu/pkg/types"
)

type editOp struct {
	OldText string `json:"oldText"`
	NewText string `json:"newText"`
}

// matchRegion records the byte range of a match in the original content.
type matchRegion struct {
	editIdx  int
	oldStart int
	oldEnd   int
}

// EditTool returns the enhanced Edit tool (TS pi-mono aligned).
func EditTool() types.AgentTool {
	return types.AgentTool{
		Def: types.ToolDef{
			Type: "function",
			Function: types.FunctionDef{
				Name:        "edit",
				Description: "Edit a single file using exact text replacement. Every edits[].oldText must match a unique, non-overlapping region of the original file. Supports multi-edit via edits array. Uses fuzzy matching for smart quotes and trailing whitespace.",
				Parameters:  types.SchemaOf[EditParams](),
			},
		},
		Guidelines: []string{
			"Include enough context lines for unique matching",
		},
		Handler: func(args json.RawMessage) (string, error) {
			// --- Parse parameters ---
			var params struct {
				FilePath  string          `json:"path"`
				OldString string          `json:"old_string"` // legacy alias
				NewString string          `json:"new_string"` // legacy alias
				OldText   string          `json:"oldText"`
				NewText   string          `json:"newText"`
				Edits     json.RawMessage `json:"edits"`
			}
			if err := json.Unmarshal(args, &params); err != nil {
				return "", err
			}
			if params.FilePath == "" {
				return "", fmt.Errorf("path is required")
			}

			// Legacy alias: old_string → oldText, new_string → newText
			if params.OldText == "" && params.OldString != "" {
				params.OldText = params.OldString
			}
			if params.NewText == "" && params.NewString != "" {
				params.NewText = params.NewString
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
				if params.OldText == "" {
					return "", fmt.Errorf("oldText (or old_string) is required in single-edit mode")
				}
				edits = []editOp{{OldText: params.OldText, NewText: params.NewText}}
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
			lineEnding := detectLineEnding(originalContent)
			originalContent = normalizeToLF(originalContent)

			// --- Normalize content + build mapper ---
			normalizedContent := normalize(originalContent)
			mapper := buildByteMapper(originalContent, normalizedContent)

			// --- Find all matches ---
			var matches []matchRegion
			for ei, edit := range edits {
				normOld := normalize(edit.OldText)
				if normOld == "" {
					return "", fmt.Errorf("edit[%d]: oldText normalizes to empty string", ei)
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

					// Each edit matches exactly once (TS pi-mono behavior)
					break
				}
			}

			// --- Verify matches found ---
			if len(matches) == 0 {
				return "", fmt.Errorf("no matches found for any edit in %s", params.FilePath)
			}

			// --- Overlap detection (multi-edit) ---
			if len(edits) > 1 {
				for i := 0; i < len(matches); i++ {
					for j := i + 1; j < len(matches); j++ {
						a, b := matches[i], matches[j]
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
			sortMatches(matches)

			result := originalContent
			totalReplacements := 0
			skippedNoChange := 0
			for _, m := range matches {
				edit := edits[m.editIdx]
				matchedText := originalContent[m.oldStart:m.oldEnd]
				if matchedText == edit.NewText {
					skippedNoChange++
					continue
				}
				result = result[:m.oldStart] + edit.NewText + result[m.oldEnd:]
				totalReplacements++
			}

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

			// --- Build response (TS pi-mono aligned) ---
			var sb strings.Builder
			if len(edits) == 1 {
				fmt.Fprintf(&sb, "Successfully replaced 1 block(s) in %s.\n", params.FilePath)
			} else {
				fmt.Fprintf(&sb, "Successfully replaced %d block(s) in %s.\n", totalReplacements, params.FilePath)
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
