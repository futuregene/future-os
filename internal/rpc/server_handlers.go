package rpc

import (
	"fmt"

	agentsession "github.com/huichen/xihu/internal/agentsession"
)

// =============================================================================
// Command Handler
// =============================================================================

func (s *Server) handleCommand(cmd RpcCommand) *RpcResponse {
	id := cmd.ID
	respond := func(command string, success bool, data interface{}) *RpcResponse {
		r := &RpcResponse{ID: id, Type: "response", Command: command, Success: success}
		if data != nil {
			r.Data = data
		}
		return r
	}
	success := func(command string, data interface{}) *RpcResponse { return respond(command, true, data) }
	fail := func(command string, err string) *RpcResponse { return respond(command, false, map[string]string{"error": err}) }

	as := s.session

	switch cmd.Type {
	// =================================================================
	// Prompting
	// =================================================================
	case "prompt":
		err := as.Prompt(cmd.Message, &agentsession.PromptOptions{
			Images:            cmd.Images,
			StreamingBehavior: cmd.StreamingBehavior,
		})
		if err != nil {
			return fail("prompt", err.Error())
		}
		return success("prompt", nil)

	case "steer":
		if err := as.Steer(cmd.Message); err != nil {
			return fail("steer", err.Error())
		}
		return success("steer", nil)

	case "follow_up":
		if err := as.FollowUp(cmd.Message); err != nil {
			return fail("follow_up", err.Error())
		}
		return success("follow_up", nil)

	case "abort":
		as.Abort()
		return success("abort", nil)

	case "new_session":
		if err := as.NewSession(); err != nil {
			return fail("new_session", err.Error())
		}
		return success("new_session", map[string]bool{"cancelled": false})

	// =================================================================
	// State
	// =================================================================
	case "get_state":
		return success("get_state", s.getState())

	case "get_messages":
		return success("get_messages", map[string]interface{}{
			"messages": as.GetMessages(),
		})

	// =================================================================
	// Model
	// =================================================================
	case "set_model":
		if err := as.SetModel(cmd.ModelID); err != nil {
			return fail("set_model", err.Error())
		}
		return success("set_model", map[string]string{"model": cmd.ModelID})

	case "cycle_model":
		next := as.CycleModel("forward")
		if next == "" {
			return success("cycle_model", nil)
		}
		return success("cycle_model", map[string]interface{}{
			"model":         next,
			"thinkingLevel": thinkingBudgetToLevel(as.Loop().Config.ThinkingBudget),
			"isScoped":      false,
		})

	case "get_available_models":
		// Return scoped models or use engine settings
		models := []string{as.Loop().Model}
		return success("get_available_models", map[string]interface{}{"models": models})

	// =================================================================
	// Thinking
	// =================================================================
	case "set_thinking_level":
		as.SetThinkingLevel(cmd.Level)
		return success("set_thinking_level", nil)

	case "cycle_thinking_level":
		level := as.CycleThinkingLevel()
		if level == "" {
			return success("cycle_thinking_level", nil)
		}
		return success("cycle_thinking_level", map[string]string{"level": level})

	// =================================================================
	// Queue Modes
	// =================================================================
	case "set_steering_mode":
		as.SetSteeringMode(cmd.Mode)
		return success("set_steering_mode", nil)

	case "set_follow_up_mode":
		as.SetFollowUpMode(cmd.Mode)
		return success("set_follow_up_mode", nil)

	// =================================================================
	// Compaction
	// =================================================================
	case "compact":
		result, err := as.Compact(cmd.CustomInstructions)
		if err != nil {
			return fail("compact", err.Error())
		}
		return success("compact", result)

	case "set_auto_compaction":
		as.SetAutoCompaction(cmd.Enabled)
		return success("set_auto_compaction", nil)

	// =================================================================
	// Retry
	// =================================================================
	case "set_auto_retry":
		as.SetAutoRetry(cmd.Enabled)
		return success("set_auto_retry", nil)

	case "abort_retry":
		as.AbortRetry()
		return success("abort_retry", nil)

	// =================================================================
	// Bash
	// =================================================================
	case "bash":
		result, err := as.ExecuteBash(cmd.Command)
		if err != nil {
			return fail("bash", err.Error())
		}
		return success("bash", result)

	case "abort_bash":
		// Bash abort is handled via context cancellation in the executor
		return success("abort_bash", nil)

	// =================================================================
	// Session
	// =================================================================
	case "get_session_stats":
		stats := as.GetSessionStats()
		return success("get_session_stats", stats)

	case "export_html":
		// TODO: implement proper HTML export
		return success("export_html", map[string]string{"path": ""})

	case "switch_session":
		sessionPath := cmd.SessionPath
		sess, err := as.SessionManager().Load(sessionPath, as.CWD())
		if err != nil {
			return fail("switch_session", err.Error())
		}
		as.Engine().Session = sess
		as.SetModel(sess.Model)
		return success("switch_session", map[string]bool{"cancelled": false})

	case "fork":
		entryID := cmd.EntryID
		newAS, err := as.Fork(entryID)
		if err != nil {
			return fail("fork", err.Error())
		}

		// Rebind to new session
		s.session = newAS
		s.unsub()
		s.unsub = newAS.Subscribe(func(event agentsession.AgentSessionEvent) {
			s.writeJSON(event)
		})

		return success("fork", map[string]interface{}{
			"text":      "",
			"cancelled": false,
		})

	case "clone":
		// Clone the latest leaf entry
		entries := as.Session().Entries
		if len(entries) == 0 {
			return fail("clone", "no entries")
		}
		leafID := entries[len(entries)-1].ID
		newAS, err := as.Fork(leafID)
		if err != nil {
			return fail("clone", err.Error())
		}
		s.session = newAS
		s.unsub()
		s.unsub = newAS.Subscribe(func(event agentsession.AgentSessionEvent) {
			s.writeJSON(event)
		})
		return success("clone", map[string]bool{"cancelled": false})

	case "get_fork_messages":
		msgs := as.GetUserMessagesForFork()
		return success("get_fork_messages", map[string]interface{}{"messages": msgs})

	case "get_last_assistant_text":
		text := as.GetLastAssistantText()
		if text == "" {
			return success("get_last_assistant_text", map[string]interface{}{"text": nil})
		}
		return success("get_last_assistant_text", map[string]interface{}{"text": text})

	case "set_session_name":
		as.SetSessionName(cmd.Name)
		return success("set_session_name", nil)

	// =================================================================
	// Commands
	// =================================================================
	case "get_commands":
		cmds := as.GetCommands()
		return success("get_commands", map[string]interface{}{"commands": cmds})

	default:
		return fail(cmd.Type, fmt.Sprintf("unknown command: %s", cmd.Type))
	}
}
