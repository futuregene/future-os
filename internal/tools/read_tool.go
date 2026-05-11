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
				Description: "Read the contents of a file. Supports text files and images (png, jpg, gif, webp, bmp, svg, ico, tiff). Images are returned as base64 data URLs. For text files, output is truncated to 50000 chars (use offset to continue).",
				Parameters:  types.SchemaOf[ReadParams](),
			},
		},
		Guidelines: []string{
			"Use read to examine files instead of cat or sed",
		},
		Handler: func(args json.RawMessage) (string, error) {
			var params struct {
				Path     string `json:"path"`
				FilePath string `json:"file_path"` // legacy alias
				Offset   int    `json:"offset"`
				Limit    int    `json:"limit"`
			}
			if err := json.Unmarshal(args, &params); err != nil {
				return "", err
			}
			// Accept 'path' (primary, TS pi-mono) with 'file_path' as legacy alias
			if params.Path == "" {
				params.Path = params.FilePath
			}
			if params.Path == "" {
				return "", fmt.Errorf("path is required")
			}
			if params.Limit == 0 {
				params.Limit = 500
			}
			if params.Offset == 0 {
				params.Offset = 1
			}

			data, err := os.ReadFile(params.Path)
			if err != nil {
				return "", fmt.Errorf("read file: %w", err)
			}

			// Image detection by extension
			ext := strings.ToLower(filepath.Ext(params.Path))
			imageExts := map[string]bool{
				".png": true, ".jpg": true, ".jpeg": true, ".gif": true,
				".webp": true, ".bmp": true, ".svg": true, ".ico": true, ".tiff": true,
			}
			if imageExts[ext] {
				mimeType := http.DetectContentType(data)
				b64 := base64.StdEncoding.EncodeToString(data)
				filename := filepath.Base(params.Path)
				return fmt.Sprintf("[Image: %s, size: %d bytes, base64: data:%s;base64,%s]",
					filename, len(data), mimeType, b64), nil
			}

			lines := strings.Split(string(data), "\n")
			if params.Offset > len(lines) {
				return "", fmt.Errorf("offset %d is beyond end of file (%d lines)", params.Offset, len(lines))
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
