package tools

// ---------------------------------------------------------------------------
// Tool Operations interfaces
//
// These interfaces define the contract for concrete tool implementations.
// They allow swapping implementations (e.g., sandboxed vs. direct, in-memory
// vs. disk, real vs. mock for testing).
// ---------------------------------------------------------------------------

// BashOperations defines the interface for executing shell commands.
type BashOperations interface {
	// Execute runs a shell command and returns its output, exit code, and any error.
	// cmd is the command to run, cwd is the working directory, env is additional env vars.
	// Returns stdout, stderr, exit code, and any execution error.
	Execute(cmd, cwd string, env []string) (stdout, stderr string, exitCode int, err error)
}

// ReadOperations defines the interface for reading files.
type ReadOperations interface {
	// Read reads the full contents of a file.
	Read(path string) ([]byte, error)

	// FileExists checks whether a file exists at the given path.
	FileExists(path string) bool
}

// WriteOperations defines the interface for writing files.
type WriteOperations interface {
	// Write writes data to a file, creating or overwriting it.
	Write(path string, data []byte) error

	// MkdirAll creates a directory and all parent directories.
	MkdirAll(path string) error
}

// EditOperations combines read and write operations for file editing.
type EditOperations interface {
	ReadOperations
	WriteOperations
}

// GrepOpts holds options for grep-style search.
type GrepOpts struct {
	// IgnoreCase enables case-insensitive search.
	IgnoreCase bool

	// InvertMatch selects non-matching lines.
	InvertMatch bool

	// WholeWord matches only whole words.
	WholeWord bool

	// FilesWithMatches only prints file paths.
	FilesWithMatches bool

	// MaxCount limits the number of matches.
	MaxCount int

	// MaxDepth limits directory recursion depth.
	MaxDepth int

	// FileGlob filters files by glob pattern.
	FileGlob string
}

// GrepMatch represents a single grep match.
type GrepMatch struct {
	File    string `json:"file"`
	LineNum int    `json:"line_num"`
	Line    string `json:"line"`
	Column  int    `json:"column,omitempty"`
}

// GrepOperations defines the interface for searching file contents.
type GrepOperations interface {
	// Search searches for pattern in path (file or directory) with the given options.
	Search(pattern, path string, opts GrepOpts) ([]GrepMatch, error)
}
