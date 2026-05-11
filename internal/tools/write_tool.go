package tools

import (
"encoding/json"
"fmt"
"os"
"path/filepath"

"github.com/huichen/xihu/pkg/types"
)

func WriteTool() types.AgentTool {
	return types.AgentTool{
		Def: types.ToolDef{
			Type: "function",
			Function: types.FunctionDef{
				Name:        "write",
				Description: "Write content to a file, overwriting if it exists",
				Parameters: types.SchemaOf[WriteParams](),
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
