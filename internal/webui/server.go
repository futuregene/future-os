package webui

import (
	"embed"
	"encoding/json"
	"fmt"
	"io"
	"log"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"github.com/huichen/xihu/internal/commands"
	"github.com/huichen/xihu/internal/engine"
	"github.com/huichen/xihu/internal/session"
	"github.com/huichen/xihu/internal/tools"
	"github.com/huichen/xihu/pkg/types"
)

//go:embed static/*
var staticFiles embed.FS

type ServerOptions struct {
	APIKey  string
	BaseURL string
	Model   string
}

type Server struct {
	mu       sync.RWMutex
	mux      *http.ServeMux
	sessMgr  *session.Manager
	opts     ServerOptions
	settingsDir string
}

func NewServer(opts ServerOptions) (*Server, error) {
	if opts.BaseURL == "" {
		opts.BaseURL = "https://api.openai.com"
	}
	if opts.Model == "" {
		if strings.Contains(opts.BaseURL, "anthropic.com") {
			opts.Model = "claude-sonnet-4-20250514"
		} else {
			opts.Model = "gpt-4o"
		}
	}

	home, _ := os.UserHomeDir()
	if home == "" {
		home = os.TempDir()
	}

	s := &Server{
		mux:         http.NewServeMux(),
		sessMgr:     session.NewManager(session.DefaultDir(".")),
		opts:        opts,
		settingsDir: filepath.Join(home, ".pi"),
	}

	s.mux.HandleFunc("/", s.handleStatic)
	s.mux.HandleFunc("/api/chat", s.handleChat)
	s.mux.HandleFunc("/api/sessions", s.handleSessions)
	s.mux.HandleFunc("/api/sessions/", s.handleSessionByID)
	s.mux.HandleFunc("/api/settings", s.handleSettings)
	s.mux.Handle("/static/", http.FileServer(http.FS(staticFiles)))

	return s, nil
}

func (s *Server) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Access-Control-Allow-Origin", "*")
	w.Header().Set("Access-Control-Allow-Methods", "GET, POST, PUT, DELETE, OPTIONS")
	w.Header().Set("Access-Control-Allow-Headers", "Content-Type")
	if r.Method == "OPTIONS" {
		w.WriteHeader(200)
		return
	}
	s.mux.ServeHTTP(w, r)
}

func (s *Server) handleStatic(w http.ResponseWriter, r *http.Request) {
	if r.URL.Path == "/" {
		data, err := staticFiles.ReadFile("static/index.html")
		if err != nil {
			http.Error(w, "Not found", 404)
			return
		}
		w.Header().Set("Content-Type", "text/html; charset=utf-8")
		w.Write(data)
	}
}

