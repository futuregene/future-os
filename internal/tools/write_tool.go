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
				Description: "Write content to a file. Creates the file if it doesn't exist, overwrites if it does. Automatically creates parent directories.",
				Parameters:  types.SchemaOf[WriteParams](),
			},
		},
		Guidelines: []string{
			"Use write only for new files or complete rewrites",
		},
		Handler: func(args json.RawMessage) (string, error) {
			var params struct {
				Path    string `json:"path"`
				Content string `json:"content"`
			}
			if err := json.Unmarshal(args, &params); err != nil {
				return "", err
			}
			if params.Path == "" {
				return "", fmt.Errorf("path is required")
			}
			dir := filepath.Dir(params.Path)
			if err := os.MkdirAll(dir, 0755); err != nil {
				return "", fmt.Errorf("create directories: %w", err)
			}
			if err := os.WriteFile(params.Path, []byte(params.Content), 0644); err != nil {
				return "", fmt.Errorf("write file: %w", err)
			}
			return fmt.Sprintf("Successfully wrote %d bytes to %s", len(params.Content), params.Path), nil
		},
	}
}
