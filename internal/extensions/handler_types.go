package extensions

// =============================================================================
// Typed event handler with return values — mirrors pi-mono ExtensionHandler
// =============================================================================

// Each handler type below corresponds to a pi-mono event handler.
// Handlers return nil to pass through, or a result to modify/cancel/block.

// ToolCallEvent is the payload for tool_call handlers.
type ToolCallEvent struct {
	ToolName string
	Args     interface{}
}

// ToolCallResult can block or allow tool execution.
type ToolCallResult struct {
	Block  bool   // if true, tool execution is blocked
	Reason string // reason for blocking
}

// ToolCallHandler mirrors pi-mono: (event: ToolCallEvent, ctx) => ToolCallEventResult
type ToolCallHandler func(event ToolCallEvent) *ToolCallResult

// ToolResultEvent is the payload for tool_result handlers.
type ToolResultEvent struct {
	ToolName string
	Content  string
	IsError  bool
}

// ToolResultResult can replace the tool result content.
type ToolResultResult struct {
	Content string // replacement content
	IsError bool   // modified error flag
}

type ToolResultHandler func(event ToolResultEvent) *ToolResultResult

// InputEvent is the payload for input handlers.
type InputEvent struct {
	Text   string
	Images []interface{}
	Source string
}

// InputResultAction mirrors pi-mono InputEventResult.
type InputResultAction string

const (
	InputContinue  InputResultAction = "continue"
	InputTransform InputResultAction = "transform"
	InputHandled   InputResultAction = "handled"
)

// InputResult can transform or short-circuit user input.
type InputResult struct {
	Action InputResultAction
	Text   string
}

type InputHandler func(event InputEvent) *InputResult

// ContextEvent is the payload for context handlers.
type ContextEvent struct {
	MessageCount int
}

// ContextResult can replace the messages array.
type ContextResult struct {
	// Messages replacement (nil = no change)
}

type ContextHandler func(event ContextEvent) *ContextResult

// BeforeProviderRequestEvent is the payload for before_provider_request handlers.
type BeforeProviderRequestEvent struct {
	Payload interface{}
}

// BeforeProviderRequestResult replaces the request payload.
type BeforeProviderRequestResult struct {
	Payload interface{}
}

type BeforeProviderRequestHandler func(event BeforeProviderRequestEvent) *BeforeProviderRequestResult

// BeforeAgentStartEvent is the payload for before_agent_start handlers.
type BeforeAgentStartEvent struct {
	SystemPrompt string
	UserMessage  string
}

// BeforeAgentStartResult can modify the system prompt or inject messages.
type BeforeAgentStartResult struct {
	SystemPrompt string
	Message      string
}

type BeforeAgentStartHandler func(event BeforeAgentStartEvent) *BeforeAgentStartResult

// MessageEndEvent is the payload for message_end handlers.
type MessageEndEvent struct {
	Role string
}

// MessageEndResult can replace the message.
type MessageEndResult struct {
	Role string
}

type MessageEndHandler func(event MessageEndEvent) *MessageEndResult

// UserBashEvent is the payload for user_bash handlers.
type UserBashEvent struct {
	Command string
	CWD     string
}

// UserBashResult can provide custom bash execution.
type UserBashResult struct {
	Output   string
	ExitCode int
}

type UserBashHandler func(event UserBashEvent) *UserBashResult

// ModelSelectEvent is the payload for model_select handlers.
type ModelSelectEvent struct {
	Model         string
	PreviousModel string
	Source        string
}

type ModelSelectHandler func(event ModelSelectEvent)

// ThinkingLevelSelectEvent is the payload for thinking_level_select handlers.
type ThinkingLevelSelectEvent struct {
	Level         string
	PreviousLevel string
}

type ThinkingLevelSelectHandler func(event ThinkingLevelSelectEvent)

// SessionBeforeSwitchEvent is the payload for session_before_switch handlers.
type SessionBeforeSwitchEvent struct {
	TargetSessionFile string
}

// SessionBeforeSwitchResult can cancel the switch.
type SessionBeforeSwitchResult struct {
	Cancel bool
}

type SessionBeforeSwitchHandler func(event SessionBeforeSwitchEvent) *SessionBeforeSwitchResult

// SessionBeforeForkEvent is the payload for session_before_fork handlers.
type SessionBeforeForkEvent struct {
	EntryID string
}

// SessionBeforeForkResult can cancel the fork.
type SessionBeforeForkResult struct {
	Cancel bool
}

type SessionBeforeForkHandler func(event SessionBeforeForkEvent) *SessionBeforeForkResult

// SessionBeforeCompactEvent is the payload for session_before_compact handlers.
type SessionBeforeCompactEvent struct {
	CustomInstructions string
}

// SessionBeforeCompactResult can cancel or provide custom compaction.
type SessionBeforeCompactResult struct {
	Cancel bool
}

type SessionBeforeCompactHandler func(event SessionBeforeCompactEvent) *SessionBeforeCompactResult

// SessionShutdownEvent is the payload for session_shutdown handlers.
type SessionShutdownEvent struct {
	Reason string
}

type SessionShutdownHandler func(event SessionShutdownEvent)