func (s *Server) handleChat(w http.ResponseWriter, r *http.Request) {
	if r.Method != "POST" {
		http.Error(w, "Method not allowed", 405)
		return
	}

	var req struct {
		Message   string `json:"message"`
		SessionID string `json:"session_id"`
	}
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		http.Error(w, err.Error(), 400)
		return
	}

	// Create or load session
	var sess *session.Session
	if req.SessionID != "" {
		var err error
		sess, err = s.sessMgr.Load(req.SessionID, ".")
		if err != nil {
			sess = &session.Session{
				ID:      req.SessionID,
				CWD:     ".",
				Model:   s.opts.Model,
				BaseURL: s.opts.BaseURL,
			}
		}
	} else {
		sess = &session.Session{
			ID:        session.GenerateID(),
			CWD:       ".",
			Model:     s.opts.Model,
			BaseURL:   s.opts.BaseURL,
			CreatedAt: time.Now(),
		}
	}

	// Check for slash command
	if strings.HasPrefix(req.Message, "/") {
		cmdCtx := &commands.Context{
			CWD:              ".",
			SessionDir:       session.DefaultDir("."),
			SettingsDir:      filepath.Join(os.Getenv("HOME"), ".pi"),
			CurrentSessionID: sess.ID,
			Model:            s.opts.Model,
			BaseURL:          s.opts.BaseURL,
		}
		result, err := commands.Handle(req.Message, cmdCtx)
		if err != nil {
			s.writeJSON(w, map[string]string{"type": "error", "content": err.Error()})
			return
		}
		s.writeJSON(w, map[string]string{
			"type":      "command",
			"result":    result,
			"sessionId": sess.ID,
		})
		return
	}

	// Create provider via engine for consistency
	eng, err := engine.NewEngine(engine.EngineOptions{
		BaseURL:        s.opts.BaseURL,
		APIKey:         s.opts.APIKey,
		Model:          s.opts.Model,
		CWD:            ".",
		SessionManager: s.sessMgr,
	})
	if err != nil {
		s.writeJSON(w, map[string]string{"type": "error", "content": fmt.Sprintf("engine error: %v", err)})
		return
	}
	eng.Session = sess

	loop := eng.Loop
	loop.SystemPrompt = "You are pi, a coding agent. Be concise and direct."
	if len(loop.Tools) == 0 {
		loop.Tools = tools.AllTools()
	}

	// Build messages
	var messages []types.Message
	if len(sess.Entries) > 0 {
		messages = session.BuildContext(sess.Entries)
	}
	userMsg := types.Message{
		Role:    "user",
		Content: json.RawMessage(fmt.Sprintf(`[{"type":"text","text":%q}]`, req.Message)),
	}
	messages = append(messages, userMsg)

	// SSE streaming
	w.Header().Set("Content-Type", "text/event-stream")
	w.Header().Set("Cache-Control", "no-cache")
	w.Header().Set("Connection", "keep-alive")

	flusher, ok := w.(http.Flusher)
	if !ok {
		http.Error(w, "Streaming not supported", 500)
		return
	}

	fmt.Fprintf(w, "data: {\"type\":\"session\",\"id\":%q}\n\n", sess.ID)
	flusher.Flush()

	finalText, finalMessages, err := loop.RunStreamingWithMessages(r.Context(), messages, func(text string) {
		data, _ := json.Marshal(map[string]string{"type": "text", "content": text})
		fmt.Fprintf(w, "data: %s\n\n", data)
		flusher.Flush()
	})

	if err != nil {
		data, _ := json.Marshal(map[string]string{"type": "error", "content": err.Error()})
		fmt.Fprintf(w, "data: %s\n\n", data)
		flusher.Flush()
		return
	}

	// Save session
	newEntries := session.MessagesToEntries(finalMessages, "")
	sess.Entries = append(sess.Entries, newEntries...)
	sess.Model = s.opts.Model
	sess.BaseURL = s.opts.BaseURL
	s.sessMgr.Save(sess)

	fmt.Fprintf(w, "data: {\"type\":\"done\",\"text\":%q}\n\n", finalText)
	flusher.Flush()
}

func (s *Server) writeJSON(w http.ResponseWriter, v interface{}) {
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(v)
}

func (s *Server) handleSessions(w http.ResponseWriter, r *http.Request) {
	switch r.Method {
	case "GET":
		sessions, err := s.sessMgr.List(".")
		if err != nil {
			http.Error(w, err.Error(), 500)
			return
		}
		json.NewEncoder(w).Encode(sessions)
	case "POST":
		sess := &session.Session{
			ID:        session.GenerateID(),
			CWD:       ".",
			Model:     s.opts.Model,
			BaseURL:   s.opts.BaseURL,
			CreatedAt: time.Now(),
		}
		if err := s.sessMgr.Save(sess); err != nil {
			http.Error(w, err.Error(), 500)
			return
		}
		json.NewEncoder(w).Encode(sess)
	default:
		http.Error(w, "Method not allowed", 405)
	}
}

func (s *Server) handleSessionByID(w http.ResponseWriter, r *http.Request) {
	id := strings.TrimPrefix(r.URL.Path, "/api/sessions/")
	if id == "" {
		http.Error(w, "Missing session ID", 400)
		return
	}

	switch r.Method {
	case "GET":
		sess, err := s.sessMgr.Load(id, ".")
		if err != nil {
			http.Error(w, err.Error(), 404)
			return
		}
		json.NewEncoder(w).Encode(sess)
	case "DELETE":
		if err := s.sessMgr.Delete(id, "."); err != nil {
			http.Error(w, err.Error(), 500)
			return
		}
		w.WriteHeader(204)
	default:
		http.Error(w, "Method not allowed", 405)
	}
}

func (s *Server) handleSettings(w http.ResponseWriter, r *http.Request) {
	switch r.Method {
	case "GET":
		s.writeJSON(w, map[string]interface{}{
			"model":    s.opts.Model,
			"base_url": s.opts.BaseURL,
			"api_key":  strings.Repeat("*", len(s.opts.APIKey)),
		})
	case "PUT":
		var req map[string]string
		if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
			http.Error(w, err.Error(), 400)
			return
		}
		if v, ok := req["model"]; ok {
			s.opts.Model = v
		}
		if v, ok := req["base_url"]; ok {
			s.opts.BaseURL = v
		}
		if v, ok := req["api_key"]; ok && v != "" && !strings.Contains(v, "*") {
			s.opts.APIKey = v
		}
		s.writeJSON(w, map[string]string{"status": "ok"})
	default:
		http.Error(w, "Method not allowed", 405)
	}
}

var _ = io.Discard
var _ = log.Default
