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

// =============================================================================
// HandlerRegistry — stores and invokes typed event handlers
// =============================================================================

// HandlerRegistry stores typed event handlers and invokes them when events fire.
// It mirrors pi-mono's extension.handlers Map<string, HandlerFn[]>.
type HandlerRegistry struct {
	toolCall             []ToolCallHandler
	toolResult           []ToolResultHandler
	input                []InputHandler
	context              []ContextHandler
	beforeProviderReq    []BeforeProviderRequestHandler
	beforeAgentStart     []BeforeAgentStartHandler
	messageEnd           []MessageEndHandler
	userBash             []UserBashHandler
	modelSelect          []ModelSelectHandler
	thinkingLevelSelect  []ThinkingLevelSelectHandler
	sessionBeforeSwitch  []SessionBeforeSwitchHandler
	sessionBeforeFork    []SessionBeforeForkHandler
	sessionBeforeCompact []SessionBeforeCompactHandler
	sessionShutdown      []SessionShutdownHandler
}

// NewHandlerRegistry creates a new HandlerRegistry.
func NewHandlerRegistry() *HandlerRegistry {
	return &HandlerRegistry{}
}

// AddToolCallHandler registers a tool_call handler.
func (h *HandlerRegistry) AddToolCallHandler(handler ToolCallHandler) {
	h.toolCall = append(h.toolCall, handler)
}

// AddToolResultHandler registers a tool_result handler.
func (h *HandlerRegistry) AddToolResultHandler(handler ToolResultHandler) {
	h.toolResult = append(h.toolResult, handler)
}

// AddInputHandler registers an input handler.
func (h *HandlerRegistry) AddInputHandler(handler InputHandler) {
	h.input = append(h.input, handler)
}

// AddContextHandler registers a context handler.
func (h *HandlerRegistry) AddContextHandler(handler ContextHandler) {
	h.context = append(h.context, handler)
}

// AddBeforeProviderRequestHandler registers a before_provider_request handler.
func (h *HandlerRegistry) AddBeforeProviderRequestHandler(handler BeforeProviderRequestHandler) {
	h.beforeProviderReq = append(h.beforeProviderReq, handler)
}

// AddBeforeAgentStartHandler registers a before_agent_start handler.
func (h *HandlerRegistry) AddBeforeAgentStartHandler(handler BeforeAgentStartHandler) {
	h.beforeAgentStart = append(h.beforeAgentStart, handler)
}

// AddMessageEndHandler registers a message_end handler.
func (h *HandlerRegistry) AddMessageEndHandler(handler MessageEndHandler) {
	h.messageEnd = append(h.messageEnd, handler)
}

// AddUserBashHandler registers a user_bash handler.
func (h *HandlerRegistry) AddUserBashHandler(handler UserBashHandler) {
	h.userBash = append(h.userBash, handler)
}

// AddModelSelectHandler registers a model_select handler.
func (h *HandlerRegistry) AddModelSelectHandler(handler ModelSelectHandler) {
	h.modelSelect = append(h.modelSelect, handler)
}

// AddThinkingLevelSelectHandler registers a thinking_level_select handler.
func (h *HandlerRegistry) AddThinkingLevelSelectHandler(handler ThinkingLevelSelectHandler) {
	h.thinkingLevelSelect = append(h.thinkingLevelSelect, handler)
}

// AddSessionBeforeSwitchHandler registers a session_before_switch handler.
func (h *HandlerRegistry) AddSessionBeforeSwitchHandler(handler SessionBeforeSwitchHandler) {
	h.sessionBeforeSwitch = append(h.sessionBeforeSwitch, handler)
}

// AddSessionBeforeForkHandler registers a session_before_fork handler.
func (h *HandlerRegistry) AddSessionBeforeForkHandler(handler SessionBeforeForkHandler) {
	h.sessionBeforeFork = append(h.sessionBeforeFork, handler)
}

// AddSessionBeforeCompactHandler registers a session_before_compact handler.
func (h *HandlerRegistry) AddSessionBeforeCompactHandler(handler SessionBeforeCompactHandler) {
	h.sessionBeforeCompact = append(h.sessionBeforeCompact, handler)
}

// AddSessionShutdownHandler registers a session_shutdown handler.
func (h *HandlerRegistry) AddSessionShutdownHandler(handler SessionShutdownHandler) {
	h.sessionShutdown = append(h.sessionShutdown, handler)
}

// InvokeToolCall runs all tool_call handlers. Returns the first non-nil result.
func (h *HandlerRegistry) InvokeToolCall(event ToolCallEvent) *ToolCallResult {
	for _, handler := range h.toolCall {
		if result := handler(event); result != nil {
			return result
		}
	}
	return nil
}

