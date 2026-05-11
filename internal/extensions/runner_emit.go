package extensions

// ---------------------------------------------------------------------------
// Event emission — mirrors pi-mono ExtensionRunner emit methods
// ---------------------------------------------------------------------------

// EmitResourcesDiscover emits the resources_discover event.
func (r *ExtensionRunner) EmitResourcesDiscover(cwd string, reason string) {
	r.Context.EventBus.Publish(Event{Name: "resources_discover", Data: map[string]interface{}{
		"cwd":    cwd,
		"reason": reason,
	}})
}

// EmitBeforeAgentStart emits the before_agent_start event and collects handler results.
func (r *ExtensionRunner) EmitBeforeAgentStart(systemPrompt string, userMessage string) (string, string, []string) {
	r.Context.EventBus.Publish(Event{Name: "before_agent_start", Data: map[string]interface{}{
		"systemPrompt": systemPrompt,
		"userMessage":  userMessage,
	}})
	if r.handlers != nil {
		result := r.handlers.InvokeBeforeAgentStart(BeforeAgentStartEvent{
			SystemPrompt: systemPrompt,
			UserMessage:  userMessage,
		})
		if result != nil {
			return result.SystemPrompt, result.Message, nil
		}
	}
	return systemPrompt, userMessage, nil
}

// EmitAgentStart emits the agent_start event.
func (r *ExtensionRunner) EmitAgentStart() {
	r.Context.EventBus.Publish(Event{Name: "agent_start", Data: nil})
}

// EmitAgentEnd emits the agent_end event.
func (r *ExtensionRunner) EmitAgentEnd() {
	r.Context.EventBus.Publish(Event{Name: "agent_end", Data: nil})
}

// EmitTurnStart emits the turn_start event.
func (r *ExtensionRunner) EmitTurnStart(turnIndex int) {
	r.Context.EventBus.Publish(Event{Name: "turn_start", Data: map[string]interface{}{
		"turnIndex": turnIndex,
	}})
}

// EmitTurnEnd emits the turn_end event.
func (r *ExtensionRunner) EmitTurnEnd(turnIndex int) {
	r.Context.EventBus.Publish(Event{Name: "turn_end", Data: map[string]interface{}{
		"turnIndex": turnIndex,
	}})
}

// EmitMessageStart emits the message_start event.
func (r *ExtensionRunner) EmitMessageStart(role string) {
	r.Context.EventBus.Publish(Event{Name: "message_start", Data: map[string]interface{}{
		"role": role,
	}})
}

// EmitMessageEnd emits the message_end event.
func (r *ExtensionRunner) EmitMessageEnd(role string) {
	r.Context.EventBus.Publish(Event{Name: "message_end", Data: map[string]interface{}{
		"role": role,
	}})
}

// EmitToolCall emits the tool_call event and checks for blocks.
func (r *ExtensionRunner) EmitToolCall(toolName string, args interface{}) *ToolCallResult {
	r.Context.EventBus.Publish(Event{Name: "tool_call", Data: map[string]interface{}{
		"tool": toolName,
		"args": args,
	}})
	if r.handlers != nil {
		return r.handlers.InvokeToolCall(ToolCallEvent{ToolName: toolName, Args: args})
	}
	return nil
}

// EmitToolResult emits the tool_result event and collects handler modifications.
func (r *ExtensionRunner) EmitToolResult(toolName string, result string, isError bool) (string, bool) {
	r.Context.EventBus.Publish(Event{Name: "tool_result", Data: map[string]interface{}{
		"tool":    toolName,
		"result":  result,
		"isError": isError,
	}})
	if r.handlers != nil {
		hr := r.handlers.InvokeToolResult(ToolResultEvent{ToolName: toolName, Content: result, IsError: isError})
		if hr != nil {
			return hr.Content, hr.IsError
		}
	}
	return result, isError
}

// EmitInput emits the input event and collects handler transforms.
func (r *ExtensionRunner) EmitInput(text string) (string, InputResultAction) {
	r.Context.EventBus.Publish(Event{Name: "input", Data: map[string]interface{}{
		"text": text,
	}})
	if r.handlers != nil {
		hr := r.handlers.InvokeInput(InputEvent{Text: text, Source: "interactive"})
		if hr != nil {
			return hr.Text, hr.Action
		}
	}
	return text, InputContinue
}

// EmitUserBash emits the user_bash event and collects handler results.
func (r *ExtensionRunner) EmitUserBash(command string, cwd string) *UserBashResult {
	r.Context.EventBus.Publish(Event{Name: "user_bash", Data: map[string]interface{}{
		"command": command,
		"cwd":     cwd,
	}})
	if r.handlers != nil {
		return r.handlers.InvokeUserBash(UserBashEvent{Command: command, CWD: cwd})
	}
	return nil
}

// EmitModelSelect emits the model_select event via both EventBus and handlers.
func (r *ExtensionRunner) EmitModelSelect(model string, previousModel string, source string) {
	r.Context.EventBus.Publish(Event{Name: "model_select", Data: map[string]interface{}{
		"model":         model,
		"previousModel": previousModel,
		"source":        source,
	}})
	if r.handlers != nil {
		r.handlers.InvokeModelSelect(ModelSelectEvent{Model: model, PreviousModel: previousModel, Source: source})
	}
}

