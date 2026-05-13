package rpc

import (
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"sync"
	"time"

	agentsession "github.com/huichen/xihu/internal/agentsession"
)

// SocketServer is an RPC server that listens on a Unix domain socket or TCP port.
type SocketServer struct {
	session *agentsession.AgentSession
	mu      sync.Mutex
	socket  string
	sseConns map[chan<- []byte]struct{}
	unsub   func()
}

// NewSocketServer creates a socket-based RPC server.
func NewSocketServer(as *agentsession.AgentSession) *SocketServer {
	s := &SocketServer{session: as}
	s.mu.Lock()
	s.sseConns = make(map[chan<- []byte]struct{})
	s.mu.Unlock()

	// Subscribe to session events and forward them as SSE
	s.unsub = as.Subscribe(func(event agentsession.AgentSessionEvent) {
		data, err := json.Marshal(event)
		if err != nil {
			return
		}
		line := fmt.Sprintf("event: %s\ndata: %s\n\n", event.Type, data)
		s.mu.Lock()
		defer s.mu.Unlock()
		for conn := range s.sseConns {
			select {
			case conn <- []byte(line):
			default:
			}
		}
	})

	return s
}

// ListenAndServe starts the server on a Unix socket.
func (s *SocketServer) ListenAndServe(socketPath string) error {
	// Remove any stale socket file from a previous crashed server
	os.Remove(socketPath)
	ln, err := net.Listen("unix", socketPath)
	if err != nil {
		return fmt.Errorf("listen on socket %s: %w", socketPath, err)
	}
	s.socket = socketPath
	return s.serve(ln)
}

// ListenAndServeTCP starts the server on a TCP port.
func (s *SocketServer) ListenAndServeTCP(addr string) error {
	ln, err := net.Listen("tcp", addr)
	if err != nil {
		return fmt.Errorf("listen on %s: %w", addr, err)
	}
	return s.serve(ln)
}

func (s *SocketServer) serve(ln net.Listener) error {
	mux := http.NewServeMux()
	mux.HandleFunc("/", s.handleHTTP)
	mux.HandleFunc("/events", s.handleSSE)
	mux.HandleFunc("/events/", s.handleSSE)

	srv := &http.Server{
		Handler:           mux,
		ReadHeaderTimeout: 10 * time.Second,
	}

	errCh := make(chan error, 1)
	go func() {
		errCh <- srv.Serve(ln)
	}()

	err := <-errCh
	return err
}

// Close shuts down the server.
func (s *SocketServer) Close() error {
	if s.unsub != nil {
		s.unsub()
	}
	return nil
}

// SocketPath returns the Unix socket path if listening on a socket.
func (s *SocketServer) SocketPath() string {
	return s.socket
}

// ─── HTTP Handler ─────────────────────────────────────────────────────────

func (s *SocketServer) handleHTTP(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Access-Control-Allow-Origin", "*")
	w.Header().Set("Access-Control-Allow-Methods", "POST, GET, OPTIONS")
	w.Header().Set("Access-Control-Allow-Headers", "Content-Type")

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

	w.Header().Set("Content-Type", "application/json")

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

// ─── SSE Handler ─────────────────────────────────────────────────────────

func (s *SocketServer) handleSSE(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Content-Type", "text/event-stream")
	w.Header().Set("Cache-Control", "no-cache")
	w.Header().Set("Connection", "keep-alive")
	w.Header().Set("Access-Control-Allow-Origin", "*")

	flusher, ok := w.(http.Flusher)
	if !ok {
		http.Error(w, "streaming not supported", http.StatusInternalServerError)
		return
	}

	ch := make(chan []byte, 64)
	s.mu.Lock()
	s.sseConns[ch] = struct{}{}
	s.mu.Unlock()

	// Send initial ping
	fmt.Fprintf(w, ": ping\n\n")
	flusher.Flush()

	// Stream events until client disconnects
	for {
		select {
		case data, ok := <-ch:
			if !ok {
				return
			}
			w.Write(data)
			flusher.Flush()
		case <-r.Context().Done():
			s.mu.Lock()
			delete(s.sseConns, ch)
			s.mu.Unlock()
			return
		}
	}
}

// ─── Command Handler ─────────────────────────────────────────────────────

func (s *SocketServer) handleRaw(raw json.RawMessage) json.RawMessage {
	var cmd RpcCommand
	if err := json.Unmarshal(raw, &cmd); err != nil {
		return mustMarshal(errorResponse("", "parse error: "+err.Error()))
	}

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
