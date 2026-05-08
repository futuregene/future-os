package diagnostic

// DiagnosticType classifies diagnostics emitted during operations.
type DiagnosticType string

const (
	Warning   DiagnosticType = "warning"
	Error     DiagnosticType = "error"
	Collision DiagnosticType = "collision"
)

// CollisionDetail describes a file collision.
type CollisionDetail struct {
	WinnerPath   string `json:"winner_path"`
	LoserPath    string `json:"loser_path"`
	WinnerSource string `json:"winner_source"`
	LoserSource  string `json:"loser_source"`
}

// Diagnostic represents a diagnostic event (warning, error, or collision).
type Diagnostic struct {
	Type         DiagnosticType   `json:"type"`
	Message      string           `json:"message"`
	ResourceType string           `json:"resource_type"`
	Name         string           `json:"name"`
	Collision    *CollisionDetail `json:"collision,omitempty"`
}

// NewDiagnostic creates a generic diagnostic.
func NewDiagnostic(typ DiagnosticType, msg, resourceType, name string) Diagnostic {
	return Diagnostic{
		Type:         typ,
		Message:      msg,
		ResourceType: resourceType,
		Name:         name,
	}
}

// NewWarning creates a warning diagnostic.
func NewWarning(msg, resourceType, name string) Diagnostic {
	return NewDiagnostic(Warning, msg, resourceType, name)
}

// NewError creates an error diagnostic.
func NewError(msg, resourceType, name string) Diagnostic {
	return NewDiagnostic(Error, msg, resourceType, name)
}

// NewCollision creates a collision diagnostic.
func NewCollision(msg, resourceType, name string, detail CollisionDetail) Diagnostic {
	return Diagnostic{
		Type:         Collision,
		Message:      msg,
		ResourceType: resourceType,
		Name:         name,
		Collision:    &detail,
	}
}
