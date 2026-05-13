// Package tui provides the interactive terminal UI for xihu using Bubble Tea.
package tui

import (

	"fmt"
	"os"
	"strings"
	"time"


	"github.com/huichen/xihu/internal/tui/components"


)

// ─── Message Types ─────────────────────────────────────────────────────────

// StreamTextMsg is a chunk of streamed text from the LLM.

func (m *AppModel) showSettingsSelector() {
	// Initialize defaults
	if m.defaultTreeFilter == "" {
		m.defaultTreeFilter = "default"
	}
	if m.doubleEscapeAction == "" {
		m.doubleEscapeAction = "tree"
	}
	if m.steeringMode == "" {
		m.steeringMode = "one-at-a-time"
	}
	if m.followUpMode == "" {
		m.followUpMode = "one-at-a-time"
	}
	if m.transport == "" {
		m.transport = "auto"
	}

	// Check terminal image support (Kitty or iTerm2 protocol)
	hasImages := os.Getenv("TERM") == "xterm-kitty" || os.Getenv("ITERM_PROFILE") != "" || os.Getenv("KITTY_WINDOW_ID") != ""

	items := []components.SelectorItem{
		{Label: "Auto-compact: " + boolToStr(m.autoCompact), Description: "Automatically compact context when it gets too large", Value: "autocompact"},
	}

	// Image settings (only shown when terminal supports images)
	if hasImages {
		items = append(items,
			components.SelectorItem{Label: "Show images: " + boolToStr(m.showImages), Description: "Render images inline in the terminal", Value: "show_images"},
			components.SelectorItem{Label: "Image width: " + fmt.Sprintf("%d cells", m.imageWidthCells), Description: "Width of inline images in terminal cells", Value: "image_width"},
		)
	}

	items = append(items,
		components.SelectorItem{Label: "Auto-resize images: " + boolToStr(m.autoResizeImages), Description: "Automatically resize images on terminal resize", Value: "auto_resize_images"},
		components.SelectorItem{Label: "Block images: " + boolToStr(m.blockImages), Description: "Block image rendering (security)", Value: "block_images"},
		components.SelectorItem{Label: "Skill commands: " + boolToStr(m.skillCommands), Description: "Enable slash-command skill invocation", Value: "skill_commands"},
		components.SelectorItem{Label: "Show hardware cursor: " + boolToStr(m.showHardwareCursor), Description: "Show terminal block cursor for IME support", Value: "hwcursor"},
		components.SelectorItem{Label: "Editor padding: " + fmt.Sprintf("%d", m.editorPadding), Description: "Horizontal padding for the input editor (0-3)", Value: "editor_padding"},
		components.SelectorItem{Label: "Autocomplete max items: " + fmt.Sprintf("%d items", m.autocompleteMax), Description: "Maximum visible items in autocomplete dropdown", Value: "autocomplete_max"},
		components.SelectorItem{Label: "Clear on shrink: " + boolToStr(m.clearOnShrink), Description: "Clear editor content when terminal shrinks", Value: "clear_on_shrink"},
		components.SelectorItem{Label: "Terminal progress: " + boolToStr(m.terminalProgress), Description: "Show terminal progress messages during operations", Value: "terminal_progress"},
		components.SelectorItem{Label: "Steering mode: " + m.steeringMode, Description: "How follow-up messages are queued: one-at-a-time or all", Value: "steering"},
		components.SelectorItem{Label: "Follow-up mode: " + m.followUpMode, Description: "How follow-up responses are delivered: one-at-a-time or all", Value: "follow_up"},
		components.SelectorItem{Label: "Transport: " + m.transport, Description: "API transport mechanism: sse, websocket, websocket-cached, or auto", Value: "transport"},
		components.SelectorItem{Label: "Hide thinking: " + boolToStr(m.chat.HideAllThinking), Description: "Hide thinking blocks in assistant responses", Value: "hide_thinking"},
		components.SelectorItem{Label: "Collapse changelog: " + boolToStr(m.collapseChangelog), Description: "Show condensed changelog after updates", Value: "collapse_changelog"},
		components.SelectorItem{Label: "Quiet startup: " + boolToStr(m.quietStartup), Description: "Suppress welcome message on startup", Value: "quiet_startup"},
		components.SelectorItem{Label: "Install telemetry: " + boolToStr(m.installTelemetry), Description: "Opt-in to anonymous installation telemetry", Value: "install_telemetry"},
		components.SelectorItem{Label: "Double-escape action: " + m.doubleEscapeAction, Description: "Action on Esc\u00d72 with empty editor: tree, fork, or none", Value: "esc2x"},
		components.SelectorItem{Label: "Tree filter mode: " + m.defaultTreeFilter, Description: "Default filter when opening /tree", Value: "treefilter"},
		components.SelectorItem{Label: "Warnings\u2026", Description: "Configure warning display settings", Value: "warnings"},
		components.SelectorItem{Label: "Thinking level: " + m.thinkingLevel, Description: "Select reasoning depth for the model", Value: "thinking"},
		components.SelectorItem{Label: "Theme: " + m.theme.Name, Description: "Select the UI color theme", Value: "theme"},
	)

	// Session info
	cwd := m.session.CWD
	if home, _ := os.UserHomeDir(); home != "" && strings.HasPrefix(cwd, home) {
		cwd = "~" + cwd[len(home):]
	}
	items = append(items,
		components.SelectorItem{Label: "Session: " + m.session.ID, Description: cwd, Value: "session"},
		components.SelectorItem{Label: "Session name: " + m.session.GetSessionName(), Description: "Use /name <name> to set", Value: "name"},
	)

	h := len(items) + 4
	if h > 30 {
		h = 30
	}
	if h < 10 {
		h = 10
	}

	m.overlay.ShowSelectorStayOnSelect("Settings \xe2\x80\x94 Enter/Space to change \xc2\xb7 Esc to cancel", items, func(value string) {
		switch value {
		case "autocompact":
			m.autoCompact = !m.autoCompact
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "show_images":
			m.showImages = !m.showImages
			m.chat.SetShowImages(m.showImages)
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "image_width":
			widths := []int{60, 80, 120}
			for i, w := range widths {
				if w == m.imageWidthCells {
					m.imageWidthCells = widths[(i+1)%len(widths)]
					m.chat.SetImageWidth(m.imageWidthCells)
					break
				}
			}
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "auto_resize_images":
			m.autoResizeImages = !m.autoResizeImages
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "block_images":
			m.blockImages = !m.blockImages
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "skill_commands":
			m.skillCommands = !m.skillCommands
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "hwcursor":
			m.showHardwareCursor = !m.showHardwareCursor
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "editor_padding":
			pads := []int{0, 1, 2, 3}
			for i, p := range pads {
				if p == m.editorPadding {
					m.editorPadding = pads[(i+1)%len(pads)]
					break
				}
			}
			m.input.SetPaddingX(m.editorPadding)
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "autocomplete_max":
			maxes := []int{3, 5, 7, 10, 15, 20}
			for i, n := range maxes {
				if n == m.autocompleteMax {
					m.autocompleteMax = maxes[(i+1)%len(maxes)]
					break
				}
			}
			m.autocomplete.SetMaxVisible(m.autocompleteMax)
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "clear_on_shrink":
			m.clearOnShrink = !m.clearOnShrink
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "terminal_progress":
			m.terminalProgress = !m.terminalProgress
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "steering":
			if m.steeringMode == "one-at-a-time" {
				m.steeringMode = "all"
			} else {
				m.steeringMode = "one-at-a-time"
			}
			m.agent.Loop().SteeringQueue.Mode = m.steeringMode
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "follow_up":
			if m.followUpMode == "one-at-a-time" {
				m.followUpMode = "all"
			} else {
				m.followUpMode = "one-at-a-time"
			}
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "transport":
			modes := []string{"sse", "websocket", "websocket-cached", "auto"}
			for i, mode := range modes {
				if mode == m.transport {
					m.transport = modes[(i+1)%len(modes)]
					break
				}
			}
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "hide_thinking":
			m.chat.HideAllThinking = !m.chat.HideAllThinking
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "collapse_changelog":
			m.collapseChangelog = !m.collapseChangelog
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "quiet_startup":
			m.quietStartup = !m.quietStartup
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "install_telemetry":
			m.installTelemetry = !m.installTelemetry
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "esc2x":
			modes := []string{"tree", "fork", "none"}
			for i, mode := range modes {
				if mode == m.doubleEscapeAction {
					m.doubleEscapeAction = modes[(i+1)%len(modes)]
					break
				}
			}
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "treefilter":
			modes := []string{"default", "no-tools", "user-only", "labeled-only", "all"}
			for i, mode := range modes {
				if mode == m.defaultTreeFilter {
					m.defaultTreeFilter = modes[(i+1)%len(modes)]
					break
				}
			}
			m.treeFilterMode = m.defaultTreeFilter
			m.saveSettings()
			go func() { time.Sleep(50 * time.Millisecond); if m.program != nil { m.program.Send(refreshSettingsMsg{}) } }()
		case "thinking":
			m.showThinkingSelector()
			return
		case "theme":
			go func() {
				time.Sleep(50 * time.Millisecond)
				if m.program != nil {
					m.program.Send(components.SelectorChosenMsg{Value: "show:theme_selector"})
				}
			}()
		case "warnings":
			m.showWarningsSelector()
			return
		case "session":
			// Info only, no action
		case "name":
			// Info only, use /name command
		}
	}, nil, 64, h)
}

// updateHeaderHints sets the header's compact and expanded key hints from actual
// keybinding values so they reflect user-customized keybindings (TS pi-mono: keyHint/keyText helpers).
