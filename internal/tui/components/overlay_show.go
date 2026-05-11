package components

import (
	"strings"

	"github.com/charmbracelet/lipgloss"
)

// isPunct returns true for punctuation characters (word boundary, TS pi-mono style).

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
			Padding(1, 2).
			Background(lipgloss.Color("#282c34")),
		anchor:      anchor,
		scrollOffset: 0,
		keyMatcher:   o.defKeyMatcher,
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
	if o.defSelHelp != "" {
		sel.HelpText = o.defSelHelp
	}
	s := overlayState{
		selector:      &sel,
		onSelect:      onSelect,
		onKey:         onKey,
		width:         w,
		height:        h,
		stayOnSelect:  true,
		anchor:        AnchorCenter,
		style: lipgloss.NewStyle().
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
	if o.defSelHelp != "" {
		sel.HelpText = o.defSelHelp
	}
	s := overlayState{
		selector: &sel,
		onSelect: onSelect,
		onKey:    onKey,
		width:    w,
		height:   h,
		anchor:   anchor,
		keyMatcher: o.defKeyMatcher,
		style: lipgloss.NewStyle().
			Padding(1, 2).
			Background(lipgloss.Color("#282c34")),
	}
	o.stack = append(o.stack, s)
}

// ShowSettingsList displays a two-column settings selector (TS pi-mono: SettingsList).
func (o *Overlay) ShowSettingsList(title string, items []SelectorItem, onSelect func(value string), onKey func(key string) bool, w, h int) {
	sel := NewListSelector(title, items)
	sel.Width = w - 6
	sel.Height = h - 4
	sel.TwoColumn = true
	if o.defSelHelp != "" {
		sel.HelpText = o.defSelHelp
	}
	s := overlayState{
		selector:      &sel,
		onSelect:      onSelect,
		onKey:         onKey,
		width:         w,
		height:        h,
		stayOnSelect:  true,
		anchor:        AnchorCenter,
		keyMatcher:    o.defKeyMatcher,
		style: lipgloss.NewStyle().
			Padding(1, 2).
			Background(lipgloss.Color("#282c34")),
	}
	o.stack = append(o.stack, s)
}

// ShowForkSelector displays a 2-line per item fork selector (TS pi-mono: UserMessageSelectorComponent).
func (o *Overlay) ShowForkSelector(title, description string, items []SelectorItem, onSelect func(value string), onKey func(key string) bool, w, h int) {
	sel := NewListSelector(title, items)
	sel.Width = w - 6
	sel.Height = h - 4
	sel.ForkMode = true
	sel.ForkDescription = description
	if o.defSelHelp != "" {
		sel.HelpText = o.defSelHelp
	}
	s := overlayState{
		selector:    &sel,
		onSelect:    onSelect,
		onKey:       onKey,
		width:       w,
		height:      h,
		anchor:      AnchorCenter,
		keyMatcher:  o.defKeyMatcher,
		style: lipgloss.NewStyle().
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
			inputCursor: 0,
		killRing:   &KillRing{},
		onSubmit:   onSubmit,
		onCancel:   onCancel,
		width:      w,
		height:     h,
		anchor:     AnchorCenter,
		hints:      o.defHints,
		keyMatcher: o.defKeyMatcher,
		style: lipgloss.NewStyle().
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
		hints:           o.defHints,
		keyMatcher:      o.defKeyMatcher,
		style: lipgloss.NewStyle().
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
		killRing:       &KillRing{},
		onEditorSubmit: onSubmit,
		onEditorCancel: onCancel,
		width:          w,
		height:         h,
		anchor:         AnchorCenter,
		hints:           o.defHints,
		keyMatcher:      o.defKeyMatcher,
		style: lipgloss.NewStyle().
			Padding(1, 2).
			Background(lipgloss.Color("#282c34")),
	}
	o.stack = append(o.stack, s)
}

// top returns a pointer to the top overlay state, or nil if empty.
