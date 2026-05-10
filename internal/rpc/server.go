// Package rpc implements the pi-mono compatible RPC protocol for headless
// agent operation via a JSON protocol over stdin/stdout.
//
// Protocol:
//   - Commands: JSON objects with `type` field, optional `id` for correlation
//   - Responses: JSON objects with `type: "response"`, `command`, `success`
//   - Events: AgentSessionEvent objects streamed as they occur
//   - Extension UI: bidirectional extension_ui_request / extension_ui_response
//
// Framing is strict JSONL with LF ('\n') as the only record delimiter.
// Clients must split on '\n' only, not on Unicode separators.
package rpc

import (
	"bufio"
	"encoding/json"
	"fmt"
	"io"
	"log"
	"os"
	"os/signal"
	"strings"
	"sync"
	"syscall"

	agentsession "github.com/huichen/xihu/internal/agentsession"
	"github.com/huichen/xihu/internal/session"
)

// =============================================================================
// JSONL Framing
// =============================================================================

// serializeLine serializes a value as a JSON line terminated by '\n'.
// This is strict JSONL — LF only, no Unicode separator splitting.
func serializeLine(v interface{}) string {
	b, _ := json.Marshal(v)
	return string(b) + "\n"
}

// =============================================================================
// RPC Server
// =============================================================================

// Server handles the RPC protocol over stdin/stdout.
type Server struct {
	session *agentsession.AgentSession

	mu       sync.Mutex
	writer   io.Writer
	shutdown bool

	// Pending extension UI requests
	pendingUI sync.Map // id → chan RpcExtensionUIResponse

	// Unsubscribe from session events
	unsub func()
}

// NewServer creates a new RPC server with the given AgentSession.
func NewServer(as *agentsession.AgentSession) *Server {
	s := &Server{
		session: as,
		writer:  os.Stdout,
	}

	// Subscribe to session events and forward them as JSONL
	s.unsub = as.Subscribe(func(event agentsession.AgentSessionEvent) {
		s.writeJSON(event)
	})

	return s
}

// writeJSON writes a JSON line to stdout (thread-safe).
func (s *Server) writeJSON(v interface{}) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.shutdown {
		return
	}
	fmt.Fprint(s.writer, serializeLine(v))
}

// Run starts the RPC loop, reading commands from stdin and handling them.
// Blocks until stdin is closed or a shutdown signal is received.
func (s *Server) Run() error {
	// Set up signal handling
	sigCh := make(chan os.Signal, 1)
	signal.Notify(sigCh, syscall.SIGTERM, syscall.SIGHUP)
	defer signal.Stop(sigCh)

	// Read commands from stdin in a goroutine
	cmdCh := make(chan RpcCommand, 8)
	errCh := make(chan error, 1)

	go func() {
		scanner := bufio.NewScanner(os.Stdin)
		scanner.Buffer(make([]byte, 0, 1<<20), 1<<20) // 1MB max per line
		for scanner.Scan() {
			line := scanner.Text()

			// Check for extension_ui_response
			var uiResp RpcExtensionUIResponse
			if err := json.Unmarshal([]byte(line), &uiResp); err == nil && uiResp.Type == "extension_ui_response" {
				s.handleUIResponse(uiResp)
				continue
			}

			// Parse as command
			var cmd RpcCommand
			if err := json.Unmarshal([]byte(line), &cmd); err != nil {
				s.writeJSON(RpcResponse{Type: "response", Command: "", Success: false, Error: "invalid json: " + err.Error()})
				continue
			}
			cmdCh <- cmd
		}
		errCh <- scanner.Err()
		close(cmdCh)
	}()

	for {
		select {
		case sig := <-sigCh:
			log.Printf("rpc: received signal %v, shutting down", sig)
			s.shutdownRPC()
			return fmt.Errorf("shutdown by signal: %v", sig)

		case cmd, ok := <-cmdCh:
			if !ok {
				// stdin closed
				s.shutdownRPC()
				return <-errCh
			}
			resp := s.handleCommand(cmd)
			if resp != nil {
				s.writeJSON(resp)
			}

		case err := <-errCh:
			return err
		}
	}
}

func (s *Server) shutdownRPC() {
	s.mu.Lock()
	s.shutdown = true
	s.mu.Unlock()
	s.unsub()
	s.session.Dispose()
}

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
		return success("get_commands", map[string]interface{}{"commands": []string{}})

	default:
		return fail(cmd.Type, fmt.Sprintf("unknown command: %s", cmd.Type))
	}
}

// =============================================================================
// State helper
// =============================================================================

func (s *Server) getState() RpcSessionState {
	as := s.session
	return RpcSessionState{
		Model:                as.Model(),
		ThinkingLevel:        thinkingBudgetToLevel(as.Loop().Config.ThinkingBudget),
		IsStreaming:          as.IsStreaming(),
		IsCompacting:         false,
		SteeringMode:         as.SteeringMode(),
		FollowUpMode:         as.FollowUpMode(),
		SessionFile:          as.SessionFile(),
		SessionID:            as.SessionID(),
		SessionName:          as.SessionName(),
		AutoCompactionEnabled: true,
		MessageCount:         len(as.GetMessages()),
		PendingMessageCount:  as.PendingMessageCount(),
	}
}

func thinkingBudgetToLevel(budget int) string {
	switch {
	case budget <= 0:
		return "off"
	case budget <= 2000:
		return "minimal"
	case budget <= 4000:
		return "low"
	case budget <= 8000:
		return "medium"
	case budget <= 16000:
		return "high"
	default:
		return "xhigh"
	}
}

// =============================================================================
// Extension UI (stub)
// =============================================================================

func (s *Server) handleUIResponse(resp RpcExtensionUIResponse) {
	if v, ok := s.pendingUI.Load(resp.ID); ok {
		ch := v.(chan RpcExtensionUIResponse)
		select {
		case ch <- resp:
		default:
		}
	}
}

var _ = strings.TrimSpace
var _ = session.GenerateID
