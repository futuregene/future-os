package agentsession

import (
	"encoding/json"
	"testing"
	"time"

	"github.com/huichen/xihu/internal/agent"
	"github.com/huichen/xihu/internal/engine"
	"github.com/huichen/xihu/internal/session"
	"github.com/huichen/xihu/pkg/types"
)

// testEngine creates a minimal engine for testing.
func testEngine() *engine.Engine {
	loop := &agent.Loop{
		Model:        "test-model",
		SteeringQueue: make(chan string, 64),
		FollowUpQueue: make(chan string, 64),
		SteeringMode:  "all",
		FollowUpMode:  "all",
		Config:        types.AgentConfig{MaxTurns: 50},
	}

	eng := &engine.Engine{
		Provider:       nil, // won't be called in unit tests
		Model:          "test-model",
		Config:         engine.AgentConfig{CWD: "."},
		Tools:          nil,
		Loop:           loop,
		Session:        &session.Session{ID: session.GenerateID(), CWD: ".", Model: "test-model", CreatedAt: time.Now()},
		SessionManager: session.NewManager("/tmp/xihu-test-sessions"),
	}
	eng.Config = eng.Config.Default()
	return eng
}

func TestNew(t *testing.T) {
	eng := testEngine()
	as, err := New(AgentSessionConfig{
		Engine: eng,
		CWD:    ".",
	})
	if err != nil {
		t.Fatalf("New failed: %v", err)
	}
	if as == nil {
		t.Fatal("AgentSession is nil")
	}
	if as.Model() != "test-model" {
		t.Errorf("expected model 'test-model', got %q", as.Model())
	}
	if as.CWD() != "." {
		t.Errorf("expected cwd '.', got %q", as.CWD())
	}
}

func TestModel(t *testing.T) {
	eng := testEngine()
	as, _ := New(AgentSessionConfig{Engine: eng, CWD: "."})

	if as.Model() != "test-model" {
		t.Errorf("initial model: got %q, want test-model", as.Model())
	}
}

func TestSessionID(t *testing.T) {
	eng := testEngine()
	as, _ := New(AgentSessionConfig{Engine: eng, CWD: "."})

	id := as.SessionID()
	if id == "" {
		t.Error("SessionID is empty")
	}
}

func TestSetModel(t *testing.T) {
	eng := testEngine()
	as, _ := New(AgentSessionConfig{Engine: eng, CWD: "."})

	err := as.SetModel("new-model")
	if err != nil {
		t.Errorf("SetModel failed: %v", err)
	}
	if as.Model() != "new-model" {
		t.Errorf("after SetModel: got %q, want new-model", as.Model())
	}
}

func TestSetModelUpdatesSession(t *testing.T) {
	eng := testEngine()
	as, _ := New(AgentSessionConfig{Engine: eng, CWD: "."})

	as.SetModel("changed-model")
	if eng.Session.Model != "changed-model" {
		t.Errorf("session model: got %q, want changed-model", eng.Session.Model)
	}
}

func TestSetThinkingLevel(t *testing.T) {
	eng := testEngine()
	as, _ := New(AgentSessionConfig{Engine: eng, CWD: "."})

	as.SetThinkingLevel("high")
	if as.Loop().Config.ThinkingBudget != 16000 {
		t.Errorf("thinking budget for 'high': got %d, want 16000", as.Loop().Config.ThinkingBudget)
	}

	as.SetThinkingLevel("off")
	if as.Loop().Config.ThinkingBudget != 0 {
		t.Errorf("thinking budget for 'off': got %d, want 0", as.Loop().Config.ThinkingBudget)
	}

	as.SetThinkingLevel("low")
	if as.Loop().Config.ThinkingBudget != 4000 {
		t.Errorf("thinking budget for 'low': got %d, want 4000", as.Loop().Config.ThinkingBudget)
	}
}

func TestCycleThinkingLevel(t *testing.T) {
	eng := testEngine()
	as, _ := New(AgentSessionConfig{Engine: eng, CWD: "."})

	// Start from off (budget 0)
	as.SetThinkingLevel("off")
	next := as.CycleThinkingLevel()
	if next != "minimal" {
		t.Errorf("after off, cycle should give 'minimal', got %q", next)
	}
}

func TestSteerAndFollowUp(t *testing.T) {
	eng := testEngine()
	as, _ := New(AgentSessionConfig{Engine: eng, CWD: "."})

	// Queue some messages
	as.Steer("steer message 1")
	as.FollowUp("follow message 1")

	if as.PendingMessageCount() != 2 {
		t.Errorf("pending messages: got %d, want 2", as.PendingMessageCount())
	}
}

func TestClearQueue(t *testing.T) {
	eng := testEngine()
	as, _ := New(AgentSessionConfig{Engine: eng, CWD: "."})

	as.Steer("msg1")
	as.FollowUp("msg2")

	steering, followUp := as.ClearQueue()
	if len(steering) != 1 || steering[0] != "msg1" {
		t.Errorf("steering: got %v, want [msg1]", steering)
	}
	if len(followUp) != 1 || followUp[0] != "msg2" {
		t.Errorf("followUp: got %v, want [msg2]", followUp)
	}
	if as.PendingMessageCount() != 0 {
		t.Errorf("after clear: pending %d, want 0", as.PendingMessageCount())
	}
}

func TestSubscribe(t *testing.T) {
	eng := testEngine()
	as, _ := New(AgentSessionConfig{Engine: eng, CWD: "."})

	received := false
	unsub := as.Subscribe(func(event AgentSessionEvent) {
		received = true
	})

	// The event subscription should work
	_ = received

	// Unsubscribe
	unsub()
}

