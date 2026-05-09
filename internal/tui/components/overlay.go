package components

import (
	"fmt"
	"os"
	"os/exec"
	"strconv"
	"strings"
	"time"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

// OverlayAnchor specifies the anchor position for overlay placement.
type OverlayAnchor string

const (
	AnchorCenter       OverlayAnchor = "center"
	AnchorTopLeft      OverlayAnchor = "top-left"
	AnchorTopRight     OverlayAnchor = "top-right"
	AnchorBottomLeft   OverlayAnchor = "bottom-left"
	AnchorBottomRight  OverlayAnchor = "bottom-right"
	AnchorTopCenter    OverlayAnchor = "top-center"
	AnchorBottomCenter OverlayAnchor = "bottom-center"
	AnchorLeftCenter   OverlayAnchor = "left-center"
	AnchorRightCenter  OverlayAnchor = "right-center"
)

// AnchorToLipgloss converts an OverlayAnchor to lipgloss.Position constants (public).
func AnchorToLipgloss(a OverlayAnchor) (h, v lipgloss.Position) {
	return anchorToLipgloss(a)
}

// anchorToLipgloss converts an OverlayAnchor to lipgloss.Position constants (internal).
func anchorToLipgloss(a OverlayAnchor) (h, v lipgloss.Position) {
	switch a {
	case AnchorTopLeft:
		return lipgloss.Left, lipgloss.Top
	case AnchorTopRight:
		return lipgloss.Right, lipgloss.Top
	case AnchorBottomLeft:
		return lipgloss.Left, lipgloss.Bottom
	case AnchorBottomRight:
		return lipgloss.Right, lipgloss.Bottom
	case AnchorTopCenter:
		return lipgloss.Center, lipgloss.Top
	case AnchorBottomCenter:
		return lipgloss.Center, lipgloss.Bottom
	case AnchorLeftCenter:
		return lipgloss.Left, lipgloss.Center
	case AnchorRightCenter:
		return lipgloss.Right, lipgloss.Center
	default: // AnchorCenter
		return lipgloss.Center, lipgloss.Center
	}
}

// overlayState holds a single overlay's full state for stack management.
type overlayState struct {
	content  string
	width    int
	height   int
	style    lipgloss.Style
	selector *ListSelector
	onSelect func(value string)
	onKey    func(key string) bool
	anchor   OverlayAnchor
	termW    int // terminal width, used for percentage sizing
	termH    int // terminal height, used for percentage sizing
	stayOnSelect bool // if true, Enter does not auto-pop (used for navigation overlays)
	scrollOffset int  // scroll position for text overlays
	nonCapturing bool // if true, overlay renders but doesn't steal keyboard focus
		margin       int  // margin from anchor edge in rows/cols (TS pi-mono: OverlayOptions.margin)
		visible      func(w, h int) bool // dynamic visibility callback (TS pi-mono: OverlayOptions.visible)

	// Input mode (single-line text input)
	inputMode  bool
	inputBuf   strings.Builder
	inputTitle string
	onSubmit   func(value string)
	onCancel   func()

	// Editor mode (multi-line text area)
	editorMode    bool
	editorLines   []string
	editorCursor  int
	editorCol     int
	editorTitle   string
	onEditorSubmit func(value string)
	onEditorCancel func()

	// General dismiss callback (called for any overlay type on esc or countdown expiry)
	onDismiss func()

	// Custom dialog mode
	customMode      bool
	customTitle     string
	customContent   string
	customButtons   []CustomButton
	customSelected  int
	onCustomChoice  func(value string)

	// Countdown timer for auto-dismiss (extension dialogs)
	countdownRemaining int // seconds remaining, 0 = no countdown
	countdownExpired   bool
}

// CustomButton represents a button in a custom overlay dialog.
type CustomButton struct {
	Label string
	Value string
}

// Overlay manages modal selectors displayed on top of the main content.
// Uses a stack for nested overlays (TS pi-mono style): pushing a new overlay
// preserves the previous one; popping restores it.
type Overlay struct {
	stack []overlayState
}

// NewOverlay creates a new overlay manager.
func NewOverlay() Overlay {
	return Overlay{}
}

// Active returns whether an overlay is currently displayed.
func (o *Overlay) Active() bool {
	return len(o.stack) > 0
}

// Depth returns the number of overlays in the stack.
func (o *Overlay) Depth() int {
	return len(o.stack)
}

// SetTermSize sets the terminal dimensions used for percentage-based sizing.
func (o *Overlay) SetTermSize(w, h int) {
	for i := range o.stack {
		o.stack[i].termW = w
		o.stack[i].termH = h
	}
}

// NonCapturing returns true if the top overlay is non-capturing (renders but doesn't steal focus).
func (o *Overlay) NonCapturing() bool {
	s := o.top()
	return s != nil && s.nonCapturing
}

// Anchor returns the anchor position of the top overlay, or AnchorCenter if none.
func (o *Overlay) Anchor() OverlayAnchor {
	s := o.top()
	if s == nil {
		return AnchorCenter
	}
	return s.anchor
}

// resolveSize computes an overlay dimension from a string value.
// Integers are used directly; percentage strings (e.g. "80%") are computed from the terminal size.
func resolveSize(val string, termSize int) int {
	if strings.HasSuffix(val, "%") {
		pct, err := strconv.ParseFloat(strings.TrimSuffix(val, "%"), 64)
		if err != nil || pct <= 0 || pct > 100 {
			return termSize
		}
		return int(float64(termSize) * pct / 100.0)
	}
	n, err := strconv.Atoi(val)
	if err != nil || n <= 0 {
		return termSize
	}
	return n
}

// Show displays a new overlay with the given text content, pushing onto the stack.
func (o *Overlay) Show(content string, w, h int) {
	o.ShowAnchored(content, w, h, AnchorCenter)
}

// ShowNonCapturingText displays a text overlay that doesn't steal keyboard focus.
// Useful for transient notifications like changelog or update banners.
func (o *Overlay) ShowNonCapturingText(content string, w, h int) {
	s := overlayState{
		content:      content,
		width:        w,
		height:       h,
		nonCapturing: true,
		style: lipgloss.NewStyle().
			Border(lipgloss.RoundedBorder()).
			BorderForeground(lipgloss.Color("#e5c07b")).
			Padding(0, 1).
			Background(lipgloss.Color("#282c34")),
		anchor: AnchorTopCenter,
	}
	o.stack = append(o.stack, s)
}

// ShowAnchored displays a text overlay at the specified anchor position.
func (o *Overlay) ShowAnchored(content string, w, h int, anchor OverlayAnchor) {
	s := overlayState{
		content: content,
		width:   w,
		height:  h,
		style: lipgloss.NewStyle().
			Border(lipgloss.RoundedBorder()).
			BorderForeground(lipgloss.Color("#61afef")).
			Padding(1, 2).
			Background(lipgloss.Color("#282c34")),
		anchor: anchor,
	}
	o.stack = append(o.stack, s)
}

// ShowScrollableText displays a text overlay that supports scrolling (pgup/pgdn/up/down).
// The content can be longer than the overlay height; the user scrolls to view it.
func (o *Overlay) ShowScrollableText(content string, w, h int) {
	o.ShowScrollableTextAnchored(content, w, h, AnchorCenter)
}

// ShowScrollableTextAnchored displays a scrollable text overlay at the specified anchor.
func (o *Overlay) ShowScrollableTextAnchored(content string, w, h int, anchor OverlayAnchor) {
	s := overlayState{
		content: content,
		width:   w,
		height:  h,
		style: lipgloss.NewStyle().
			Border(lipgloss.RoundedBorder()).
			BorderForeground(lipgloss.Color("#61afef")).
			Padding(1, 2).
			Background(lipgloss.Color("#282c34")),
		anchor:      anchor,
		scrollOffset: 0,
	}
	o.stack = append(o.stack, s)
}

// ShowSelector displays a ListSelector in the overlay, pushing onto the stack.
func (o *Overlay) ShowSelector(title string, items []SelectorItem, onSelect func(value string), w, h int) {
	o.ShowSelectorAnchored(title, items, onSelect, nil, w, h, AnchorCenter)
}

// ShowSelectorWithKeyHandler displays a ListSelector with an optional custom key handler.
func (o *Overlay) ShowSelectorWithKeyHandler(title string, items []SelectorItem, onSelect func(value string), onKey func(key string) bool, w, h int) {
	o.ShowSelectorAnchored(title, items, onSelect, onKey, w, h, AnchorCenter)
}

// ShowSelectorStayOnSelect displays a ListSelector that stays visible after Enter.
// The callback is invoked but the overlay is NOT popped, so the callback can
// push a sub-overlay for nested navigation (e.g. settings → thinking selector).
func (o *Overlay) ShowSelectorStayOnSelect(title string, items []SelectorItem, onSelect func(value string), onKey func(key string) bool, w, h int) {
	sel := NewListSelector(title, items)
	sel.Width = w - 6
	sel.Height = h - 4
	s := overlayState{
		selector:      &sel,
		onSelect:      onSelect,
		onKey:         onKey,
		width:         w,
		height:        h,
		stayOnSelect:  true,
		anchor:        AnchorCenter,
		style: lipgloss.NewStyle().
			Border(lipgloss.RoundedBorder()).
			BorderForeground(lipgloss.Color("#61afef")).
			Padding(1, 2).
			Background(lipgloss.Color("#282c34")),
	}
	o.stack = append(o.stack, s)
}

// ShowSelectorAnchored displays a ListSelector at the specified anchor position.
func (o *Overlay) ShowSelectorAnchored(title string, items []SelectorItem, onSelect func(value string), onKey func(key string) bool, w, h int, anchor OverlayAnchor) {
	sel := NewListSelector(title, items)
	sel.Width = w - 6
	sel.Height = h - 4
	s := overlayState{
		selector: &sel,
		onSelect: onSelect,
		onKey:    onKey,
		width:    w,
		height:   h,
		anchor:   anchor,
		style: lipgloss.NewStyle().
			Border(lipgloss.RoundedBorder()).
			BorderForeground(lipgloss.Color("#61afef")).
			Padding(1, 2).
			Background(lipgloss.Color("#282c34")),
	}
	o.stack = append(o.stack, s)
}

// ShowInput displays a single-line text input overlay.
func (o *Overlay) ShowInput(title string, onSubmit func(value string), onCancel func(), w, h int) {
	s := overlayState{
		inputMode:  true,
		inputTitle: title,
		onSubmit:   onSubmit,
		onCancel:   onCancel,
		width:      w,
		height:     h,
		anchor:     AnchorCenter,
		style: lipgloss.NewStyle().
			Border(lipgloss.RoundedBorder()).
			BorderForeground(lipgloss.Color("#61afef")).
			Padding(1, 2).
			Background(lipgloss.Color("#282c34")),
	}
	o.stack = append(o.stack, s)
}

// ShowCustom displays a custom dialog overlay with content text and action buttons.
func (o *Overlay) ShowCustom(title string, contentText string, buttons []CustomButton, onChoice func(value string), onCancel func(), w, h int) {
	s := overlayState{
		customMode:     true,
		customTitle:    title,
		customContent:  contentText,
		customButtons:  buttons,
		customSelected: 0,
		onCustomChoice: onChoice,
		onCancel:       onCancel,
		width:          w,
		height:         h,
		anchor:         AnchorCenter,
		style: lipgloss.NewStyle().
			Border(lipgloss.RoundedBorder()).
			BorderForeground(lipgloss.Color("#61afef")).
			Padding(1, 2).
			Background(lipgloss.Color("#282c34")),
	}
	o.stack = append(o.stack, s)
}

// ShowEditor displays a multi-line text editor overlay.
func (o *Overlay) ShowEditor(title string, prefill string, onSubmit func(value string), onCancel func(), w, h int) {
	lines := strings.Split(prefill, "\n")
	if len(lines) == 1 && lines[0] == "" {
		lines = []string{""}
	}
	s := overlayState{
		editorMode:     true,
		editorTitle:    title,
		editorLines:    lines,
		editorCursor:   len(lines) - 1,
		editorCol:      len(lines[len(lines)-1]),
		onEditorSubmit: onSubmit,
		onEditorCancel: onCancel,
		width:          w,
		height:         h,
		anchor:         AnchorCenter,
		style: lipgloss.NewStyle().
			Border(lipgloss.RoundedBorder()).
			BorderForeground(lipgloss.Color("#61afef")).
			Padding(1, 2).
			Background(lipgloss.Color("#282c34")),
	}
	o.stack = append(o.stack, s)
}

// top returns a pointer to the top overlay state, or nil if empty.
func (o *Overlay) top() *overlayState {
	if len(o.stack) == 0 {
		return nil
	}
	return &o.stack[len(o.stack)-1]
}

// ReplaceItems updates the selector's items and title without recreating the overlay.
func (o *Overlay) ReplaceItems(title string, items []SelectorItem) {
	s := o.top()
	if s == nil || s.selector == nil {
		return
	}
	s.selector.SetItems(items)
	s.selector.SetTitle(title)
	s.selector.Filter = ""
	s.selector.Selected = 0
}

// SetFilter sets the filter text on the inner selector.
func (o *Overlay) SetFilter(filter string) {
	s := o.top()
	if s != nil && s.selector != nil {
		s.selector.Filter = filter
	}
}

// SelectedValue returns the value of the currently selected item, or "".
func (o *Overlay) SelectedValue() string {
	s := o.top()
	if s == nil || s.selector == nil {
		return ""
	}
	sel := s.selector.SelectedItem()
	if sel == nil {
		return ""
	}
	return sel.Value
}

// SelectIdx sets the selected index in the active selector (clamped).
func (o *Overlay) SelectIdx(idx int) {
	s := o.top()
	if s != nil && s.selector != nil {
		s.selector.SelectIdx(idx)
	}
}

// SelectedIndex returns the current selected index, or -1 if no selector.
func (o *Overlay) SelectedIndex() int {
	s := o.top()
	if s == nil || s.selector == nil || len(s.selector.Items) == 0 {
		return -1
	}
	return s.selector.Selected
}

// ItemCount returns the number of items in the active selector.
func (o *Overlay) ItemCount() int {
	s := o.top()
	if s == nil || s.selector == nil {
		return 0
	}
	return len(s.selector.Items)
}

// Hide dismisses the topmost overlay and restores the previous one (TS pi-mono pop behavior).
func (o *Overlay) Hide() {
	if len(o.stack) == 0 {
		return
	}
	o.stack = o.stack[:len(o.stack)-1]
}

// SetMargin sets the margin (in rows/cols) for the top overlay.
// Positive values add spacing from the anchor edge (TS pi-mono: OverlayOptions.margin).
func (o *Overlay) SetMargin(margin int) {
	s := o.top()
	if s != nil {
		s.margin = margin
	}
}

// SetVisible sets a visibility callback for the top overlay.
// The callback receives the terminal width and height and should return
// true if the overlay should be rendered (TS pi-mono: OverlayOptions.visible).
func (o *Overlay) SetVisible(fn func(w, h int) bool) {
	s := o.top()
	if s != nil {
		s.visible = fn
	}
}

// StartCountdown starts a countdown timer on the top overlay.
// When the countdown reaches zero, the overlay is auto-dismissed.
func (o *Overlay) StartCountdown(seconds int) {
	s := o.top()
	if s != nil && seconds > 0 {
		s.countdownRemaining = seconds
	}
}

// OnDismiss sets a callback invoked when the top overlay is dismissed
// (via esc, countdown expiry, or other cancel mechanism).
func (o *Overlay) OnDismiss(fn func()) {
	s := o.top()
	if s != nil {
		s.onDismiss = fn
	}
}

// HideAll dismisses all overlays.
func (o *Overlay) HideAll() {
	o.stack = nil
}

// CountdownTickMsg is sent every second when a dialog has an active countdown timer.
type CountdownTickMsg struct{}

// CountdownTick returns a command that fires a CountdownTickMsg after one second.
func CountdownTick() tea.Cmd {
	return tea.Tick(time.Second, func(t time.Time) tea.Msg {
		return CountdownTickMsg{}
	})
}

// Update handles Bubble Tea messages for the top overlay.
func (o *Overlay) Update(msg tea.Msg) tea.Cmd {
	s := o.top()
	if s == nil {
		return nil
	}

	// Countdown tick — decrement and auto-dismiss on expiry
	if _, ok := msg.(CountdownTickMsg); ok {
		if s.countdownRemaining > 0 {
			s.countdownRemaining--
			if s.countdownRemaining <= 0 {
				s.countdownExpired = true
				o.dismissTop()
				return nil
			}
		}
		if s.countdownRemaining > 0 {
			return CountdownTick()
		}
		return nil
	}

	if keyMsg, ok := msg.(tea.KeyMsg); ok {
		ks := keyMsg.String()

		// Input mode
		if s.inputMode {
			return o.updateInput(ks, s)
		}

		// Editor mode
		if s.editorMode {
			return o.updateEditor(ks, s)
		}

		// Custom dialog mode
		if s.customMode {
			return o.updateCustom(ks, s)
		}

		// In selector mode, handle navigation keys
		if s.selector != nil {
			return o.updateSelector(ks, s)
		}

		// Text mode: scrolling + confirm/cancel
		return o.updateText(ks, s)
	}
	return nil
}

// updateCustom handles key events for custom dialog overlays.
func (o *Overlay) updateCustom(ks string, s *overlayState) tea.Cmd {
	switch ks {
	case "esc":
		o.dismissTop()
		return nil
	case "enter":
		if len(s.customButtons) > 0 && s.customSelected >= 0 && s.customSelected < len(s.customButtons) {
			val := s.customButtons[s.customSelected].Value
			cb := s.onCustomChoice
			o.Hide()
			if cb != nil {
				cb(val)
			}
		}
		return nil
	case "left", "up":
		if s.customSelected > 0 {
			s.customSelected--
		}
		return nil
	case "right", "down":
		if s.customSelected < len(s.customButtons)-1 {
			s.customSelected++
		}
		return nil
	}
	return nil
}

// renderCustom renders the custom dialog overlay content.
func (o Overlay) renderCustom(s *overlayState) string {
	titleStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#61afef")).
		Bold(true)
	contentStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#abb2bf"))
	btnStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#abb2bf")).
		Padding(0, 2)
	btnSelectedStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#1e1e2e")).
		Background(lipgloss.Color("#61afef")).
		Padding(0, 2)
	hintStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#5c6370"))

	var sb strings.Builder
	sb.WriteString(titleStyle.Render(s.customTitle))
	sb.WriteByte('\n')
	// Content text (clipped to available height minus title, buttons, and hint)
	if s.customContent != "" {
		lines := strings.Split(s.customContent, "\n")
		maxLines := s.height - 6
		if maxLines < 1 {
			maxLines = 1
		}
		for i, line := range lines {
			if i >= maxLines {
				break
			}
			sb.WriteString(contentStyle.Render(line))
			sb.WriteByte('\n')
		}
	}
	sb.WriteByte('\n')
	// Render buttons
	if len(s.customButtons) > 0 {
		for i, btn := range s.customButtons {
			if i == s.customSelected {
				sb.WriteString(btnSelectedStyle.Render(btn.Label))
			} else {
				sb.WriteString(btnStyle.Render(btn.Label))
			}
			if i < len(s.customButtons)-1 {
				sb.WriteByte(' ')
			}
		}
		sb.WriteByte('\n')
	}
	sb.WriteString(hintStyle.Render("Enter confirm  Esc cancel"))
	return sb.String()
}

// dismissTop dismisses the top overlay, calling any cancel/dismiss callbacks.
func (o *Overlay) dismissTop() {
	if len(o.stack) == 0 {
		return
	}
	s := &o.stack[len(o.stack)-1]
	if s.inputMode && s.onCancel != nil {
		s.onCancel()
	}
	if s.editorMode && s.onEditorCancel != nil {
		s.onEditorCancel()
	}
	if s.customMode && s.onCancel != nil {
		s.onCancel()
	}
	if s.onDismiss != nil {
		s.onDismiss()
	}
	o.stack = o.stack[:len(o.stack)-1]
}

// updateInput handles key events for single-line text input overlays.
func (o *Overlay) updateInput(ks string, s *overlayState) tea.Cmd {
	switch ks {
	case "enter":
		val := s.inputBuf.String()
		cb := s.onSubmit
		o.Hide()
		if cb != nil {
			cb(val)
		}
		return nil
	case "esc":
		cb := s.onCancel
		o.Hide()
		if cb != nil {
			cb()
		}
		return nil
	case "backspace":
		buf := s.inputBuf.String()
		if len(buf) > 0 {
			s.inputBuf.Reset()
			s.inputBuf.WriteString(buf[:len(buf)-1])
		}
		return nil
	default:
		if len(ks) == 1 && ks[0] >= 32 && ks[0] < 127 {
			s.inputBuf.WriteString(ks)
		}
		return nil
	}
}

// updateEditor handles key events for multi-line editor overlays.
func (o *Overlay) updateEditor(ks string, s *overlayState) tea.Cmd {
	switch ks {
	case "enter":
		val := strings.Join(s.editorLines, "\n")
		cb := s.onEditorSubmit
		o.Hide()
		if cb != nil {
			cb(val)
		}
		return nil
	case "esc":
		cb := s.onEditorCancel
		o.Hide()
		if cb != nil {
			cb()
		}
		return nil
	case "ctrl+g":
		// Open external editor ($EDITOR or nano/vi) with current content
		if newText := openExternalEditor(strings.Join(s.editorLines, "\n")); newText != "" {
			s.editorLines = strings.Split(newText, "\n")
			if len(s.editorLines) == 0 {
				s.editorLines = []string{""}
			}
			s.editorCursor = len(s.editorLines) - 1
			s.editorCol = len(s.editorLines[s.editorCursor])
		}
		return nil
	case "up":
		if s.editorCursor > 0 {
			s.editorCursor--
			if s.editorCol > len(s.editorLines[s.editorCursor]) {
				s.editorCol = len(s.editorLines[s.editorCursor])
			}
		}
		return nil
	case "down":
		if s.editorCursor < len(s.editorLines)-1 {
			s.editorCursor++
			if s.editorCol > len(s.editorLines[s.editorCursor]) {
				s.editorCol = len(s.editorLines[s.editorCursor])
			}
		}
		return nil
	case "left":
		if s.editorCol > 0 {
			s.editorCol--
		} else if s.editorCursor > 0 {
			s.editorCursor--
			s.editorCol = len(s.editorLines[s.editorCursor])
		}
		return nil
	case "right":
		if s.editorCol < len(s.editorLines[s.editorCursor]) {
			s.editorCol++
		} else if s.editorCursor < len(s.editorLines)-1 {
			s.editorCursor++
			s.editorCol = 0
		}
		return nil
	case "backspace":
		if s.editorCol > 0 {
			line := s.editorLines[s.editorCursor]
			s.editorLines[s.editorCursor] = line[:s.editorCol-1] + line[s.editorCol:]
			s.editorCol--
		} else if s.editorCursor > 0 {
			// Merge with previous line
			prevLine := s.editorLines[s.editorCursor-1]
			s.editorCol = len(prevLine)
			s.editorLines[s.editorCursor-1] = prevLine + s.editorLines[s.editorCursor]
			s.editorLines = append(s.editorLines[:s.editorCursor], s.editorLines[s.editorCursor+1:]...)
			s.editorCursor--
		}
		return nil
	case "delete", "ctrl+d":
		line := s.editorLines[s.editorCursor]
		if s.editorCol < len(line) {
			s.editorLines[s.editorCursor] = line[:s.editorCol] + line[s.editorCol+1:]
		} else if s.editorCursor < len(s.editorLines)-1 {
			// Merge with next line
			s.editorLines[s.editorCursor] = line + s.editorLines[s.editorCursor+1]
			s.editorLines = append(s.editorLines[:s.editorCursor+1], s.editorLines[s.editorCursor+2:]...)
		}
		return nil
	case "home":
		s.editorCol = 0
		return nil
	case "end":
		s.editorCol = len(s.editorLines[s.editorCursor])
		return nil
	default:
		if len(ks) == 1 && ks[0] >= 32 && ks[0] < 127 {
			line := s.editorLines[s.editorCursor]
			s.editorLines[s.editorCursor] = line[:s.editorCol] + string(ks[0]) + line[s.editorCol:]
			s.editorCol++
		} else if ks == "enter" && false { // disabled: enter submits
		} else if ks == "tab" {
			line := s.editorLines[s.editorCursor]
			s.editorLines[s.editorCursor] = line[:s.editorCol] + "\t" + line[s.editorCol:]
			s.editorCol++
		}
		return nil
	}
}

// updateSelector handles key events for list selector overlays.
func (o *Overlay) updateSelector(ks string, s *overlayState) tea.Cmd {
	// Custom key handler takes priority
	if s.onKey != nil && s.onKey(ks) {
		return nil
	}
	switch ks {
	case "esc":
		o.dismissTop()
		return nil
	case "up", "ctrl+k", "ctrl+p":
		s.selector.MoveUp()
		return nil
	case "down", "ctrl+j", "ctrl+n":
		s.selector.MoveDown()
		return nil
	case "enter":
		val := ""
		if sel := s.selector.SelectedItem(); sel != nil {
			val = sel.Value
		}
		cb := s.onSelect
		if !s.stayOnSelect {
			o.Hide()
		}
		if cb != nil {
			cb(val)
		}
		return nil
	case "backspace":
		if len(s.selector.Filter) > 0 {
			s.selector.Filter = s.selector.Filter[:len(s.selector.Filter)-1]
			s.selector.Selected = 0
		}
		return nil
	default:
		// Single printable character = add to filter
		if len(ks) == 1 && ks[0] >= 32 && ks[0] < 127 {
			s.selector.ApplyFilter(s.selector.Filter + ks)
			s.selector.Selected = 0
			return nil
		}
		return nil
	}
}

// updateText handles key events for scrollable text overlays.
func (o *Overlay) updateText(ks string, s *overlayState) tea.Cmd {
	switch ks {
	case "esc":
		o.dismissTop()
		return nil
	case "enter":
		o.Hide()
		return func() tea.Msg { return OverlayConfirmMsg{} }
	case "up", "ctrl+k", "ctrl+p":
		if s.scrollOffset > 0 {
			s.scrollOffset--
		}
		return nil
	case "down", "ctrl+j", "ctrl+n":
		s.scrollOffset++
		return nil
	case "pgup":
		visibleLines := s.height - 4
		if visibleLines < 1 {
			visibleLines = 1
		}
		s.scrollOffset -= visibleLines
		if s.scrollOffset < 0 {
			s.scrollOffset = 0
		}
		return nil
	case "pgdown":
		visibleLines := s.height - 4
		if visibleLines < 1 {
			visibleLines = 1
		}
		s.scrollOffset += visibleLines
		return nil
	case "home":
		s.scrollOffset = 0
		return nil
	case "end":
		lines := strings.Split(s.content, "\n")
		visibleLines := s.height - 4
		if visibleLines < 1 {
			visibleLines = 1
		}
		s.scrollOffset = len(lines) - visibleLines
		if s.scrollOffset < 0 {
			s.scrollOffset = 0
		}
		return nil
	}
	return nil
}

// countdownStyle is the style for the countdown indicator in timed dialogs.
var countdownStyle = lipgloss.NewStyle().
	Foreground(lipgloss.Color("#e5c07b")).
	Bold(true)

// View renders the topmost overlay content (styled, without placement).
// Placement is done by app.go which has terminal dimensions.
func (o Overlay) View() string {
	s := o.top()
	if s == nil {
		return ""
	}

	// Check dynamic visibility callback (TS pi-mono: OverlayOptions.visible)
	if s.visible != nil && !s.visible(s.termW, s.termH) {
		return ""
	}

	var content string
	if s.inputMode {
		content = o.renderInput(s)
	} else if s.editorMode {
		content = o.renderEditor(s)
	} else if s.customMode {
		content = o.renderCustom(s)
	} else if s.selector != nil {
		content = s.selector.View()
	} else {
		content = o.scrollText(s)
	}

	// Countdown timer display
	if s.countdownRemaining > 0 {
		countdownLabel := countdownStyle.Render(fmt.Sprintf("⏳ %ds", s.countdownRemaining))
		content = countdownLabel + "\n" + content
	}

	style := s.style.Copy().Width(s.width).Height(s.height)
	if s.margin > 0 {
		style = style.Margin(s.margin, s.margin)
	}
	return style.Render(content)
}

// renderInput renders the text input overlay content.
func (o Overlay) renderInput(s *overlayState) string {
	titleStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#61afef")).
		Bold(true)
	inputStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#abb2bf"))
	cursorStyle := lipgloss.NewStyle().
		Background(lipgloss.Color("#89b4fa")).
		Foreground(lipgloss.Color("#1e1e2e"))
	hintStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#5c6370"))

	text := s.inputBuf.String()
	// Render with cursor
	var line string
	if text == "" {
		line = cursorStyle.Render(" ")
	} else {
		line = inputStyle.Render(text) + cursorStyle.Render(" ")
	}

	var sb strings.Builder
	sb.WriteString(titleStyle.Render(s.inputTitle))
	sb.WriteByte('\n')
	sb.WriteString(line)
	sb.WriteByte('\n')
	sb.WriteString(hintStyle.Render("Enter submit  Esc cancel"))

	return sb.String()
}

// renderEditor renders the multi-line editor overlay content.
func (o Overlay) renderEditor(s *overlayState) string {
	titleStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#61afef")).
		Bold(true)
	lineStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#abb2bf"))
	cursorStyle := lipgloss.NewStyle().
		Background(lipgloss.Color("#89b4fa")).
		Foreground(lipgloss.Color("#1e1e2e"))
	hintStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#5c6370"))

	visibleLines := s.height - 5
	if visibleLines < 1 {
		visibleLines = 1
	}

	// Calculate scroll window
	totalLines := len(s.editorLines)
	start := s.editorCursor - visibleLines/2
	if start < 0 {
		start = 0
	}
	end := start + visibleLines
	if end > totalLines {
		end = totalLines
		start = end - visibleLines
		if start < 0 {
			start = 0
		}
	}

	var sb strings.Builder
	sb.WriteString(titleStyle.Render(s.editorTitle))
	sb.WriteByte('\n')

	for i := start; i < end; i++ {
		line := s.editorLines[i]
		if i == s.editorCursor {
			// Show cursor position within the line
			if s.editorCol >= len(line) {
				sb.WriteString(lineStyle.Render(line))
				sb.WriteString(cursorStyle.Render(" "))
			} else {
				before := line[:s.editorCol]
				cursor := string(line[s.editorCol])
				after := line[s.editorCol+1:]
				sb.WriteString(lineStyle.Render(before))
				sb.WriteString(cursorStyle.Render(cursor))
				sb.WriteString(lineStyle.Render(after))
			}
		} else {
			sb.WriteString(lineStyle.Render(line))
		}
		sb.WriteByte('\n')
	}

	// Scroll indicator
	if totalLines > visibleLines {
		scrollStyle := lipgloss.NewStyle().
			Foreground(lipgloss.Color("#5c6370"))
		sb.WriteString(scrollStyle.Render(fmt.Sprintf("(%d/%d)", s.editorCursor+1, totalLines)))
		sb.WriteByte('\n')
	}

	sb.WriteString(hintStyle.Render("Enter submit  Esc cancel  Ctrl+G external editor"))
	return sb.String()
}

// scrollText returns the visible portion of text content based on scroll offset.
// Available height (for content lines) is height - 4 (2 padding + 2 border).
func (o Overlay) scrollText(s *overlayState) string {
	lines := strings.Split(s.content, "\n")
	visibleLines := s.height - 4
	if visibleLines < 1 {
		visibleLines = 1
	}

	totalLines := len(lines)
	if totalLines <= visibleLines {
		return s.content // no scrolling needed
	}

	// Clamp scroll offset
	if s.scrollOffset > totalLines-visibleLines {
		s.scrollOffset = totalLines - visibleLines
	}
	if s.scrollOffset < 0 {
		s.scrollOffset = 0
	}

	start := s.scrollOffset
	end := start + visibleLines
	if end > totalLines {
		end = totalLines
	}

	var sb strings.Builder

	// Top scroll indicator
	if s.scrollOffset > 0 {
		indicator := fmt.Sprintf("─── ↑ %d more lines ↑ ───", s.scrollOffset)
		sb.WriteString(indicatorStyle.Render(indicator))
		sb.WriteByte('\n')
	}

	sb.WriteString(strings.Join(lines[start:end], "\n"))

	// Bottom scroll indicator
	if end < totalLines {
		remaining := totalLines - end
		indicator := fmt.Sprintf("─── ↓ %d more lines ↓ ───", remaining)
		sb.WriteByte('\n')
		sb.WriteString(indicatorStyle.Render(indicator))
	}

	return sb.String()
}

// indicatorStyle is the style for scroll indicators in text overlays.
var indicatorStyle = lipgloss.NewStyle().
	Foreground(lipgloss.Color("#5c6370")).
	Faint(true)

// OverlayConfirmMsg is sent when the user confirms the overlay selection.
type OverlayConfirmMsg struct{}

// SelectorChosenMsg is sent when the user selects an item from a ListSelector overlay.
type SelectorChosenMsg struct {
	Value string
}

// openExternalEditor opens $EDITOR (or nano/vi) on a temp file and returns the content.
// Used by the overlay editor's Ctrl+G handler.
func openExternalEditor(currentText string) string {
	editor := os.Getenv("EDITOR")
	if editor == "" {
		editor = os.Getenv("VISUAL")
	}
	if editor == "" {
		editor = "nano"
	}

	f, err := os.CreateTemp("", "xihu-overlay-edit-*.md")
	if err != nil {
		return ""
	}
	defer os.Remove(f.Name())

	if currentText != "" {
		f.WriteString(currentText)
	}
	f.Close()

	cmd := exec.Command(editor, f.Name())
	cmd.Stdin = os.Stdin
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	if err := cmd.Run(); err != nil {
		return ""
	}

	content, err := os.ReadFile(f.Name())
	if err != nil {
		return ""
	}
	return strings.TrimSpace(string(content))
}