// InvokeToolResult runs all tool_result handlers chained: each can modify content.
func (h *HandlerRegistry) InvokeToolResult(event ToolResultEvent) *ToolResultResult {
	current := &ToolResultResult{Content: event.Content, IsError: event.IsError}
	for _, handler := range h.toolResult {
		if result := handler(ToolResultEvent{
			ToolName: event.ToolName,
			Content:  current.Content,
			IsError:  current.IsError,
		}); result != nil {
			current = result
		}
	}
	return current
}

// InvokeInput runs all input handlers. First "handled" or last "transform" wins.
func (h *HandlerRegistry) InvokeInput(event InputEvent) *InputResult {
	var lastTransform *InputResult
	for _, handler := range h.input {
		result := handler(event)
		if result == nil {
			continue
		}
		if result.Action == InputHandled {
			return result
		}
		if result.Action == InputTransform {
			lastTransform = result
			event.Text = result.Text // chain transforms
		}
	}
	return lastTransform
}

// InvokeContext runs all context handlers (chained).
func (h *HandlerRegistry) InvokeContext(event ContextEvent) *ContextResult {
	var lastResult *ContextResult
	for _, handler := range h.context {
		if result := handler(event); result != nil {
			lastResult = result
		}
	}
	return lastResult
}

// InvokeBeforeProviderRequest runs all handler chained.
func (h *HandlerRegistry) InvokeBeforeProviderRequest(event BeforeProviderRequestEvent) *BeforeProviderRequestResult {
	current := &BeforeProviderRequestResult{Payload: event.Payload}
	for _, handler := range h.beforeProviderReq {
		if result := handler(BeforeProviderRequestEvent{Payload: current.Payload}); result != nil {
			current = result
		}
	}
	return current
}

// InvokeBeforeAgentStart runs all handler chained.
func (h *HandlerRegistry) InvokeBeforeAgentStart(event BeforeAgentStartEvent) *BeforeAgentStartResult {
	current := &BeforeAgentStartResult{SystemPrompt: event.SystemPrompt, Message: event.UserMessage}
	for _, handler := range h.beforeAgentStart {
		if result := handler(BeforeAgentStartEvent{
			SystemPrompt: current.SystemPrompt,
			UserMessage:  current.Message,
		}); result != nil {
			current = result
		}
	}
	return current
}

// InvokeMessageEnd runs all message_end handlers chained.
func (h *HandlerRegistry) InvokeMessageEnd(event MessageEndEvent) *MessageEndResult {
	current := &MessageEndResult{Role: event.Role}
	for _, handler := range h.messageEnd {
		if result := handler(MessageEndEvent{Role: current.Role}); result != nil {
			current = result
		}
	}
	return current
}

// InvokeUserBash runs user_bash handlers. First result wins.
func (h *HandlerRegistry) InvokeUserBash(event UserBashEvent) *UserBashResult {
	for _, handler := range h.userBash {
		if result := handler(event); result != nil {
			return result
		}
	}
	return nil
}

// InvokeModelSelect runs all model_select handlers (fire-and-forget).
func (h *HandlerRegistry) InvokeModelSelect(event ModelSelectEvent) {
	for _, handler := range h.modelSelect {
		handler(event)
	}
}

// InvokeThinkingLevelSelect runs all thinking_level_select handlers.
func (h *HandlerRegistry) InvokeThinkingLevelSelect(event ThinkingLevelSelectEvent) {
	for _, handler := range h.thinkingLevelSelect {
		handler(event)
	}
}

// InvokeSessionBeforeSwitch runs session_before_switch handlers. First cancel wins.
func (h *HandlerRegistry) InvokeSessionBeforeSwitch(event SessionBeforeSwitchEvent) *SessionBeforeSwitchResult {
	for _, handler := range h.sessionBeforeSwitch {
		if result := handler(event); result != nil {
			return result
		}
	}
	return nil
}

// InvokeSessionBeforeFork runs session_before_fork handlers. First cancel wins.
func (h *HandlerRegistry) InvokeSessionBeforeFork(event SessionBeforeForkEvent) *SessionBeforeForkResult {
	for _, handler := range h.sessionBeforeFork {
		if result := handler(event); result != nil {
			return result
		}
	}
	return nil
}

// InvokeSessionBeforeCompact runs session_before_compact handlers.
func (h *HandlerRegistry) InvokeSessionBeforeCompact(event SessionBeforeCompactEvent) *SessionBeforeCompactResult {
	for _, handler := range h.sessionBeforeCompact {
		if result := handler(event); result != nil {
			return result
		}
	}
	return nil
}

// InvokeSessionShutdown runs session_shutdown handlers (fire-and-forget).
func (h *HandlerRegistry) InvokeSessionShutdown(event SessionShutdownEvent) {
	for _, handler := range h.sessionShutdown {
		handler(event)
	}
}
