package engine

import (
	"github.com/huichen/xihu/internal/tools"
	"github.com/huichen/xihu/pkg/types"
)

// ---------------------------------------------------------------------------
// Tool subsets
// ---------------------------------------------------------------------------

// CodingTools returns the default built-in coding tools:
// read, bash, edit, write (aligned with TS pi-mono).
func CodingTools() []types.AgentTool {
	return []types.AgentTool{
		tools.BashTool(),
		tools.ReadTool(),
		tools.WriteTool(),
		tools.EditTool(),
	}
}

// ReadOnlyTools returns a read-only subset of tools:
// read, grep, ls, find (no bash, write, or edit).
func ReadOnlyTools() []types.AgentTool {
	return []types.AgentTool{
		tools.ReadTool(),
		tools.GrepTool(),
		tools.LsTool(),
		tools.FindTool(),
	}
}
