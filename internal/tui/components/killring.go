package components

// KillRing is a ring buffer for Emacs-style kill/yank operations.
// Consecutive kills accumulate into a single entry.
// Supports yank (paste most recent) and yank-pop (cycle through older entries).
type KillRing struct {
	ring []string
}

// Push adds text to the kill ring.
// If accumulate is true, the text is merged with the most recent entry.
// If prepend is true, text is prepended (for backward deletion); otherwise appended.
func (k *KillRing) Push(text string, prepend, accumulate bool) {
	if text == "" {
		return
	}
	if accumulate && len(k.ring) > 0 {
		last := k.ring[len(k.ring)-1]
		if prepend {
			k.ring[len(k.ring)-1] = text + last
		} else {
			k.ring[len(k.ring)-1] = last + text
		}
	} else {
		k.ring = append(k.ring, text)
	}
}

// Peek returns the most recent entry without modifying the ring.
func (k *KillRing) Peek() string {
	if len(k.ring) == 0 {
		return ""
	}
	return k.ring[len(k.ring)-1]
}

// Rotate moves the last entry to the front (for yank-pop cycling).
func (k *KillRing) Rotate() {
	if len(k.ring) > 1 {
		last := k.ring[len(k.ring)-1]
		k.ring = append([]string{last}, k.ring[:len(k.ring)-1]...)
	}
}

// Len returns the number of entries in the kill ring.
func (k *KillRing) Len() int {
	return len(k.ring)
}