// EmitThinkingLevelSelect emits the thinking_level_select event.
func (r *ExtensionRunner) EmitThinkingLevelSelect(level string, previousLevel string) {
	r.Context.EventBus.Publish(Event{Name: "thinking_level_select", Data: map[string]interface{}{
		"level":         level,
		"previousLevel": previousLevel,
	}})
	if r.handlers != nil {
		r.handlers.InvokeThinkingLevelSelect(ThinkingLevelSelectEvent{Level: level, PreviousLevel: previousLevel})
	}
}

// EmitSessionBeforeSwitch emits and checks for cancellation.
func (r *ExtensionRunner) EmitSessionBeforeSwitch(targetSessionFile string) *SessionBeforeSwitchResult {
	r.Context.EventBus.Publish(Event{Name: "session_before_switch", Data: map[string]interface{}{
		"targetSessionFile": targetSessionFile,
	}})
	if r.handlers != nil {
		return r.handlers.InvokeSessionBeforeSwitch(SessionBeforeSwitchEvent{TargetSessionFile: targetSessionFile})
	}
	return nil
}

// EmitSessionBeforeFork emits and checks for cancellation.
func (r *ExtensionRunner) EmitSessionBeforeFork(entryID string) *SessionBeforeForkResult {
	r.Context.EventBus.Publish(Event{Name: "session_before_fork", Data: map[string]interface{}{
		"entryId": entryID,
	}})
	if r.handlers != nil {
		return r.handlers.InvokeSessionBeforeFork(SessionBeforeForkEvent{EntryID: entryID})
	}
	return nil
}

// EmitSessionBeforeCompact emits and checks for cancellation.
func (r *ExtensionRunner) EmitSessionBeforeCompact(customInstructions string) *SessionBeforeCompactResult {
	r.Context.EventBus.Publish(Event{Name: "session_before_compact", Data: map[string]interface{}{
		"customInstructions": customInstructions,
	}})
	if r.handlers != nil {
		return r.handlers.InvokeSessionBeforeCompact(SessionBeforeCompactEvent{CustomInstructions: customInstructions})
	}
	return nil
}

// EmitSessionShutdown emits the session_shutdown event.
func (r *ExtensionRunner) EmitSessionShutdown(reason string) {
	r.Context.EventBus.Publish(Event{Name: "session_shutdown", Data: map[string]interface{}{
		"reason": reason,
	}})
	if r.handlers != nil {
		r.handlers.InvokeSessionShutdown(SessionShutdownEvent{Reason: reason})
	}
}

// EmitSessionStart emits the session_start event.
func (r *ExtensionRunner) EmitSessionStart() {
	r.Context.EventBus.Publish(Event{Name: "session_start", Data: nil})
}

// EmitContext emits the context event before each LLM call.
func (r *ExtensionRunner) EmitContext(messageCount int) {
	r.Context.EventBus.Publish(Event{Name: "context", Data: map[string]interface{}{
		"messageCount": messageCount,
	}})
	if r.handlers != nil {
		r.handlers.InvokeContext(ContextEvent{MessageCount: messageCount})
	}
}

// EmitBeforeProviderRequest emits the before_provider_request event.
func (r *ExtensionRunner) EmitBeforeProviderRequest(payload interface{}) interface{} {
	r.Context.EventBus.Publish(Event{Name: "before_provider_request", Data: map[string]interface{}{
		"payload": payload,
	}})
	if r.handlers != nil {
		result := r.handlers.InvokeBeforeProviderRequest(BeforeProviderRequestEvent{Payload: payload})
		if result != nil {
			return result.Payload
		}
	}
	return payload
}

// EmitAfterProviderResponse emits the after_provider_response event.
func (r *ExtensionRunner) EmitAfterProviderResponse(status int) {
	r.Context.EventBus.Publish(Event{Name: "after_provider_response", Data: map[string]interface{}{
		"status": status,
	}})
}

// EmitToolExecutionStart emits the tool_execution_start event.
func (r *ExtensionRunner) EmitToolExecutionStart(toolCallID string, toolName string, args interface{}) {
	r.Context.EventBus.Publish(Event{Name: "tool_execution_start", Data: map[string]interface{}{
		"toolCallId": toolCallID,
		"toolName":   toolName,
		"args":       args,
	}})
}

// EmitToolExecutionEnd emits the tool_execution_end event.
func (r *ExtensionRunner) EmitToolExecutionEnd(toolCallID string, toolName string, result string, isError bool) {
	r.Context.EventBus.Publish(Event{Name: "tool_execution_end", Data: map[string]interface{}{
		"toolCallId": toolCallID,
		"toolName":   toolName,
		"result":     result,
		"isError":    isError,
	}})
}

// EmitSessionCompact emits the session_compact event.
func (r *ExtensionRunner) EmitSessionCompact(summary string) {
	r.Context.EventBus.Publish(Event{Name: "session_compact", Data: map[string]interface{}{
		"summary": summary,
	}})
}

// EmitMessageUpdate emits the message_update event (token-by-token streaming).
func (r *ExtensionRunner) EmitMessageUpdate(role string) {
	r.Context.EventBus.Publish(Event{Name: "message_update", Data: map[string]interface{}{
		"role": role,
	}})
}

// EmitExtensionError publishes an extension error event.
func (r *ExtensionRunner) EmitExtensionError(extName string, err error) {
	r.Context.EventBus.Publish(Event{Name: "extension_error", Data: ExtensionDiagnostic{
		Type:    "error",
		Message: err.Error(),
		Path:    extName,
	}})
}
