package tools

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/huichen/xihu/pkg/types"
)

// LsTool lists directory contents: dirs first (with / suffix), alphabetical,
// shows sizes, limited to a configurable number of entries (default 500).
func LsTool() types.AgentTool {
	return types.AgentTool{
		Def: types.ToolDef{
			Type: "function",
			Function: types.FunctionDef{
				Name:        "ls",
				Description: "List directory contents. Sorted alphabetically with directories first and a trailing / suffix. Shows file sizes. Configurable limit (default 500).",
				Parameters: types.SchemaOf[lsParams](),
			},
		},
		Guidelines: []string{
			"Use ls to explore directory structure before reading files",
		},
		Handler: func(args json.RawMessage) (string, error) {
			var params struct {
				Path  string `json:"path"`
				Limit int    `json:"limit"`
			}
			if err := json.Unmarshal(args, &params); err != nil {
				return "", err
			}
			if params.Path == "" {
				params.Path = "."
			}
			if params.Limit <= 0 {
				params.Limit = 500
			}

			entries, err := os.ReadDir(params.Path)
			if err != nil {
				return "", fmt.Errorf("read directory %s: %w", params.Path, err)
			}

			type entry struct {
				Name  string
				IsDir bool
				Size  int64
			}

			var list []entry
			for _, e := range entries {
				info, err := e.Info()
				size := int64(0)
				if err == nil {
					size = info.Size()
				}
				list = append(list, entry{Name: e.Name(), IsDir: e.IsDir(), Size: size})
			}

			// Sort: dirs first, then alphabetical
			sort.Slice(list, func(i, j int) bool {
				if list[i].IsDir != list[j].IsDir {
					return list[i].IsDir
				}
				return strings.ToLower(list[i].Name) < strings.ToLower(list[j].Name)
			})

			limit := len(list)
			truncated := false
			if limit > params.Limit {
				limit = params.Limit
				truncated = true
			}

			var sb strings.Builder
			absPath, _ := filepath.Abs(params.Path)
			sb.WriteString(fmt.Sprintf("%s:\n", absPath))

			for i := 0; i < limit; i++ {
				e := list[i]
				suffix := ""
				if e.IsDir {
					suffix = "/"
				}
				sb.WriteString(fmt.Sprintf("%s%s  (%d)\n", e.Name, suffix, e.Size))
			}

			if truncated {
				sb.WriteString(fmt.Sprintf("... and %d more entries\n", len(list)-params.Limit))
			}

			return sb.String(), nil
		},
	}
}