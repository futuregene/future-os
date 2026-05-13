package rpc

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"time"

	agentsession "github.com/huichen/xihu/internal/agentsession"
)

// SocketServer is an RPC server that listens on a Unix domain socket or TCP port.
type SocketServer struct {
	server   *http.Server
	socket   string   // Unix socket path (empty if TCP)
	listener net.Listener
	session  *agentsession.AgentSession
}

// NewSocketServer creates a socket-based RPC server.
func NewSocketServer(as *agentsession.AgentSession) *SocketServer {
	return &SocketServer{session: as}
}

// ListenAndServe starts the server on a Unix socket.
func (s *SocketServer) ListenAndServe(socketPath string) error {
	ln, err := net.Listen("unix", socketPath)
	if err != nil {
		return fmt.Errorf("listen on socket %s: %w", socketPath, err)
	}
	s.socket = socketPath
	s.listener = ln
	s.server = &http.Server{
		Handler:           s,
		ReadHeaderTimeout: 10 * time.Second,
	}
	return s.server.Serve(ln)
}

// ListenAndServeTCP starts the server on a TCP port.
func (s *SocketServer) ListenAndServeTCP(addr string) error {
	ln, err := net.Listen("tcp", addr)
	if err != nil {
		return fmt.Errorf("listen on %s: %w", addr, err)
	}
	s.listener = ln
	s.server = &http.Server{
		Handler:           s,
		ReadHeaderTimeout: 10 * time.Second,
	}
	return s.server.Serve(ln)
}

// Close shuts down the server.
func (s *SocketServer) Close() error {
	if s.server != nil {
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		return s.server.Shutdown(ctx)
	}
	return nil
}

// SocketPath returns the Unix socket path if listening on a socket.
func (s *SocketServer) SocketPath() string {
	return s.socket
}

// ServeHTTP handles HTTP JSON-RPC requests.
func (s *SocketServer) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	// Set CORS headers for TypeScript client
	w.Header().Set("Access-Control-Allow-Origin", "*")
	w.Header().Set("Access-Control-Allow-Methods", "POST, OPTIONS")
	w.Header().Set("Access-Control-Allow-Headers", "Content-Type")
	w.Header().Set("Content-Type", "application/json")

	if r.Method == "OPTIONS" {
		w.WriteHeader(http.StatusOK)
		return
	}

	if r.Method != "POST" {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}

	body, err := io.ReadAll(r.Body)
	if err != nil {
		http.Error(w, "failed to read body", http.StatusBadRequest)
		return
	}
	defer r.Body.Close()

	// Handle batch requests and single requests
	var responses []json.RawMessage
	isBatch := false

	var rawReqs []json.RawMessage
	if err := json.Unmarshal(body, &rawReqs); err == nil && len(rawReqs) > 0 {
		isBatch = true
		for _, raw := range rawReqs {
			resp := s.handleRaw(raw)
			responses = append(responses, resp)
		}
	} else {
		resp := s.handleRaw(body)
		responses = append(responses, resp)
	}

	var out []byte
	if isBatch {
		out, _ = json.Marshal(responses)
	} else {
		out = responses[0]
	}
	w.Write(out)
}

// handleRaw handles a single raw JSON-RPC request.
func (s *SocketServer) handleRaw(raw json.RawMessage) json.RawMessage {
	var cmd RpcCommand
	if err := json.Unmarshal(raw, &cmd); err != nil {
		return mustMarshal(errorResponse("", "parse error: "+err.Error()))
	}

	// Use the shared Server just for command dispatch (session is shared)
	tmp := &Server{session: s.session}
	resp := tmp.handleCommand(cmd)
	out, err := json.Marshal(resp)
	if err != nil {
		return mustMarshal(errorResponse(cmd.ID, "marshal error: "+err.Error()))
	}
	return out
}

func errorResponse(id string, message string) *RpcResponse {
	return &RpcResponse{
		ID:      id,
		Type:    "response",
		Command: "",
		Success: false,
		Error:   message,
	}
}

func mustMarshal(v interface{}) json.RawMessage {
	b, _ := json.Marshal(v)
	return b
}
