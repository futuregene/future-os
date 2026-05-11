// Package tui provides the interactive terminal UI for xihu using Bubble Tea.
package tui

import (

	"encoding/json"
	"strings"

	"github.com/charmbracelet/lipgloss"

	"github.com/huichen/xihu/internal/session"
	"github.com/huichen/xihu/internal/tui/components"


)

// ─── Message Types ─────────────────────────────────────────────────────────

// StreamTextMsg is a chunk of streamed text from the LLM.

func (m *AppModel) showSessionTree() {
	if m.session == nil || len(m.session.Entries) == 0 {
		m.chat.AppendSystem("No entries in session")
		return
	}

	// Initialize transient tree state
	if m.treeFoldedNodes == nil {
		m.treeFoldedNodes = make(map[string]bool)
	}
	if m.treeFilterMode == "" {
		m.treeFilterMode = m.defaultTreeFilter
		if m.treeFilterMode == "" {
			m.treeFilterMode = "default"
		}
	}

	// Build parent→children map (full tree never changes)
	childrenOf := make(map[string][]session.SessionEntry)
	rootEntries := make([]session.SessionEntry, 0)
	entryByID := make(map[string]session.SessionEntry)
	for _, e := range m.session.Entries {
		entryByID[e.ID] = e
		if e.ParentID == "" {
			rootEntries = append(rootEntries, e)
		} else {
			childrenOf[e.ParentID] = append(childrenOf[e.ParentID], e)
		}
	}

	// Extract text preview from an entry
	extractPreview := func(e session.SessionEntry) string {
		if len(e.Content) == 0 {
			return e.Type
		}
		var contentBlocks []struct {
			Type string `json:"type"`
			Text string `json:"text"`
		}
		if err := json.Unmarshal(e.Content, &contentBlocks); err == nil {
			for _, block := range contentBlocks {
				if block.Type == "text" && block.Text != "" {
					return strings.ReplaceAll(block.Text, "\n", " ")
				}
			}
		}
		return e.Type
	}

	// Determine entry type character for badge
	typeChar := func(e session.SessionEntry) string {
		switch {
		case e.Type == "compaction":
			return "C"
		case e.Type == "model_change":
			return "M"
		case e.Type == "label":
			return "L"
		case e.Type == "session_info":
			return "I"
		case e.Type == "branch_summary":
			return "B"
		}
		switch e.Role {
		case "user":
			return "U"
		case "assistant":
			return "A"
		case "tool":
			return "T"
		case "system":
			return "S"
		}
		return "?"
	}

	// Check if entry has children
	hasChildren := func(id string) bool {
		return len(childrenOf[id]) > 0
	}

	// Check if entry passes the current filter mode
	passesFilter := func(e session.SessionEntry, mode string) bool {
		isSettings := e.Type == "label" || e.Type == "custom" || e.Type == "model_change" ||
			e.Type == "thinking_level_change" || e.Type == "session_info"
		switch mode {
		case "user-only":
			return e.Role == "user"
		case "no-tools":
			return !isSettings && e.Role != "tool"
		case "labeled-only":
			return e.Label != "" || e.Type == "label"
		default: // "default"
			return !isSettings
		}
	}

	// Build active path set (from root to current leaf)
	// TS pi-mono: walks from currentLeafId to root, marks all ancestors + leaf with bullet
	currentLeafID := session.EffectiveLeafID(m.session)
	activePath := make(map[string]bool)
	if currentLeafID != "" {
		for id := currentLeafID; id != ""; {
			activePath[id] = true
			if e, ok := entryByID[id]; ok {
				id = e.ParentID
			} else {
				break
			}
		}
	}

	var treeItemIndents []int

	// buildTreeItems rebuilds items from current fold/filter/search state
	buildTreeItems := func() []components.SelectorItem {
		type flatEntry struct {
			entry   session.SessionEntry
			indent  int
			prefix  string // "├─ " or "└─ " or ""
			gutters []bool // true = show │ at each level
		}
		var flat []flatEntry

		var walk func(entries []session.SessionEntry, indent int, gutters []bool)
		walk = func(entries []session.SessionEntry, indent int, gutters []bool) {
			for i, e := range entries {
				isLast := i == len(entries)-1
				prefix := ""
				if indent > 0 {
					if isLast {
						prefix = "└─ "
					} else {
						prefix = "├─ "
					}
				}
				flat = append(flat, flatEntry{entry: e, indent: indent, prefix: prefix, gutters: gutters})

				// Skip children of folded nodes
				if m.treeFoldedNodes[e.ID] {
					continue
				}

				children := childrenOf[e.ID]
				if len(children) > 0 {
					// Build gutters for children: extend with whether this node continues siblings
					childGutters := make([]bool, len(gutters))
					copy(childGutters, gutters)
					if indent > 0 {
						childGutters = append(childGutters, !isLast)
					}
					walk(children, indent+1, childGutters)
				}
			}
		}
		walk(rootEntries, 0, nil)

		// Apply filter mode and search
		var filtered []flatEntry
		for _, fe := range flat {
			if !passesFilter(fe.entry, m.treeFilterMode) {
				continue
			}
			// Apply search query
			if m.treeSearchQuery != "" {
				preview := strings.ToLower(extractPreview(fe.entry))
				id := strings.ToLower(fe.entry.ID)
				role := strings.ToLower(fe.entry.Role)
				q := strings.ToLower(m.treeSearchQuery)
				if !fuzzyMatch(q, preview) && !fuzzyMatch(q, id) && !fuzzyMatch(q, role) {
					continue
				}
			}
			filtered = append(filtered, fe)
		}

		items := make([]components.SelectorItem, 0, len(filtered))
		treeItemIndents = make([]int, 0, len(filtered))
		for _, fe := range filtered {
			e := fe.entry

			// Build prefix with gutters at each level
			treePrefix := ""
			for _, show := range fe.gutters {
				if show {
					treePrefix += "│ "
				} else {
					treePrefix += "  "
				}
			}
			if fe.prefix != "" {
				treePrefix += fe.prefix
			}

			// Fold indicator
			foldIndicator := ""
			if hasChildren(e.ID) {
				if m.treeFoldedNodes[e.ID] {
					foldIndicator = "⊞ "
				} else {
					foldIndicator = "⊟ "
				}
			}

			// Active path marker
			pathMarker := ""
			if activePath[e.ID] {
				pathMarker = "• "
			}

			// Type badge
			tc := typeChar(e)

			// Per-type color (TS pi-mono: themed colors per entry type)
			typeColor := treeColorForEntry(e)

			// Content preview (truncated)
			preview := extractPreview(e)
			if len(preview) > 50 {
				preview = preview[:47] + "..."
			}

			// Build colored label: dim connectors + colored badge + colored preview
			dimTree := lipgloss.NewStyle().Faint(true).Foreground(lipgloss.Color("#5c6370"))
			connectors := dimTree.Render(treePrefix + foldIndicator + pathMarker)
			badge := typeColor.Render("[" + tc + "]")
			previewColored := typeColor.Render(preview)
			label := connectors + badge + " " + previewColored
			desc := ""
			if m.treeShowTimestamps {
				desc = e.Timestamp.Format("01/02 15:04")
				if e.ParentID != "" {
					desc = e.ID[:8] + " · " + desc
				}
			} else {
				desc = e.ID[:8]
			}
			items = append(items, components.SelectorItem{
				Label:       label,
				Description: desc,
				Value:       e.ID,
			})
			treeItemIndents = append(treeItemIndents, fe.indent)
		}
		return items
	}

	// Build title with filter mode indicator
	buildTitle := func() string {
		return "Session Tree"
	}

	// Custom key handler for tree-specific keys
	onKey := func(key string) bool {
		needRebuild := false
		switch key {
		case "ctrl+d":
			m.treeFilterMode = "default"
			needRebuild = true
		case "ctrl+t":
			if m.treeFilterMode == "no-tools" {
				m.treeFilterMode = "default"
			} else {
				m.treeFilterMode = "no-tools"
			}
			needRebuild = true
		case "ctrl+u":
			if m.treeFilterMode == "user-only" {
				m.treeFilterMode = "default"
			} else {
				m.treeFilterMode = "user-only"
			}
			needRebuild = true
		case "ctrl+l":
			if m.treeFilterMode == "labeled-only" {
				m.treeFilterMode = "default"
			} else {
				m.treeFilterMode = "labeled-only"
			}
			needRebuild = true
		case "ctrl+a":
			if m.treeFilterMode == "all" {
				m.treeFilterMode = "default"
			} else {
				m.treeFilterMode = "all"
			}
			needRebuild = true
		case "ctrl+o":
			modes := []string{"default", "no-tools", "user-only", "labeled-only", "all"}
			for i, mode := range modes {
				if mode == m.treeFilterMode {
					m.treeFilterMode = modes[(i+1)%len(modes)]
					break
				}
			}
			needRebuild = true
		case "ctrl+shift+o":
			modes := []string{"default", "no-tools", "user-only", "labeled-only", "all"}
			for i, mode := range modes {
				if mode == m.treeFilterMode {
					m.treeFilterMode = modes[(i-1+len(modes))%len(modes)]
					break
				}
			}
			needRebuild = true
		case "ctrl+left", "alt+left":
			// Fold current node; if not foldable, jump to branch segment start (TS pi-mono: findBranchSegmentStart)
			val := m.overlay.SelectedValue()
			if val != "" && hasChildren(val) && !m.treeFoldedNodes[val] {
				m.treeFoldedNodes[val] = true
				needRebuild = true
			} else {
				// Jump to parent branch point: walk up to find item with lower indent
				idx := m.overlay.SelectedIndex()
				if idx > 0 && idx < len(m.treeItemIndents) {
					curIndent := m.treeItemIndents[idx]
					for i := idx - 1; i >= 0; i-- {
						if m.treeItemIndents[i] < curIndent {
							m.overlay.SelectIdx(i)
							break
						}
					}
				}
			}
		case "ctrl+right", "alt+right":
			// Unfold current node; if not folded, jump to first child or next sibling (TS pi-mono: branch segment)
			val := m.overlay.SelectedValue()
			if val != "" && m.treeFoldedNodes[val] {
				delete(m.treeFoldedNodes, val)
				needRebuild = true
			} else {
				// Jump to first child; if no children, move to next item (TS pi-mono)
				idx := m.overlay.SelectedIndex()
				found := false
				if idx >= 0 && idx < len(m.treeItemIndents)-1 {
					curIndent := m.treeItemIndents[idx]
					for i := idx + 1; i < len(m.treeItemIndents); i++ {
						if m.treeItemIndents[i] < curIndent {
							break // hit a higher-level node
						}
						if m.treeItemIndents[i] > curIndent {
							m.overlay.SelectIdx(i)
							found = true
							break
						}
					}
				}
				if !found && idx < m.overlay.ItemCount()-1 {
					m.overlay.SelectIdx(idx + 1)
				}
			}
		case "enter":
			// Toggle fold on current node if foldable; otherwise let default handler select
			val := m.overlay.SelectedValue()
			if val != "" && hasChildren(val) {
				if m.treeFoldedNodes[val] {
					delete(m.treeFoldedNodes, val)
				} else {
					m.treeFoldedNodes[val] = true
				}
				needRebuild = true
				return true // consume Enter so it doesn't close overlay
			}
			return false // let default handler process (select item)
		case "shift+l":
			// Edit tree label: close tree and put label text in editor
			val := m.overlay.SelectedValue()
			if val != "" {
				if e, ok := entryByID[val]; ok {
					labelText := ""
					if e.Type == "label" && e.Label != "" {
						labelText = e.Label
					} else {
						labelText = extractPreview(e)
					}
					m.overlay.Hide()
					m.input.SetValue("/name " + labelText)
					m.chat.AppendSystem("Editing label — press Enter to save, Esc to cancel")
				}
			}
			return true
		case "shift+t":
			// Toggle label timestamp display
			m.treeShowTimestamps = !m.treeShowTimestamps
			if m.treeShowTimestamps {
				m.chat.AppendSystem("Timestamps shown")
			} else {
				m.chat.AppendSystem("Timestamps hidden")
			}
			needRebuild = true
			return true
		case "backspace":
			if m.treeSearchQuery != "" {
				m.treeSearchQuery = m.treeSearchQuery[:len(m.treeSearchQuery)-1]
				needRebuild = true
				return true
			}
			return false // let default handler process
		case "esc":
			// Clear search first, then close overlay on second Esc
			if m.treeSearchQuery != "" {
				m.treeSearchQuery = ""
				needRebuild = true
				return true
			}
			return false // let default handler close overlay
		default:
			// Printable characters update search
			if len(key) == 1 && key[0] >= 32 && key[0] < 127 {
				m.treeSearchQuery += key
				needRebuild = true
				return true
			}
			return false
		}

		if needRebuild {
			items := buildTreeItems()
			m.treeItemIndents = treeItemIndents
			m.overlay.ReplaceItems(buildTitle(), items)
			// Sync search query with selector filter display
			if m.treeSearchQuery != "" {
				m.overlay.SetFilter(m.treeSearchQuery)
			}
		}
		return true
	}

	// Build initial items
	items := buildTreeItems()
	m.treeItemIndents = treeItemIndents
	title := buildTitle()
	h := len(items) + 5
	if h < 10 {
		h = 10
	}
	if h > 24 {
		h = 24
	}
	w := 86

	m.overlay.ShowSelectorWithKeyHandler(title, items, func(value string) {
		if value != "" {
			// TS pi-mono: selecting the current leaf is a no-op
			if value == currentLeafID {
				m.chat.AppendSystem("Already at this point")
			} else {
				// Navigate to selected entry
				m.sessMgr.Branch(m.session, value)
				// Re-render chat from the new leaf path (TS pi-mono: chatContainer.clear + renderInitialMessages)
				m.chat.Clear()
				for _, entry := range m.session.Entries {
					ce := sessionEntryToChatEntry(entry)
					if ce != nil {
						m.chat.AppendChatEntry(*ce)
					}
				}
				m.chat.AppendSystem("Navigated to selected point")
			}
		}
		// Reset tree state on close
		m.treeFoldedNodes = nil
		m.treeFilterMode = ""
		m.treeSearchQuery = ""
	}, onKey, w, h)
}

