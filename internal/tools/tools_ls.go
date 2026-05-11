package tools

import (
	"encoding/json"
	"fmt"
	"os"
	"sort"
	"strings"

	"github.com/huichen/xihu/pkg/types"
)

// LsTool lists directory contents alphabetically (case-insensitive), directories
// get a / suffix, limited to a configurable number of entries (default 500).
// TS pi-mono aligned: no file sizes, pure alphabetical sorting.
func LsTool() types.AgentTool {
	return types.AgentTool{
		Def: types.ToolDef{
			Type: "function",
			Function: types.FunctionDef{
				Name:        "ls",
				Description: "List directory contents. Returns entries sorted alphabetically, with '/' suffix for directories. Includes dotfiles. Output is truncated to 500 entries.",
				Parameters:  types.SchemaOf[lsParams](),
			},
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

			if len(entries) == 0 {
				return "(empty directory)", nil
			}

			type entry struct {
				Name  string
				IsDir bool
			}

			var list []entry
			for _, e := range entries {
				list = append(list, entry{Name: e.Name(), IsDir: e.IsDir()})
			}

			// Sort alphabetically, case-insensitive (TS pi-mono aligned)
			sort.Slice(list, func(i, j int) bool {
				return strings.ToLower(list[i].Name) < strings.ToLower(list[j].Name)
			})

			limit := len(list)
			truncated := false
			if limit > params.Limit {
				limit = params.Limit
				truncated = true
			}

			var sb strings.Builder
			for i := 0; i < limit; i++ {
				e := list[i]
				suffix := ""
				if e.IsDir {
					suffix = "/"
				}
				sb.WriteString(e.Name + suffix + "\n")
			}

			if truncated {
				sb.WriteString(fmt.Sprintf("[%d entries limit reached. Use limit=%d for more]\n", params.Limit, params.Limit*2))
			}

			return sb.String(), nil
		},
	}
}
