package tools

// SourceInfo carries metadata about where a file or resource originates.
type SourceInfo struct {
	Path    string // The target path where the resource is placed
	Source  string // The origin (e.g., a URL, package name, or relative path to a source file)
	Scope   string // Classification of the source (e.g., "builtin", "plugin", "user", "external")
	Origin  string // Descriptive origin (e.g., "repository", "generated", "uploaded")
	BaseDir string // Base directory from which relative paths are resolved
}

// NewSourceInfo creates a SourceInfo from the given path and optional
// configuration. Unset fields remain at their zero values.
func NewSourceInfo(path string, opts ...func(*SourceInfo)) SourceInfo {
	si := SourceInfo{Path: path}
	for _, o := range opts {
		o(&si)
	}
	return si
}

// WithSource sets the Source field.
func WithSource(s string) func(*SourceInfo) {
	return func(si *SourceInfo) { si.Source = s }
}

// WithScope sets the Scope field.
func WithScope(s string) func(*SourceInfo) {
	return func(si *SourceInfo) { si.Scope = s }
}

// WithOrigin sets the Origin field.
func WithOrigin(s string) func(*SourceInfo) {
	return func(si *SourceInfo) { si.Origin = s }
}

// WithBaseDir sets the BaseDir field.
func WithBaseDir(s string) func(*SourceInfo) {
	return func(si *SourceInfo) { si.BaseDir = s }
}
