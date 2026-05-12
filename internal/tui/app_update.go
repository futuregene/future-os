// Package tui provides the interactive terminal UI for xihu using Bubble Tea.
package tui

import (

	"fmt"
	"os"
	"strings"
	"sync/atomic"
	"time"

	tea "github.com/charmbracelet/bubbletea"

	"github.com/huichen/xihu/internal/events"
	"github.com/huichen/xihu/internal/extensions"
	"github.com/huichen/xihu/internal/prompt"
	"github.com/huichen/xihu/internal/tui/components"


)

// ─── Message Types ─────────────────────────────────────────────────────────

// StreamTextMsg is a chunk of streamed text from the LLM.

func (m AppModel) Update(msg tea.Msg) (outModel tea.Model, outCmd tea.Cmd) {
	// Crash recovery: prevent any panic in TUI handlers from killing the process
	defer func() {
		if r := recover(); r != nil {
			m.chat.AppendError(fmt.Sprintf("Internal error recovered: %v", r))
			fmt.Fprintf(os.Stderr, "\n[xihu] panic recovered: %v\n", r)
			outModel = m
			outCmd = nil
		}
	}()

	switch msg := msg.(type) {
	case tea.WindowSizeMsg:
		// Clear on shrink: if terminal shrank, clear editor
		if m.clearOnShrink && (msg.Width < m.width || msg.Height < m.height) {
			if m.customEditor != nil {
				if !m.customEditor.Empty() {
					m.customEditor.Reset()
				}
			} else if !m.editorEmpty() {
				m.input.Reset()
			}
		}
		m.width = msg.Width
		m.height = msg.Height
		m.header.SetWidth(msg.Width)
		if m.customHeader != nil {
			m.customHeader.SetWidth(msg.Width)
		}
		if m.customEditor != nil {
			m.customEditor.SetWidth(msg.Width - 4)
			m.customEditor.SetHeight(msg.Height)
		} else {
			m.input.SetHeight(1)
			m.input.SetWidth(msg.Width - 4)
		}
		editorHeight := m.editorHeight()
		if m.customFooter != nil {
			m.customFooter.SetWidth(msg.Width)
		}
		footerHeight := m.footerHeight()
		headerHeight := 0 // header is now a no-op — welcome text lives in chat viewport
		if m.customHeader != nil {
			headerHeight = m.customHeader.Height()
		}
		m.chat.SetSize(msg.Width, msg.Height-editorHeight-footerHeight-headerHeight)
		m.footer.SetWidth(msg.Width)
		m.overlay.SetTermSize(msg.Width, msg.Height)
		return m, nil

	case WelcomeMsg:
		m.showWelcome(msg)
		return m, nil

	case tea.MouseMsg:
		// Route mouse events to chat viewport for native scroll handling
		_, _ = m.chat.Update(msg)
		return m, nil

	case components.CountdownTickMsg:
		if m.overlay.Active() {
			return m, m.overlay.Update(msg)
		}
		return m, nil

	case tea.KeyMsg:
		// Bracketed paste detection (TS pi-mono: handlePaste with CSI 200~ / 201~)
		// Bubble Tea v1.3.10+ already decodes paste events and sets Key.Paste = true.
		// Route paste content through the editor's Paste method for large-paste markers.
		if k := tea.Key(msg); k.Paste && len(k.Runes) > 0 {
			if m.customEditor != nil {
				m.customEditor.SetValue(string(k.Runes))
			} else {
				m.input.Paste(string(k.Runes))
			}
			return m, nil
		}

		// Dispatch to extension terminal input handlers first
		if m.inputRegistry != nil {
			consumed, _ := m.inputRegistry.dispatch(msg.String())
			if consumed {
				return m, nil
			}
		}

		// Keybindings global action dispatch (pi-mono aligned)
		if m.keybindings != nil {
			ks := msg.String()
			switch {
			case m.keybindings.Matches(ks, GlobalInterrupt):
				m.quitting = true
				return m, tea.Quit
			case m.keybindings.Matches(ks, GlobalClear):
				if !m.editorEmpty() {
					m.input.Reset()
					return m, nil
				}
				if !m.streaming && !m.compacting {
					now := time.Now()
					if now.Sub(m.lastCtrlCTime) < 500*time.Millisecond {
						m.lastCtrlCTime = time.Time{}
						m.quitting = true
						return m, tea.Quit
					}
					m.lastCtrlCTime = now
					return m, nil
				}
			case m.keybindings.Matches(ks, GlobalExit):
				if !m.streaming && !m.compacting && m.editorEmpty() {
					m.quitting = true
					return m, tea.Quit
				}
				if !m.editorEmpty() {
					_, cmd := m.input.Update(msg)
					return m, cmd
				}
		case m.keybindings.Matches(ks, GlobalToggleHeader):
			m.welcomeExpanded = !m.welcomeExpanded
			// Rebuild welcome message with new expansion state
			m.rebuildWelcome()
			// Recalculate chat size
			editorHeight := m.editorHeight()
			footerHeight := m.footerHeight()
			headerHeight := 0 // header is now a no-op — welcome text lives in chat viewport
			if m.customHeader != nil {
				headerHeight = m.customHeader.Height()
			}
			m.chat.SetSize(m.width, m.height-editorHeight-footerHeight-headerHeight)
			return m, nil
		case m.keybindings.Matches(ks, GlobalToggleTools):
			m.chat.ToggleAllTools()
			return m, nil
			case m.keybindings.Matches(ks, GlobalToggleThinking):
				m.chat.HideAllThinking = !m.chat.HideAllThinking
				visible := "hidden"
				if !m.chat.HideAllThinking {
					visible = "visible"
				}
				m.chat.AppendSystem("Thinking blocks: " + visible)
				return m, nil
			case m.keybindings.Matches(ks, GlobalModelSelector):
				m.showModelSelector()
				return m, nil
			case m.keybindings.Matches(ks, GlobalCycleModelFwd):
				m.cycleModelForward()
				return m, nil
			case m.keybindings.Matches(ks, GlobalCycleModelBack):
				m.cycleModelBackward()
				return m, nil
			case m.keybindings.Matches(ks, GlobalCycleThinking):
				m.cycleThinking()
				return m, nil
			case m.keybindings.Matches(ks, GlobalExternalEditor):
				m.openExternalEditor()
				return m, nil
			case m.keybindings.Matches(ks, EditorYank):
				_, cmd := m.input.Update(msg)
				return m, cmd
			case m.keybindings.Matches(ks, EditorYankPop):
				_, cmd := m.input.Update(msg)
				return m, cmd
			case m.keybindings.Matches(ks, EditorUndo):
				_, cmd := m.input.Update(msg)
				return m, cmd
			}
		}

		switch msg.String() {
		case "ctrl+c":
			// TS pi-mono: double-press guard — second Ctrl+C within 500ms exits.
			// Every Ctrl+C clears the editor and records the timestamp.
			now := time.Now()
			if now.Sub(m.lastCtrlCTime) < 500*time.Millisecond {
				m.lastCtrlCTime = time.Time{}
				m.quitting = true
				return m, tea.Quit
			}
			m.lastCtrlCTime = now
			if !m.editorEmpty() {
				m.input.Reset()
			}
			return m, nil
		case "ctrl+z":
			// TS pi-mono: Suspend to background
			return m, tea.Suspend
		case "ctrl+d":
			if !m.streaming && !m.compacting && m.editorEmpty() {
				m.quitting = true
				return m, tea.Quit
			}
			// Forward to editor for delete-char-forward when editor has content
			if !m.editorEmpty() {
				_, cmd := m.input.Update(msg)
				return m, cmd
			}
		case "ctrl+h":
		// Toggle header expanded/collapsed (TS pi-mono: ExpandableText header)
		if m.customHeader != nil {
			// Custom header — just re-layout
		}
		headerHeight := 3 // spacer + empty + spacer
		if m.customHeader != nil {
			headerHeight = m.customHeader.Height()
		}
			editorHeight := m.editorHeight()
			footerHeight := m.footerHeight()
			m.chat.SetSize(m.width, m.height-editorHeight-footerHeight-headerHeight)
			return m, nil
		case "ctrl+o":
			// TS pi-mono: Toggle ALL tool outputs expand/collapse globally
			m.chat.ToggleAllTools()
			return m, nil
		case "ctrl+l":
			// TS pi-mono: Open model selector; close any existing overlay first
			m.overlay.HideAll()
			m.showModelSelector()
			return m, nil
		case "ctrl+g":
			// TS pi-mono: Open external editor ($EDITOR)
			text := m.openExternalEditor()
			if text != "" && m.program != nil {
				m.program.Send(components.SubmitMsg(text))
			}
			return m, nil
		case "esc":
			// TS pi-mono: Escape during streaming = abort current LLM call
			if m.bashCancelCh != nil {
				close(m.bashCancelCh)
				m.bashCancelCh = nil
				return m, nil
			}
			if m.compacting {
				// Signal compaction cancellation via event bus
				if m.agent != nil && m.agent.Loop().EventBus != nil {
					m.agent.Loop().EventBus.Emit(events.CompactionEnd(0, "", true, "manual"))
				}
				return m, nil
			}
			if m.streaming {
				m.agent.Abort()
				// Restore queued messages to editor (TS pi-mono: prepend to existing content)
				msgs := m.agent.Loop().DrainQueues()
				if len(msgs) > 0 {
					queued := strings.Join(msgs, "\n\n")
					current := m.input.Value()
					if current != "" {
						m.input.SetValue(queued + "\n\n" + current)
					} else {
						m.input.SetValue(queued)
					}
					// Silent abort — agent response indicates cancellation via stopReason
				}
				return m, nil
			}
			// Double-escape with empty editor: trigger tree or fork (TS pi-mono)
			if m.editorEmpty() && m.doubleEscapeAction != "none" {
				now := time.Now()
				if now.Sub(m.lastEscapeTime) < 500*time.Millisecond {
					m.lastEscapeTime = time.Time{}
					if m.doubleEscapeAction == "tree" {
						m.showSessionTree()
					} else if m.doubleEscapeAction == "fork" {
						m.showForkSelector()
					}
					return m, nil
				}
				m.lastEscapeTime = now
			}
		case "shift+tab":
			// Cycle thinking level: off → low → medium → high → xhigh → off
			m.cycleThinking()
			return m, nil
		case "ctrl+t":
			// Toggle thinking visibility (TS pi-mono: hideThinkingBlock)
			m.chat.HideAllThinking = !m.chat.HideAllThinking
			visible := "hidden"
			if !m.chat.HideAllThinking {
				visible = "visible"
			}
			m.chat.AppendSystem("Thinking blocks: " + visible)
			return m, nil
		case "ctrl+p":
			// TS pi-mono: Cycle model forward
			if len(m.availableModels) > 0 {
				m.cycleModelForward()
			}
			return m, nil
		case "ctrl+shift+p":
			// TS pi-mono: Cycle model backward
			if len(m.availableModels) > 0 {
				m.cycleModelBackward()
			}
			return m, nil
		case "alt+up":
			// TS pi-mono: Dequeue — prepend queued messages to existing editor content
			msgs := m.agent.Loop().DrainQueues()
			if len(msgs) > 0 {
				queued := strings.Join(msgs, "\n\n")
				current := m.input.Value()
				if current != "" {
					m.input.SetValue(queued + "\n\n" + current)
				} else {
					m.input.SetValue(queued)
				}
				noun := "message"
				if len(msgs) > 1 {
					noun = "messages"
				}
				m.chat.AppendSystem(fmt.Sprintf("Restored %d queued %s to editor", len(msgs), noun))
			} else {
				m.chat.AppendSystem("No queued messages to restore")
			}
			m.pendingSteeringMsgs = nil
			m.pendingFollowUpMsgs = nil
			return m, nil
		}

		// Route to overlay if active and capturing (nonCapturing overlays let keys through)
		if m.overlay.Active() && !m.overlay.NonCapturing() {
			cmd := m.overlay.Update(msg)
			return m, cmd
		}
		// Non-capturing overlay: only Esc closes it, all other keys pass through
		if m.overlay.Active() && m.overlay.NonCapturing() {
			if msg.String() == "esc" {
				m.overlay.Hide()
				return m, nil
			}
		}

		// Handle autocomplete navigation (arrow keys when autocomplete is active)
		if m.autocomplete.Active() {
			switch msg.String() {
			case "up":
				m.autocomplete.SelectPrev()
				return m, nil
			case "down":
				m.autocomplete.SelectNext()
				return m, nil
			case "tab":
				// TS pi-mono: Tab cycles to next autocomplete candidate
				m.autocomplete.SelectNext()
				if selected := m.autocomplete.Selected(); selected != "" {
					m.input.SetValue(selected)
				}
				return m, nil
			case "shift+tab":
				// TS pi-mono: Shift+Tab cycles to previous
				m.autocomplete.SelectPrev()
				if selected := m.autocomplete.Selected(); selected != "" {
					m.input.SetValue(selected)
				}
				return m, nil
			case "enter":
				selected := m.autocomplete.Selected()
				if selected != "" {
					m.input.SetValue(selected)
					m.autocomplete.Hide()
					m.input.ExitSlashMode()
				}
				return m, nil
			}
		}

		// Route scroll/chat keys to chat viewport (handles pgup/pgdown/ctrl+u/ctrl+d/mouse wheel natively via bubbles/viewport)
		switch msg.String() {
		case "pgup", "pgdown", "ctrl+u", "ctrl+d", "home", "end":
			_, cmd := m.chat.Update(msg)
			return m, cmd
		case "G":
			// Jump to bottom (follow mode)
			m.chat.ScrollToBottom()
			return m, nil
		case "g":
			// gg: jump to top on double-g within 500ms
			// Only intercept when editor is empty (otherwise user is typing "g" as text)
			if m.editorEmpty() {
				now := time.Now()
				if now.Sub(m.lastGTime) < 500*time.Millisecond {
					m.lastGTime = time.Time{}
					m.chat.ScrollToTop()
					return m, nil
				}
				m.lastGTime = now
				return m, nil
			}
			// Forward to editor
		case "ctrl+v":
			// Paste from system clipboard (TS pi-mono: clipboard paste with markers for large text)
			if text, err := pasteFromClipboard(); err == nil && text != "" {
				if marker := m.input.StorePaste(text); marker != "" {
					m.input.SetValue(m.input.Value() + marker)
				} else {
					m.input.SetValue(m.input.Value() + text)
				}
			} else if err != nil {
				m.chat.AppendSystem("Clipboard paste failed: " + err.Error())
			}
			return m, nil
		}

	case components.SubmitMsg:
		text := m.input.ExpandPastes(string(msg))
		if m.streaming {
			// TS-style steer: inject message without aborting current stream
			m.input.RecordSubmission(text)
			m.pendingSteeringMsgs = append(m.pendingSteeringMsgs, text)
			m.agent.Steer(text)
			return m, nil
		}
		if m.compacting {
			m.input.RecordSubmission(text)
			m.compactionQueue = append(m.compactionQueue, text)
			m.chat.AppendSystem("Queued message for after compaction")
			return m, nil
		}
		{
			atomic.AddInt32(&m.streamID, 1)
			if strings.HasPrefix(text, "!") {
				// TS pi-mono: bash already-running guard
				if m.bashCancelCh != nil {
					m.chat.AppendWarning("A bash command is already running. Press Esc to cancel it first.")
					m.input.SetValue(text)
					return m, nil
				}
			}
			if strings.HasPrefix(text, "!!") {
				cmd := strings.TrimPrefix(text, "!!")
				cmd = strings.TrimSpace(cmd)
				if cmd != "" {
					go m.runBashDirect(cmd, true)
				}
			} else if strings.HasPrefix(text, "!") {
				cmd := strings.TrimPrefix(text, "!")
				cmd = strings.TrimSpace(cmd)
				if cmd != "" {
					go m.runBashDirect(cmd, false)
				}
			} else if strings.HasPrefix(text, "/skill:") && m.skillCommands {
				atomic.AddInt32(&m.streamID, 1)
				skillName := strings.TrimPrefix(text, "/skill:")
				found := false
				for _, s := range m.Skills {
					if s.Name == skillName {
						content, err := os.ReadFile(s.Path)
						if err != nil {
							m.chat.AppendSystem("Skill error: " + err.Error())
							found = true
							break
						}
						m.chat.AppendCustomMessage("skill", fmt.Sprintf("Invoking skill: %s\n%s", s.Name, s.Description))
						go m.runAgent("Follow the skill instructions:\n\n" + string(content), m.streamID)
						found = true
						break
					}
				}
				if !found {
					m.chat.AppendSystem("Skill not found: " + skillName)
				}
			} else if strings.HasPrefix(text, "/") && !strings.HasPrefix(text, "//") {
				// Check for prompt template: /:name [args...]
				cmdName, cmdArgs := splitSlashCommand(text)
				if tmpl := m.findTemplate(cmdName); tmpl != nil {
					atomic.AddInt32(&m.streamID, 1)
					expanded := prompt.ExpandTemplate(*tmpl, cmdArgs...)
					go m.runAgent(expanded, m.streamID)
				} else {
					result, handled := m.handleSlashCmd(text)
					if !handled {
						// TS pi-mono: unknown slash commands fall through to LLM as normal prompts
						m.chat.AppendUserMessage(text)
						m.input.RecordSubmission(text)
						go m.runAgent(text, m.streamID)
					} else if result != "" {
						m.chat.AppendSystem(result)
					}
				}
			} else {
				m.chat.AppendUserMessage(text)
				m.input.RecordSubmission(text)
				go m.runAgent(text, m.streamID)
			}
		}
		return m, nil

	case components.FollowUpMsg:
		// TS pi-mono: Alt+Enter queues message for after agent finishes
		text := m.input.ExpandPastes(string(msg))
		m.pendingFollowUpMsgs = append(m.pendingFollowUpMsgs, text)
		m.agent.FollowUp(text) // Uses FollowUpQueue → processed after agent finishes
		return m, nil

	case StreamTextMsg:
		m.chat.AppendText(string(msg))
		m.footer.SetWorkingMessage("Thinking...")
		return m, nil

	case ThinkingMsg:
		m.chat.AppendThinking(string(msg))
		m.footer.SetWorkingMessage("Thinking...")
		return m, nil

	case ToolCallMsg:
		if msg.Name == "bash" {
			// Replace pending tool_call entry with bordered bash display
			m.chat.RemovePendingToolCall(msg.ID)
			cmd := extractBashCommand(msg.Arguments)
			m.chat.AddBashExecution(cmd, false)
		} else {
			// Finalize pending tool_call entry's args in-place (avoids duplicate)
			m.chat.CompleteToolCall(msg.ID, msg.Arguments)
		}
		return m, nil

	case ToolCallStartMsg:
		m.chat.AddToolCall(msg.ID, msg.Name, "")
		// Working message set in ToolRunningMsg when execution actually starts (TS pi-mono timing)
		return m, nil

	case ToolCallDeltaMsg:
		m.chat.AppendToolCallDelta(msg.Text)
		return m, nil

	case ToolRunningMsg:
		m.chat.MarkToolRunning(msg.ID)
		m.footer.SetWorkingMessage("Running " + msg.Name + "...")
		return m, nil

	case ToolResultMsg:
		durStr := formatDuration(msg.DurationMs)
		if msg.ID == "bash" {
			if msg.Error != "" {
				m.chat.CompleteBash(1, false); m.chat.SetLastBashDuration(durStr)
			} else {
				m.chat.CompleteBash(0, false); m.chat.SetLastBashDuration(durStr)
			}
			_ = durStr
		} else {
			if msg.Error != "" {
				m.chat.UpdateToolResult(msg.ID, msg.Error, true)
			} else {
				m.chat.UpdateToolResult(msg.ID, msg.Output, false)
			}
			m.chat.SetToolDuration(msg.ID, durStr)
		}
		return m, nil

	case StopReasonMsg:
		m.chat.MarkLastStopReason(msg.Reason)
		return m, nil

	case AgentDoneMsg:
		m.streaming = false
		m.stopProgress()
		m.footer.SetWorkingMessage("")
		// Scroll to show latest response, then stop auto-scrolling (TS pi-mono behavior)
		m.chat.ScrollToBottom()
		m.chat.DisableAutoScroll()
		// Auto-save session after each agent turn (TS pi-mono: saves on message_end)
		if m.sessMgr != nil && m.session != nil {
			m.sessMgr.Save(m.session)
		}
		return m, nil

	case AgentErrorMsg:
		m.streaming = false
		m.stopProgress()
		// Save even on error to preserve conversation up to failure
		if m.sessMgr != nil && m.session != nil {
			m.sessMgr.Save(m.session)
		}
		m.chat.AppendError(msg.Error.Error())
		return m, nil

	case BashExecResultMsg:
		m.chat.AppendBashOutput(msg.Output)
		m.chat.CompleteBash(msg.ExitCode, msg.Cancelled)
		if msg.Truncated && msg.FullOutputPath != "" {
			m.chat.AppendWarning("Output truncated. Full output: " + msg.FullOutputPath)
		}
		// Scroll to show completed output, then stop auto-scrolling (TS pi-mono behavior)
		m.chat.ScrollToBottom()
		m.chat.DisableAutoScroll()
		return m, nil

	case ShareResultMsg:
		if msg.Error != "" {
			m.chat.AppendError(msg.Error)
		} else if msg.GistURL != "" {
			if msg.PreviewURL != "" {
				m.chat.AppendSystem("Share URL: " + msg.PreviewURL + "\nGist: " + msg.GistURL)
			} else {
				m.chat.AppendSystem("Share URL: " + msg.GistURL + "\nGist: " + msg.GistURL)
			}
		}
		return m, nil

	case refreshScopedSelectorMsg:
		m.overlay.Hide()
		m.showScopedModelSelector()
		return m, nil

	case refreshSettingsMsg:
		m.overlay.Hide()
		m.showSettingsSelector()
		if m.showHardwareCursor {
			return m, tea.ShowCursor
		}
		return m, tea.HideCursor

	case refreshWarningsMsg:
		m.overlay.Hide()
		m.showWarningsSelector()
		return m, nil

	case refreshModelSelectorMsg:
		m.overlay.Hide()
		m.showModelSelector()
		return m, nil

	case components.SelectorChosenMsg:
		if strings.HasPrefix(msg.Value, "show:") {
			sub := strings.TrimPrefix(msg.Value, "show:")
			switch sub {
			case "theme_selector":
				m.showThemeSelector()
			case "model_selector":
				m.showModelSelector()
			}
			return m, nil
		}
		if strings.HasPrefix(msg.Value, "session:") {
			sid := strings.TrimPrefix(msg.Value, "session:")
			m.switchToSession(sid)
		} else if msg.Value != "" {
			m.switchToModel(msg.Value)
		}
		return m, nil

	case extensionSelectMsg:
		items := make([]components.SelectorItem, len(msg.options))
		for i, opt := range msg.options {
			items[i] = components.SelectorItem{Label: opt, Value: opt}
		}
		w := m.width * 60 / 100
		h := m.height * 60 / 100
		m.overlay.ShowSelector(msg.title, items, func(value string) {
			msg.respCh <- extensionUIResponse{value: value}
		}, w, h)
		m.overlay.OnDismiss(func() {
			msg.respCh <- extensionUIResponse{err: fmt.Errorf("cancelled")}
		})
		if msg.timeout > 0 {
			m.overlay.StartCountdown(int(msg.timeout.Seconds()))
			return m, components.CountdownTick()
		}
		return m, nil

	case extensionInputMsg:
		w := m.width * 60 / 100
		h := 10
		m.overlay.ShowInput(msg.title, func(value string) {
			msg.respCh <- extensionUIResponse{value: value}
		}, func() {
			msg.respCh <- extensionUIResponse{err: fmt.Errorf("cancelled")}
		}, w, h)
		if msg.timeout > 0 {
			m.overlay.StartCountdown(int(msg.timeout.Seconds()))
			return m, components.CountdownTick()
		}
		return m, nil

	case extensionEditorMsg:
		w := m.width * 70 / 100
		h := m.height * 70 / 100
		m.overlay.ShowEditor(msg.title, msg.prefill, func(value string) {
			msg.respCh <- extensionUIResponse{value: value}
		}, func() {
			msg.respCh <- extensionUIResponse{err: fmt.Errorf("cancelled")}
		}, w, h)
		return m, nil

	case extensionCustomMsg:
		w := m.width * 60 / 100
		h := m.height * 60 / 100
		overlayButtons := make([]components.CustomButton, len(msg.buttons))
		for i, b := range msg.buttons {
			overlayButtons[i] = components.CustomButton{Label: b.Label, Value: b.Value}
		}
		m.overlay.ShowCustom(msg.title, msg.content, overlayButtons, func(value string) {
			msg.respCh <- extensionUIResponse{value: value}
		}, func() {
			msg.respCh <- extensionUIResponse{err: fmt.Errorf("cancelled")}
		}, w, h)
		m.overlay.OnDismiss(func() {
			msg.respCh <- extensionUIResponse{err: fmt.Errorf("cancelled")}
		})
		if msg.timeout > 0 {
			m.overlay.StartCountdown(int(msg.timeout.Seconds()))
			return m, components.CountdownTick()
		}
		return m, nil

	case extensionStatusMsg:
		if m.extensionStatuses == nil {
			m.extensionStatuses = make(map[string]string)
		}
		if msg.text == "" {
			delete(m.extensionStatuses, msg.key)
		} else {
			m.extensionStatuses[msg.key] = msg.text
		}
		m.footer.SetExtensionStatuses(m.extensionStatuses)
		return m, nil

	case extensionSetTitleMsg:
		fmt.Printf("\033]0;%s\007", msg.title)
		return m, nil

	case extensionHiddenThinkingLabelMsg:
		if msg.label == "" {
			m.chat.HiddenThinkingLabel = "Thinking..."
		} else {
			m.chat.HiddenThinkingLabel = msg.label
		}
		return m, nil

	case extensionWorkingMessageMsg:
		if msg.message == "" {
			m.workingMessage = "Working..."
		} else {
			m.workingMessage = msg.message
		}
		m.footer.SetWorkingMessage(m.workingMessage)
		return m, nil

	case extensionWorkingVisibleMsg:
		m.workingVisible = msg.visible
		m.footer.SetWorkingVisible(msg.visible)
		return m, nil

	case extensionWorkingIndicatorMsg:
		m.workingFrames = msg.frames
		m.workingIntervalMs = msg.intervalMs
		m.footer.SetWorkingIndicator(msg.frames, msg.intervalMs)
		return m, nil

	case extensionEditorTextMsg:
		if msg.isSet {
			if m.customEditor != nil {
				m.customEditor.SetValue(msg.text)
			} else {
				m.input.SetValue(msg.text)
			}
		} else {
			if m.customEditor != nil {
				msg.respCh <- m.customEditor.Value()
			} else {
				msg.respCh <- m.input.Value()
			}
		}
		return m, nil

	case extensionPasteMsg:
		if m.customEditor != nil {
			// Custom editors don't have Paste, fall back to SetValue
			m.customEditor.SetValue(msg.text)
		} else {
			m.input.Paste(msg.text)
		}
		return m, nil

	case extensionWidgetMsg:
		if msg.content == "" {
			if msg.placement == "belowEditor" {
				delete(m.widgetsBelow, msg.key)
			} else {
				delete(m.widgetsAbove, msg.key)
			}
		} else {
			if msg.placement == "belowEditor" {
				if m.widgetsBelow == nil {
					m.widgetsBelow = make(map[string]string)
				}
				m.widgetsBelow[msg.key] = msg.content
			} else {
				if m.widgetsAbove == nil {
					m.widgetsAbove = make(map[string]string)
				}
				m.widgetsAbove[msg.key] = msg.content
			}
		}
		return m, nil

	case extensionGetAllThemesMsg:
		themes := []extensions.ThemeInfo{
			{Name: "dark", Path: ""},
			{Name: "light", Path: ""},
		}
		customPaths, _ := DiscoverThemes("")
		for _, p := range customPaths {
			t, err := LoadTheme(p)
			if err != nil || t.Name == "" {
				continue
			}
			if t.Name == "dark" || t.Name == "light" {
				continue
			}
			themes = append(themes, extensions.ThemeInfo{Name: t.Name, Path: p})
		}
		msg.respCh <- themes
		return m, nil

	case extensionGetCurrentThemeNameMsg:
		if m.theme != nil {
			msg.respCh <- m.theme.Name
		} else {
			msg.respCh <- "dark"
		}
		return m, nil

	case extensionSetThemeMsg:
		switch msg.name {
		case "dark":
			m.ApplyTheme(DefaultTheme())
			msg.respCh <- nil
		case "light":
			m.ApplyTheme(LightTheme())
			msg.respCh <- nil
		default:
			// Search custom themes
			customPaths, _ := DiscoverThemes("")
			found := false
			for _, p := range customPaths {
				t, err := LoadTheme(p)
				if err != nil || t.Name != msg.name {
					continue
				}
				m.ApplyTheme(t)
				msg.respCh <- nil
				found = true
				break
			}
			if !found {
				msg.respCh <- fmt.Errorf("theme %q not found", msg.name)
			}
		}
		return m, nil

	case extensionGetToolsExpandedMsg:
		msg.respCh <- m.chat.AllToolsExpanded
		return m, nil

	case extensionSetToolsExpandedMsg:
		if m.chat.AllToolsExpanded != msg.expanded {
			m.chat.ToggleAllTools()
		}
		return m, nil

	// ── Extension Component Replacement ────────────────────────────────────

	case extSetFooterMsg:
		if msg.factory != nil {
			m.customFooter = msg.factory()
		} else {
			m.customFooter = nil
		}
		return m, nil

	case extSetHeaderMsg:
		if msg.factory != nil {
			m.customHeader = msg.factory()
		} else {
			m.customHeader = nil
		}
		return m, nil

	case extSetEditorMsg:
		if msg.factory != nil {
			m.customEditor = msg.factory()
			m.customEditorNeedsInit = true
		} else {
			if m.customEditor != nil {
				m.customEditor.Blur()
			}
			m.customEditor = nil
			m.customEditorNeedsInit = false
		}
		return m, nil

	case extGetEditorMsg:
		// Return nil — extensions store their own factory reference.
		msg.respCh <- nil
		return m, nil

	case appendSystemMsg:
		m.chat.AppendSystem(string(msg))
		return m, nil

	case appendErrorMsg:
		m.chat.AppendError(string(msg))
		return m, nil

	case appendWarningMsg:
		m.chat.AppendWarning(string(msg))
		return m, nil

	case StatusMsg:
		m.lastStatus = msg
		m.footer.Update(
			msg.TokensIn, msg.TokensOut,
			msg.TokensCacheR, msg.TokensCacheW,
			msg.TotalCost, msg.ContextUsed,
			msg.Streaming,
		)
		return m, nil

	case TickMsg:
		m.spinnerFrame = (m.spinnerFrame + 1) % 10
		m.footer.SetSpinnerFrame(m.spinnerFrame)
		return m, nil

	case BranchTickMsg:
		// Check if git branch changed and update footer + terminal title
		newBranch := getGitBranch(m.session.CWD)
		if newBranch != m.gitBranch {
			m.gitBranch = newBranch
			m.footer.SetGitBranch(newBranch)
			updateTerminalTitle(m.session.GetSessionName(), m.session.CWD)
		}
		return m, tea.Tick(3*time.Second, func(t time.Time) tea.Msg {
			return BranchTickMsg(t)
		})

	case RetryTickMsg:
		if m.retryTicking && m.retryDelaySec > 0 {
			m.retryDelaySec--
			if m.retryDelaySec > 0 {
				msg := fmt.Sprintf("Retrying (%d/%d) in %ds... (Esc to cancel)", m.retryAttempt, m.retryMaxAttempts, m.retryDelaySec)
				m.chat.ReplaceLastSystem(msg)
				return m, tea.Tick(1*time.Second, func(t time.Time) tea.Msg {
					return RetryTickMsg(t)
				})
			}
		}
		return m, nil

	case ResizeMsg:
		m.width = msg.Width
		m.height = msg.Height
		return m, nil
	}

	// Route to input editor (custom or built-in)
	var cmd tea.Cmd
	if m.customEditor != nil {
		// Init custom editor on first Update after instantiation
		if m.customEditorNeedsInit {
			m.customEditorNeedsInit = false
			cmd = m.customEditor.Init()
		}
		var next tea.Model
		next, cmd = m.customEditor.Update(msg)
		if ec, ok := next.(EditorComponent); ok {
			m.customEditor = ec
		}
	} else {
		*m.input, cmd = m.input.Update(msg)
	}

	// After editor update, check for slash mode and update autocomplete
	if m.input.IsSlashMode() {
		prefix := m.input.GetSlashPrefix()
		// Detect argument mode: if prefix contains a space, show argument completions
		if idx := strings.Index(prefix, " "); idx >= 0 {
			cmdName := strings.TrimPrefix(strings.ToLower(prefix[:idx]), "/")
			argPrefix := strings.TrimLeft(prefix[idx+1:], " ")
			argCandidates := m.getArgCompletions(cmdName, argPrefix)
			if len(argCandidates) > 0 {
				names := make([]string, len(argCandidates))
				descs := make(map[string]string)
				for i, c := range argCandidates {
					names[i] = c.Name
					descs[c.Name] = c.Description
				}
				m.input.SetSlashCandidates(names)
				// Show argument completions as non-command items in autocomplete
				m.autocomplete.Show(names, descs, argPrefix)
				return m, cmd
			}
			m.autocomplete.Hide()
			return m, cmd
		}

		candidates := m.filterSlashCandidates(prefix)
		// Merge extension-registered commands
		if m.extRunner != nil {
			for cmdName := range extensions.GetAllSlashCommands() {
				if prefix == "" || strings.HasPrefix(cmdName, prefix) {
					candidates = append(candidates, components.SlashCommand{
						Name:        cmdName,
						Description: "(extension)",
					})
				}
			}
		}
		// Merge skill commands (TS pi-mono: /skill:name for each loaded skill)
		if m.skillCommands && len(m.Skills) > 0 {
			for _, sk := range m.Skills {
				cmdName := "/skill:" + sk.Name
				if prefix == "" || strings.HasPrefix(cmdName, prefix) {
					desc := sk.Description
					if desc == "" {
						desc = fmt.Sprintf("Invoke skill %s", sk.Name)
					}
					candidates = append(candidates, components.SlashCommand{
						Name:        cmdName,
						Description: desc,
					})
				}
			}
		}
		// Merge extension autocomplete providers
		for _, provider := range extensions.GetAllAutocompleteProviders() {
			for _, candidate := range provider(prefix) {
				if prefix == "" || strings.HasPrefix(strings.ToLower(candidate), strings.ToLower(prefix)) {
					candidates = append(candidates, components.SlashCommand{
						Name:        candidate,
						Description: "(ext)",
					})
				}
			}
		}
		names := make([]string, len(candidates))
		for i, c := range candidates {
			names[i] = c.Name
		}
		m.input.SetSlashCandidates(names)
		// Update autocomplete overlay with formatted candidates
		m.updateAutocomplete(candidates, prefix)
	} else if m.input.IsFileMode() {
		prefix := m.input.GetFilePrefix()
		rawPrefix := m.input.GetFilePrefixRaw()
		// Detect quoted path mode: @"path with spaces"
		quoted := strings.HasPrefix(rawPrefix, "\"")
		// Find files matching the prefix
		matches := components.FindFiles(prefix)
		if len(matches) == 0 {
			// If prefix is empty, list files in CWD
			if prefix == "" {
				matches = components.FindFiles(".")
			}
		}
		if len(matches) > 0 {
			// Preserve quoting in displayed completions (TS pi-mono: quoted @ paths)
			names := matches
			if quoted {
				for i, m := range matches {
					names[i] = "\"" + m + "\""
				}
			}
			descs := make(map[string]string)
			for i := range matches {
				descs[names[i]] = "" // file paths don't need descriptions
			}
			if len(names) > 20 {
				names = names[:20]
			}
			displayPrefix := prefix
			if quoted {
				displayPrefix = "\"" + prefix
			}
			m.autocomplete.Show(names, descs, displayPrefix)
		} else {
			m.autocomplete.Hide()
		}
	} else if m.input.IsSymbolMode() {
		prefix := m.input.GetSymbolPrefix()
		// Collect symbols from recent session entries (TS pi-mono: # symbol autocomplete)
		symbols := m.collectSymbols(prefix)
		if len(symbols) > 0 {
			names := make([]string, 0, len(symbols))
			descs := make(map[string]string)
			for _, s := range symbols {
				names = append(names, s.Name)
				descs[s.Name] = s.Description
			}
			m.autocomplete.Show(names, descs, prefix)
		} else {
			m.autocomplete.Hide()
		}
	} else {
		m.autocomplete.Hide()
	}

	// Return editor command batched with spinner tick if streaming or compacting
	var cmds []tea.Cmd
	if cmd != nil {
		cmds = append(cmds, cmd)
	}
	if m.streaming || m.compacting {
		cmds = append(cmds, tea.Tick(time.Millisecond*100, func(t time.Time) tea.Msg {
			return TickMsg(t)
		}))
	}
	return m, tea.Batch(cmds...)
}

// View renders the entire UI.
