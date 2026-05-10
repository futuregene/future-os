package sdk

import (
	"bufio"
	"encoding/json"
	"sync"
)

// =============================================================================
// JSONL Serialization
// =============================================================================

// serializeJSONLine serializes a value as a JSON line terminated by '\n'.
// This is strict JSONL — LF only, no Unicode separator splitting.
func serializeJSONLine(v interface{}) (string, error) {
	b, err := json.Marshal(v)
	if err != nil {
		return "", err
	}
	return string(b) + "\n", nil
}

// =============================================================================
// JSONL Reader — event dispatch + response routing
// =============================================================================

// jsonlReader reads JSONL lines from an io.Reader, routes responses to
// pending requests, and dispatches events to listeners.
type jsonlReader struct {
	mu        sync.RWMutex
	listeners []EventHandler

	// Pending requests keyed by id
	pending   map[string]chan *rpcResponse
	pendingMu sync.Mutex
}

// newJSONLReader creates a new jsonlReader.
func newJSONLReader() *jsonlReader {
	return &jsonlReader{
		pending: make(map[string]chan *rpcResponse),
	}
}

// addListener registers an event listener. Returns a function to unsubscribe.
func (r *jsonlReader) addListener(listener EventHandler) func() {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.listeners = append(r.listeners, listener)
	idx := len(r.listeners) - 1
	return func() {
		r.mu.Lock()
		defer r.mu.Unlock()
		// Set to nil instead of removing to avoid index shifting
		if idx < len(r.listeners) {
			r.listeners[idx] = nil
		}
	}
}

// registerPending registers a pending request and returns a channel
// that will receive the response. Mirrors the TypeScript pendingRequests Map.
func (r *jsonlReader) registerPending(id string) chan *rpcResponse {
	ch := make(chan *rpcResponse, 1)
	r.pendingMu.Lock()
	r.pending[id] = ch
	r.pendingMu.Unlock()
	return ch
}

// deregisterPending removes a pending request.
func (r *jsonlReader) deregisterPending(id string) {
	r.pendingMu.Lock()
	delete(r.pending, id)
	r.pendingMu.Unlock()
}

// handleLine processes a single JSONL line from the agent's stdout.
// Mirrors the TypeScript handleLine method exactly:
//   - If type is "response" and id matches a pending request, resolve it
//   - Otherwise, dispatch to all event listeners
func (r *jsonlReader) handleLine(line string) {
	var raw json.RawMessage
	if err := json.Unmarshal([]byte(line), &raw); err != nil {
		return // ignore non-JSON lines (matches TS catch block)
	}

	// Check if it's a response with an id that has a pending request
	var peek struct {
		Type string `json:"type"`
		ID   string `json:"id"`
	}
	if err := json.Unmarshal(raw, &peek); err != nil {
		return
	}

	if peek.Type == "response" && peek.ID != "" {
		r.pendingMu.Lock()
		ch, ok := r.pending[peek.ID]
		r.pendingMu.Unlock()
		if ok {
			var resp rpcResponse
			if err := json.Unmarshal(raw, &resp); err == nil {
				// Non-blocking send — the receiver may have timed out
				select {
				case ch <- &resp:
				default:
				}
			}
			return
		}
	}

	// Otherwise dispatch to event listeners
	r.mu.RLock()
	listeners := make([]EventHandler, len(r.listeners))
	copy(listeners, r.listeners)
	r.mu.RUnlock()

	for _, l := range listeners {
		if l != nil {
			l(raw)
		}
	}
}

// startReading reads lines from a bufio.Scanner and processes them.
// It runs until the scanner stops (EOF or error). Usually run in a goroutine.
func (r *jsonlReader) startReading(scanner *bufio.Scanner) {
	for scanner.Scan() {
		r.handleLine(scanner.Text())
	}
}
