// Package tui provides the interactive terminal UI for xihu using Bubble Tea.
package tui

import (

	"fmt"
	"os"
	"strings"


	"github.com/huichen/xihu/internal/session"
	"github.com/huichen/xihu/internal/tui/components"


)

// ─── Message Types ─────────────────────────────────────────────────────────

// StreamTextMsg is a chunk of streamed text from the LLM.

func (m *AppModel) showSessionSelector() {
	if m.sessMgr == nil || m.session == nil {
		m.chat.AppendSystem("No session manager available")
		return
	}

	// Shorten CWD with ~
	cwd := m.session.CWD
	if home, _ := os.UserHomeDir(); home != "" && strings.HasPrefix(cwd, home) {
		cwd = "~" + cwd[len(home):]
	}

	// Local state for sort and filter
	sortByDate := true // true=by date (newest first), false=by name
	namedOnly := false
	showPath := false     // TS pi-mono: togglePath — show CWD path instead of session name
	threadedMode := true // TS pi-mono: threaded tree view (Ctrl+T toggles)
	sessionNames := make(map[string]string) // session ID → name for rename
	foldedNodes := make(map[string]bool)     // TS pi-mono: folded nodes in tree

	// sessionTreeNode represents a node in the session tree.
	type sessionTreeNode struct {
		session  *session.Session
		children []*sessionTreeNode
	}

	// flatSessionNode is a flattened tree node for display.
	type flatSessionNode struct {
		session           *session.Session
		depth             int
		isLast            bool
		ancestorContinues []bool
	}

	var currentTree []*sessionTreeNode // cached tree for foldable checks (set in buildItems)

	// buildSessionTree builds a tree from sessions based on ParentSessionID.
	buildSessionTree := func(sessions []*session.Session) []*sessionTreeNode {
		byID := make(map[string]*sessionTreeNode)
		for _, s := range sessions {
			byID[s.ID] = &sessionTreeNode{session: s}
		}
		roots := make([]*sessionTreeNode, 0)
		for _, s := range sessions {
			node := byID[s.ID]
			if s.ParentSessionID != "" {
				if parent, ok := byID[s.ParentSessionID]; ok {
					parent.children = append(parent.children, node)
					continue
				}
			}
			roots = append(roots, node)
		}
		return roots
	}

	// flattenTree flattens a session tree for display.
	// Skips descendants of folded nodes.
	flattenTree := func(roots []*sessionTreeNode) []flatSessionNode {
		var result []flatSessionNode
		var walk func(*sessionTreeNode, int, []bool, bool)
		walk = func(node *sessionTreeNode, depth int, ancestorContinues []bool, isLast bool) {
			result = append(result, flatSessionNode{
				session:           node.session,
				depth:             depth,
				isLast:            isLast,
				ancestorContinues: ancestorContinues,
			})
			// Skip children if this node is folded
			if threadedMode && foldedNodes[node.session.ID] {
				return
			}
			for i := 0; i < len(node.children); i++ {
				childIsLast := i == len(node.children)-1
				childContinues := make([]bool, 0, len(ancestorContinues)+1)
				childContinues = append(childContinues, ancestorContinues...)
				if depth > 0 && !isLast {
					childContinues = append(childContinues, true)
				} else {
					childContinues = append(childContinues, false)
				}
				walk(node.children[i], depth+1, childContinues, childIsLast)
			}
		}
		for i := 0; i < len(roots); i++ {
			walk(roots[i], 0, nil, i == len(roots)-1)
		}
		return result
	}

	// isFoldable returns true if a node has children (can be folded).
	isFoldable := func(tree []*sessionTreeNode, id string) bool {
		for _, root := range tree {
			var find func(*sessionTreeNode) bool
			find = func(n *sessionTreeNode) bool {
				if n.session.ID == id {
					return len(n.children) > 0
				}
				for _, c := range n.children {
					if find(c) {
						return true
					}
				}
				return false
			}
			if find(root) {
				return true
			}
		}
		return false
	}

	// buildTreePrefix builds the box-drawing prefix for a tree node.
	// Shows fold indicator ⊟ for foldable nodes, ⊞ for folded nodes.
	buildTreePrefix := func(depth int, isLast bool, ancestorContinues []bool, nodeID string) string {
		if depth == 0 {
			// Fold indicator for root nodes
			if threadedMode && isFoldable(currentTree, nodeID) {
				if foldedNodes[nodeID] {
					return "⊞ "
				}
				return "⊟ "
			}
			return ""
		}
		var sb strings.Builder
		for _, continues := range ancestorContinues {
			if continues {
				sb.WriteString("│  ")
			} else {
				sb.WriteString("   ")
			}
		}
		// Branch character with fold indicator
		if isLast {
			sb.WriteString("└")
		} else {
			sb.WriteString("├")
		}
		if threadedMode && isFoldable(currentTree, nodeID) {
			if foldedNodes[nodeID] {
				sb.WriteString("⊞ ")
			} else {
				sb.WriteString("⊟ ")
			}
		} else {
			sb.WriteString("─ ")
		}
		return sb.String()
	}

	// buildItems loads sessions and builds selector items
	buildItems := func() []components.SelectorItem {
		rawSessions, err := m.sessMgr.List(m.session.CWD)
		if err != nil || len(rawSessions) == 0 {
			return nil
		}

		// Convert to pointer slice
		sessions := make([]*session.Session, len(rawSessions))
		for i := range rawSessions {
			sessions[i] = &rawSessions[i]
		}

		// Filter: named only
		if namedOnly {
			filtered := sessions[:0]
			for _, s := range sessions {
				if s.GetSessionName() != "" {
					filtered = append(filtered, s)
				}
			}
			sessions = filtered
		}

		// Sort
		if sortByDate {
			// Already sorted by date from List()
		} else {
			// Sort by name (or ID if no name)
			for i := 0; i < len(sessions); i++ {
				for j := i + 1; j < len(sessions); j++ {
					ni := sessions[i].GetSessionName()
					nj := sessions[j].GetSessionName()
					if ni == "" {
						ni = sessions[i].ID
					}
					if nj == "" {
						nj = sessions[j].ID
					}
					if ni > nj {
						sessions[i], sessions[j] = sessions[j], sessions[i]
					}
				}
			}
		}

		// Session name map
		sessionNames = make(map[string]string)

		// Build flat nodes: threaded tree or flat list
		var flatNodes []flatSessionNode
		if threadedMode {
			currentTree = buildSessionTree(sessions)
			flatNodes = flattenTree(currentTree)
		} else {
			flatNodes = make([]flatSessionNode, len(sessions))
			for i, s := range sessions {
				flatNodes[i] = flatSessionNode{
					session: s,
					depth:   0,
					isLast:  i == len(sessions)-1,
				}
			}
		}

		items := make([]components.SelectorItem, 0, len(flatNodes))
		for _, fn := range flatNodes {
			s := fn.session
			sessionNames[s.ID] = s.GetSessionName()
			isCurrent := s.ID == m.session.ID
			label := s.ID
			if showPath {
				label = s.CWD
				if home, _ := os.UserHomeDir(); home != "" && strings.HasPrefix(label, home) {
					label = "~" + label[len(home):]
				}
			} else {
				name := s.GetSessionName()
				if name != "" {
					label = name
				}
			}
			if isCurrent {
				label = "✓ " + label
			}
			// Tree prefix for threaded mode
			prefix := buildTreePrefix(fn.depth, fn.isLast, fn.ancestorContinues, s.ID)
			count := len(s.Entries)
			age := formatRelativeDate(s.UpdatedAt)
			desc := fmt.Sprintf("%d msgs · %s · %s", count, age, s.ID)
			if isCurrent {
				desc = "current · " + desc
			}
			items = append(items, components.SelectorItem{
				Label:       prefix + label,
				Description: desc,
				Value:       s.ID,
			})
		}
		return items
	}

	items := buildItems()
	if len(items) == 0 {
		m.chat.AppendSystem("No saved sessions found")
		return
	}

	buildTitle := func() string {
		title := fmt.Sprintf("Resume Session (%s)", cwd)
		if namedOnly {
			title += " [named]"
		}
		if !sortByDate {
			title += " [by name]"
		}
		if showPath {
			title += " [paths]"
		}
		if !threadedMode {
			title += " [flat]"
		}
		return title
	}

	confirmingDeletePath := "" // TS pi-mono: two-step delete confirmation

	onKey := func(key string) bool {
		needRebuild := false

		// Handle delete confirmation state first (TS pi-mono)
		if confirmingDeletePath != "" {
			switch key {
			case "enter":
				pathToDelete := confirmingDeletePath
				confirmingDeletePath = ""
				err := m.sessMgr.Delete(pathToDelete, m.session.CWD)
				if err != nil {
					m.chat.AppendSystem("Failed to delete session: " + err.Error())
				} else {
					m.chat.AppendSystem("Session moved to trash")
				}
				m.overlay.SetHelpText("")
				needRebuild = true
				return true
			case "esc":
				confirmingDeletePath = ""
				m.overlay.SetHelpText("")
				needRebuild = true
				return true
			default:
				return true // ignore all other keys while confirming
			}
		}

		switch key {
		case "ctrl+s":
			sortByDate = !sortByDate
			needRebuild = true
		case "ctrl+n":
			namedOnly = !namedOnly
			needRebuild = true
		case "ctrl+p":
			showPath = !showPath
			needRebuild = true
		case "ctrl+t":
			threadedMode = !threadedMode
			needRebuild = true
		case "h":
			// Fold current node (TS pi-mono: tree.foldOrUp)
			if threadedMode {
				val := m.overlay.SelectedValue()
				if val != "" && isFoldable(currentTree, val) && !foldedNodes[val] {
					foldedNodes[val] = true
					needRebuild = true
				}
			}
		case "l":
			// Unfold current node (TS pi-mono: tree.unfoldOrDown)
			if threadedMode {
				val := m.overlay.SelectedValue()
				if val != "" && foldedNodes[val] {
					foldedNodes[val] = false
					needRebuild = true
				}
			}
		case "ctrl+backspace", "ctrl+d":
			// Initiate delete confirmation (TS pi-mono: two-step)
			val := m.overlay.SelectedValue()
			if val != "" {
				if val == m.session.ID {
					m.chat.AppendSystem("Cannot delete the currently active session")
				} else {
					confirmingDeletePath = val
					m.overlay.SetHelpText("Delete session? Enter confirm \xc2\xb7 Esc cancel")
				}
			}
		case "ctrl+r":
			// Rename session: close selector and set editor to /name for inline editing
			val := m.overlay.SelectedValue()
			if val != "" {
				if name, ok := sessionNames[val]; ok && name != "" {
					m.overlay.Hide()
					m.input.SetValue("/name " + name)
				} else {
					m.overlay.Hide()
					m.input.SetValue("/name ")
				}
				m.chat.AppendSystem("Editing session name — press Enter to save, Esc to cancel")
			}
			return true
		case "backspace":
			// Let default handler clear filter
			return false
		case "esc":
			// Let default handler close
			return false
		default:
			return false
		}

		if needRebuild {
			newItems := buildItems()
			if len(newItems) == 0 {
				m.overlay.Hide()
				m.chat.AppendSystem("No sessions match the filter")
				return true
			}
			m.overlay.ReplaceItems(buildTitle(), newItems)
		}
		return true
	}

	h := len(items) + 5
	if h < 10 {
		h = 10
	}
	if h > 20 {
		h = 20
	}
	m.overlay.ShowSelectorWithKeyHandler(buildTitle(), items, func(value string) {
		if value != "" && m.program != nil {
			m.program.Send(components.SelectorChosenMsg{Value: "session:" + value})
		}
	}, onKey, 70, h)
}

// showScopedModelSelector opens a model selector overlay for scoped model management (TS pi-mono: /scoped-models).
// Shows all available models with their scoped status (enabled/disabled).
