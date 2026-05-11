package components

import (

)

// isPunct returns true for punctuation characters (word boundary, TS pi-mono style).

func (o *Overlay) SetMargin(margin int) {
	s := o.top()
	if s != nil {
		s.margin = margin
	}
}

// OffsetX returns the horizontal offset of the top overlay.
func (o *Overlay) OffsetX() int {
	if s := o.top(); s != nil {
		return s.offsetX
	}
	return 0
}

// OffsetY returns the vertical offset of the top overlay.
func (o *Overlay) OffsetY() int {
	if s := o.top(); s != nil {
		return s.offsetY
	}
	return 0
}

// SetOffset sets the position offset from the anchor point for the top overlay.
// Positive offsetX shifts right, positive offsetY shifts down (TS pi-mono).
func (o *Overlay) SetOffset(x, y int) {
	if s := o.top(); s != nil {
		s.offsetX = x
		s.offsetY = y
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

// SetOnSelectionChange sets a callback invoked when the selector cursor moves.
// Used for live preview (TS pi-mono: theme preview on cursor move).
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
func (o *Overlay) HideAll() {
	o.stack = nil
}

// CountdownTickMsg is sent every second when a dialog has an active countdown timer.
func (o *Overlay) SetOnSelectionChange(cb func(int)) {
	if s := o.top(); s != nil {
		s.onSelectionChange = cb
	}
}

// SetKeyMatcher sets a keybinding-aware matcher function on the top overlay.
// When set, the overlay uses this to check key presses against binding IDs
// instead of hardcoded key strings (TS pi-mono: kb.matches()).
func (o *Overlay) SetKeyMatcher(fn func(keyString, bindingID string) bool) {
	s := o.top()
	if s != nil {
		s.keyMatcher = fn
	}
}

// SetHints sets dynamic keybinding-aware hint strings for the top overlay.
// Empty strings are ignored (the overlay uses its built-in defaults).
// submit, cancel, extEditor correspond to InputSubmit, SelectCancel, GlobalExternalEditor.
func (o *Overlay) SetHints(submit, cancel, extEditor string) {
	s := o.top()
	if s != nil {
		if submit != "" {
			s.hints.Submit = submit
		}
		if cancel != "" {
			s.hints.Cancel = cancel
		}
		if extEditor != "" {
			s.hints.ExternalEditor = extEditor
		}
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
