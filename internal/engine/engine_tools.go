package engine

import (
	"github.com/huichen/xihu/internal/tools"
	"github.com/huichen/xihu/pkg/types"
)

// ---------------------------------------------------------------------------
// Tool subsets
// ---------------------------------------------------------------------------

// CodingTools returns the full set of built-in coding tools:
// bash, read, write, edit, grep, ls, find.
func CodingTools() []types.AgentTool {
	return []types.AgentTool{
		tools.BashTool(),
		tools.ReadTool(),
		tools.WriteTool(),
		tools.EditTool(),
		tools.GrepTool(),
		tools.LsTool(),
		tools.FindTool(),
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