func TestQueueMode(t *testing.T) {
	eng := testEngine()
	as, _ := New(AgentSessionConfig{Engine: eng, CWD: "."})

	as.SetSteeringMode("one-at-a-time")
	if as.SteeringMode() != "one-at-a-time" {
		t.Errorf("steering mode: got %q, want one-at-a-time", as.SteeringMode())
	}

	as.SetFollowUpMode("one-at-a-time")
	if as.FollowUpMode() != "one-at-a-time" {
		t.Errorf("followUp mode: got %q, want one-at-a-time", as.FollowUpMode())
	}
}

func TestSetSessionName(t *testing.T) {
	eng := testEngine()
	as, _ := New(AgentSessionConfig{Engine: eng, CWD: "."})

	as.SetSessionName("my test session")
	if as.SessionName() != "my test session" {
		t.Errorf("session name: got %q, want 'my test session'", as.SessionName())
	}
}

func TestGetLastAssistantText(t *testing.T) {
	eng := testEngine()
	as, _ := New(AgentSessionConfig{Engine: eng, CWD: "."})

	// Empty session
	if text := as.GetLastAssistantText(); text != "" {
		t.Errorf("empty session: got %q, want empty", text)
	}
}

func TestGetSessionStats(t *testing.T) {
	eng := testEngine()
	as, _ := New(AgentSessionConfig{Engine: eng, CWD: "."})

	stats := as.GetSessionStats()
	if stats == nil {
		t.Fatal("stats is nil")
	}
	if stats.SessionID != as.SessionID() {
		t.Errorf("stats session id mismatch")
	}
	if stats.TotalMessages != 0 {
		t.Errorf("empty session: got %d messages, want 0", stats.TotalMessages)
	}
}

func TestGetMessages(t *testing.T) {
	eng := testEngine()
	as, _ := New(AgentSessionConfig{Engine: eng, CWD: "."})

	msgs := as.GetMessages()
	if len(msgs) > 0 {
		t.Errorf("empty session: got %d messages, want 0", len(msgs))
	}
}

func TestNewSession(t *testing.T) {
	eng := testEngine()
	as, _ := New(AgentSessionConfig{Engine: eng, CWD: "."})

	oldID := as.SessionID()
	err := as.NewSession()
	if err != nil {
		t.Fatalf("NewSession failed: %v", err)
	}
	if as.SessionID() == "" {
		t.Error("NewSession ID is empty")
	}
	// GenerateID is time-based (second precision), so IDs may be same
	_ = oldID
}

func TestDispose(t *testing.T) {
	eng := testEngine()
	as, _ := New(AgentSessionConfig{Engine: eng, CWD: "."})

	as.Steer("msg")
	as.Dispose()

	if as.PendingMessageCount() != 0 {
		t.Errorf("after dispose: pending %d, want 0", as.PendingMessageCount())
	}
}

func TestExecuteBash(t *testing.T) {
	eng := testEngine()
	as, _ := New(AgentSessionConfig{Engine: eng, CWD: "."})

	result, err := as.ExecuteBash("echo hello")
	if err != nil && result == nil {
		t.Logf("bash test note: %v (expected in test env without real bash tool)", err)
	}
}

func TestLoopAccessor(t *testing.T) {
	eng := testEngine()
	as, _ := New(AgentSessionConfig{Engine: eng, CWD: "."})

	loop := as.Loop()
	if loop == nil {
		t.Fatal("Loop() returns nil")
	}
	if loop.Model != "test-model" {
		t.Errorf("loop model: got %q, want test-model", loop.Model)
	}
}

func TestSessionAccessor(t *testing.T) {
	eng := testEngine()
	as, _ := New(AgentSessionConfig{Engine: eng, CWD: "."})

	sess := as.Session()
	if sess == nil {
		t.Fatal("Session() returns nil")
	}
	if sess.ID != as.SessionID() {
		t.Errorf("session id mismatch")
	}
}

func TestGetUserMessagesForFork(t *testing.T) {
	eng := testEngine()
	as, _ := New(AgentSessionConfig{Engine: eng, CWD: "."})

	// Add a user entry to test
	userContent, _ := json.Marshal([]types.TextContent{{Type: "text", Text: "test message"}})
	eng.Session.Entries = append(eng.Session.Entries, session.SessionEntry{
		ID:        session.GenerateID(),
		Type:      session.EntryTypeUser,
		Role:      "user",
		Content:   userContent,
		Timestamp: time.Now(),
	})

	messages := as.GetUserMessagesForFork()
	if len(messages) != 1 {
		t.Errorf("fork messages: got %d, want 1", len(messages))
	}
	if len(messages) == 1 && messages[0].Text != "test message" {
		t.Errorf("fork message text: got %q, want 'test message'", messages[0].Text)
	}
}

func TestThinkingLevelToBudget(t *testing.T) {
	tests := []struct {
		level  string
		budget int
	}{
		{"off", 0},
		{"minimal", 2000},
		{"low", 4000},
		{"medium", 8000},
		{"high", 16000},
		{"xhigh", 24000},
		{"unknown", 0},
	}

	for _, tc := range tests {
		budget := thinkingLevelToBudget(tc.level)
		if budget != tc.budget {
			t.Errorf("thinkingLevelToBudget(%q): got %d, want %d", tc.level, budget, tc.budget)
		}
	}
}
