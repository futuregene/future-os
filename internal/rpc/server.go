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
