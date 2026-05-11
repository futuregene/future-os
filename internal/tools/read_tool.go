package tools

import (
"encoding/base64"
"encoding/json"
"fmt"
"net/http"
"os"
"path/filepath"
"strings"
"unicode/utf8"

"github.com/huichen/xihu/pkg/types"
)

func ReadTool() types.AgentTool {
	return types.AgentTool{
		Def: types.ToolDef{
			Type: "function",
			Function: types.FunctionDef{
				Name:        "read",
				Description: "Read the contents of a file. For text files, returns numbered lines (optionally offset/limit). For image files, returns a base64 data URL.",
				Parameters: types.SchemaOf[ReadParams](),
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

