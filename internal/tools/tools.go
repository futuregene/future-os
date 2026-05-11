package tools

import (
	"encoding/json"

	"github.com/huichen/xihu/pkg/types"
)

// ─── Tool parameter types — replaces hand-written JSON Schema strings ────────
// Each type defines both the Go struct for json.Unmarshal AND
// the JSON Schema via types.SchemaOf[T]() (mirroring TS pi-mono's TypeBox).

type BashParams struct {
	Command string `json:"command" jsonschema:"required,description=The shell command to execute"`
	Timeout int    `json:"timeout,omitempty" jsonschema:"description=Optional timeout in seconds"`
}

type ReadParams struct {
	Path   string `json:"path,omitempty" jsonschema:"description=Path to the file to read (primary)"`
	Offset int    `json:"offset,omitempty" jsonschema:"description=Line number to start reading from (1-indexed)"`
	Limit  int    `json:"limit,omitempty" jsonschema:"description=Maximum number of lines to read"`
}

type WriteParams struct {
	Path    string `json:"path" jsonschema:"required,description=Path to the file to write"`
	Content string `json:"content" jsonschema:"required,description=Content to write to the file"`
}

type EditParams struct {
	Path    string          `json:"path" jsonschema:"required,description=Path to the file to edit"`
	OldText string          `json:"oldText,omitempty" jsonschema:"description=Exact text to find and replace"`
	NewText string          `json:"newText,omitempty" jsonschema:"description=Replacement text (empty to delete)"`
	Edits   json.RawMessage `json:"edits,omitempty" jsonschema:"description=Array of {oldText, newText} for multi-edit mode"`
}

type grepParams struct {
	Pattern    string `json:"pattern" jsonschema:"required,description=Regex pattern to search for"`
	Path       string `json:"path,omitempty" jsonschema:"description=Directory or file to search in"`
	Glob       string `json:"glob,omitempty" jsonschema:"description=File pattern filter (e.g. *.go)"`
	IgnoreCase bool   `json:"ignoreCase,omitempty" jsonschema:"description=Case-insensitive search"`
	Literal    bool   `json:"literal,omitempty" jsonschema:"description=Treat pattern as literal string"`
	Context    int    `json:"context,omitempty" jsonschema:"description=Context lines before/after each match"`
	Limit      int    `json:"limit,omitempty" jsonschema:"description=Max matching lines to return (default: 100)"`
}

type lsParams struct {
	Path  string `json:"path,omitempty" jsonschema:"description=Directory path to list"`
	Limit int    `json:"limit,omitempty" jsonschema:"description=Max entries (default: 500)"`
}

type findParams struct {
	Pattern string `json:"pattern,omitempty" jsonschema:"description=Glob pattern (e.g. *.go)"`
	Path    string `json:"path,omitempty" jsonschema:"description=Directory to search from"`
	Limit   int    `json:"limit,omitempty" jsonschema:"description=Max results (default: 1000)"`
}

func AllTools() []types.AgentTool {
	return []types.AgentTool{
		BashTool(),
		ReadTool(),
		WriteTool(),
		EditTool(),
		GrepTool(),
		LsTool(),
		FindTool(),
	}
}