// treeColorForEntry returns a lipgloss style with the appropriate color for a tree entry.
// Matches TS pi-mono's themed per-type colors.
func treeColorForEntry(e session.SessionEntry) lipgloss.Style {
	// Special types
	switch e.Type {
	case "compaction":
		return lipgloss.NewStyle().Foreground(lipgloss.Color("#c678dd"))
	case "branch_summary", "label":
		return lipgloss.NewStyle().Foreground(lipgloss.Color("#e5c07b"))
	case "model_change", "thinking_level_change", "custom", "session_info":
		return lipgloss.NewStyle().Faint(true).Foreground(lipgloss.Color("#5c6370"))
	}
	// Role-based colors
	switch e.Role {
	case "user":
		return lipgloss.NewStyle().Foreground(lipgloss.Color("#61afef"))
	case "assistant":
		return lipgloss.NewStyle().Foreground(lipgloss.Color("#98c379"))
	case "tool":
		return lipgloss.NewStyle().Faint(true).Foreground(lipgloss.Color("#5c6370"))
	case "system":
		return lipgloss.NewStyle().Faint(true).Foreground(lipgloss.Color("#5c6370"))
	}
	return lipgloss.NewStyle().Foreground(lipgloss.Color("#abb2bf"))
}

// forkFromEntry truncates the current session at the given entry ID,
// keeping entries up to and including the specified entry (TS pi-mono: fork).
// The original session is preserved; the current session becomes the fork.
