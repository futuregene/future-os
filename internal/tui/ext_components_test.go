package tui

import (
	"testing"

	tea "github.com/charmbracelet/bubbletea"
)

// ─── Test Helpers — simple component implementations ──────────────────────────

// testHeader is a minimal HeaderComponent for testing.
type testHeader struct {
	text  string
	width int
}

func (h *testHeader) Init() tea.Cmd                          { return nil }
func (h *testHeader) View() string                            { return h.text }
func (h *testHeader) Update(msg tea.Msg) (tea.Model, tea.Cmd) { return h, nil }
func (h *testHeader) SetWidth(w int)                          { h.width = w }

// testFooter is a minimal FooterComponent for testing.
type testFooter struct {
	text  string
	width int
}

func (f *testFooter) Init() tea.Cmd                          { return nil }
func (f *testFooter) View() string                            { return f.text }
func (f *testFooter) Update(msg tea.Msg) (tea.Model, tea.Cmd) { return f, nil }
func (f *testFooter) SetWidth(w int)                          { f.width = w }

// testEditor is a minimal EditorComponent for testing.
type testEditor struct {
	value  string
	width  int
	height int
}

func (e *testEditor) Init() tea.Cmd                          { return nil }
func (e *testEditor) View() string                            { return e.value }
func (e *testEditor) Update(msg tea.Msg) (tea.Model, tea.Cmd) { return e, nil }
func (e *testEditor) Value() string                           { return e.value }
func (e *testEditor) SetValue(v string)                       { e.value = v }
func (e *testEditor) Reset()                                  { e.value = "" }
func (e *testEditor) Focus() tea.Cmd                          { return nil }
func (e *testEditor) Blur()                                   {}
func (e *testEditor) SetWidth(w int)                          { e.width = w }
func (e *testEditor) SetHeight(h int)                         { e.height = h }
func (e *testEditor) Height() int                             { return e.height }
func (e *testEditor) Empty() bool                             { return e.value == "" }

// ─── Interface Satisfaction Tests ────────────────────────────────────────────

func TestHeaderComponentSatisfaction(t *testing.T) {
	var h HeaderComponent = &testHeader{text: "custom header", width: 80}
	if h.View() != "custom header" {
		t.Errorf("expected 'custom header', got %q", h.View())
	}
	h.SetWidth(120)
}

func TestFooterComponentSatisfaction(t *testing.T) {
	var f FooterComponent = &testFooter{text: "custom footer"}
	if f.View() != "custom footer" {
		t.Errorf("expected 'custom footer', got %q", f.View())
	}
}

func TestEditorComponentSatisfaction(t *testing.T) {
	var e EditorComponent = &testEditor{value: "hello", width: 80, height: 5}
	if e.Value() != "hello" {
		t.Errorf("expected 'hello', got %q", e.Value())
	}
	if e.Empty() {
		t.Error("expected non-empty editor")
	}
	if e.Height() != 5 {
		t.Errorf("expected height 5, got %d", e.Height())
	}
	e.SetValue("world")
	if e.Value() != "world" {
		t.Errorf("expected 'world', got %q", e.Value())
	}
	e.Reset()
	if !e.Empty() {
		t.Error("expected empty editor after Reset")
	}
}

// ─── Factory Tests ──────────────────────────────────────────────────────────

func TestFooterFactory(t *testing.T) {
	factory := FooterFactory(func() FooterComponent {
		return &testFooter{text: "factory footer"}
	})
	f := factory()
	if f.View() != "factory footer" {
		t.Errorf("expected 'factory footer', got %q", f.View())
	}
}

func TestHeaderFactory(t *testing.T) {
	factory := HeaderFactory(func() HeaderComponent {
		return &testHeader{text: "factory header"}
	})
	h := factory()
	if h.View() != "factory header" {
		t.Errorf("expected 'factory header', got %q", h.View())
	}
}

func TestEditorFactory(t *testing.T) {
	factory := EditorFactory(func() EditorComponent {
		return &testEditor{value: "factory editor", height: 3}
	})
	e := factory()
	if e.Value() != "factory editor" {
		t.Errorf("expected 'factory editor', got %q", e.Value())
	}
	if e.Height() != 3 {
		t.Errorf("expected height 3, got %d", e.Height())
	}
}

// ─── Bubble Tea Model Compliance ────────────────────────────────────────────

func TestEditorComponentBubbleTeaModel(t *testing.T) {
	e := &testEditor{value: "test", width: 80, height: 5}
	_ = e.Init()
	next, cmd := e.Update(tea.WindowSizeMsg{Width: 100, Height: 40})
	if cmd != nil {
		t.Error("expected nil cmd from WindowSizeMsg")
	}
	if ec, ok := next.(EditorComponent); !ok {
		t.Error("Update must return EditorComponent")
	} else if ec.Value() != "test" {
		t.Errorf("expected 'test', got %q", ec.Value())
	}
}
