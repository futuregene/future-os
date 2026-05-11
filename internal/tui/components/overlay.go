package components

import (

	tea "github.com/charmbracelet/bubbletea"
)

// isPunct returns true for punctuation characters (word boundary, TS pi-mono style).

func NewOverlay() Overlay {
	return Overlay{}
}

// SetDefaultHints sets the default key hints for new interactive overlays.
// These are inherited by ShowInput, ShowEditor, and ShowCustom.
// submit, cancel, extEditor correspond to InputSubmit, SelectCancel, GlobalExternalEditor.
func (o *Overlay) SetDefaultHints(submit, cancel, extEditor string) {
	o.defHints = overlayHints{Submit: submit, Cancel: cancel, ExternalEditor: extEditor}
}

// SetDefaultKeyMatcher sets the default keybinding matcher inherited by all new overlays.
// This enables keybinding-aware dispatch in selectors, custom dialogs, and text overlays.
func (o *Overlay) SetDefaultKeyMatcher(fn func(keyString, bindingID string) bool) {
	o.defKeyMatcher = fn
}

// SetDefaultSelectorHelp sets the default help text for new selector overlays.
// Use this to show keybinding-aware hints (TS pi-mono: keyHint/keyText).
// Example: "↑↓ navigate  Enter select  Esc cancel  / filter"
func (o *Overlay) SetDefaultSelectorHelp(text string) {
	o.defSelHelp = text
}

// SetHelpText updates the help text on the currently displayed selector overlay.
// This is used for dynamic hints like delete confirmation (TS pi-mono: confirmingDeletePath).
func (o *Overlay) SetHelpText(text string) {
	if len(o.stack) > 0 {
		top := &o.stack[len(o.stack)-1]
		if top.selector != nil {
			top.selector.HelpText = text
		}
	}
}

// SetSelectionInfoFunc sets the selection info callback on the current selector overlay.
// The function receives the selected index and item, and returns an info line string.
// Used for "Model Name: GPT-4o" display in model selector (TS pi-mono: ModelSelectorComponent).
func (o *Overlay) SetSelectionInfoFunc(fn func(index int, item SelectorItem) string) {
	if len(o.stack) > 0 {
		top := &o.stack[len(o.stack)-1]
		if top.selector != nil {
			top.selector.SelectionInfoFunc = fn
		}
	}
}

// SetNoMatchText sets the no-match message on the current selector overlay.
func (o *Overlay) SetNoMatchText(text string) {
	if len(o.stack) > 0 {
		top := &o.stack[len(o.stack)-1]
		if top.selector != nil {
			top.selector.NoMatchText = text
		}
	}
}

// RefreshItems updates the items on the current selector without hiding/reopening.
// Preserves the selected index (clamped to new range). Use this to avoid the visual
// stutter of closing and reopening the overlay when items change (TS pi-mono: SettingsList.updateValue).
func (o *Overlay) RefreshItems(items []SelectorItem) {
	if len(o.stack) > 0 && o.stack[len(o.stack)-1].selector != nil {
		sel := o.stack[len(o.stack)-1].selector
		oldIdx := sel.Selected
		sel.SetItems(items)
		if oldIdx < len(items) {
			sel.Selected = oldIdx
		}
	}
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

// resolveSize computes an overlay dimension from a string value.
// Integers are used directly; percentage strings (e.g. "80%") are computed from the terminal size.


// updateCustom handles key events for custom dialog overlays.
