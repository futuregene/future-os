// Package tui provides the interactive terminal UI for xihu using Bubble Tea.
package tui

import (

	"encoding/json"
	"fmt"
	"strings"
	"time"


	"github.com/huichen/xihu/internal/commands"
	"github.com/huichen/xihu/internal/extensions"
	"github.com/huichen/xihu/internal/session"


)

// ─── Message Types ─────────────────────────────────────────────────────────

// StreamTextMsg is a chunk of streamed text from the LLM.

func (m *AppModel) handleSlashCmd(text string) (string, bool) {
	parts := strings.Fields(text)
	if len(parts) == 0 {
		return "", true
	}
	cmd := strings.ToLower(parts[0])

	// Local TUI-only commands
	switch cmd {
	case "/help":
		return "xihu — AI coding assistant.\n\n" +
			"Quick start:\n" +
			"  /model       select model (opens selector UI)\n" +
			"  /settings    open settings menu\n" +
			"  /login       configure provider authentication\n" +
			"  /new         start a new session\n" +
			"  /hotkeys     all keyboard shortcuts\n" +
			"  /help        this help", true
	case "/hotkeys":
		m.showHelpOverlay()
		return "", true
	case "/changelog":
		m.showFullChangelog()
		return "", true
	case "/model":
		if len(parts) > 1 {
			m.switchToModel(parts[1])
			return "", true
		}
		m.showModelSelector()
		return "", true
	case "/name":
		if len(parts) > 1 {
			name := strings.Join(parts[1:], " ")
			if m.session != nil {
				m.session.SetSessionName(name)
				if err := m.sessMgr.Save(m.session); err == nil {
					m.footer.SetSession(m.session.CWD, getGitBranch(m.session.CWD), name, "", "", "")
					return "Session name set: " + name, true
				}
				return "Error saving session name", true
			}
			return "No active session", true
		}
		if m.session != nil && m.session.GetSessionName() != "" {
			return "Session name: " + m.session.GetSessionName(), true
		}
		m.chat.AppendWarning("Usage: /name <name>")
			return "", true

	case "/clone":
		if m.session != nil && m.sessMgr != nil {
			if len(m.session.Entries) == 0 {
				return "Nothing to clone yet", true
			}
			m.cloneSession()
			return "", true
		}
		return "No active session", true
	case "/sessions":
		m.showSessionSelector()
		return "", true
	case "/new":
		if m.session != nil && m.sessMgr != nil {
			m.session.ID = session.GenerateID()
			m.session.Entries = nil
			m.session.Name = ""
			m.session.CreatedAt = time.Now()
			m.session.UpdatedAt = time.Now()
			if err := m.sessMgr.Save(m.session); err == nil {
				m.footer.SetSession(m.session.CWD, getGitBranch(m.session.CWD), "", "", "", "")
				m.chat.AppendSystem("✓ New session started")
				return "", true
			}
			return "Error creating new session", true
		}
		return "No active session", true
	case "/quit":
		m.quitting = true
		return "", true
	case "/scoped-models":
		if len(parts) > 1 {
			sub := strings.ToLower(parts[1])
			switch sub {
			case "enable":
				if len(parts) > 2 {
					model := strings.Join(parts[2:], " ")
					m.scopedModels[model] = true
					return "", true
				}
				return "", true
			case "disable":
				if len(parts) > 2 {
					model := strings.Join(parts[2:], " ")
					delete(m.scopedModels, model)
					return "", true
				}
				return "", true
			case "clear":
				m.scopedModels = make(map[string]bool)
				return "", true
			case "list":
				if len(m.scopedModels) == 0 {
					return "", true
				}
				var names []string
				for name := range m.scopedModels {
					names = append(names, name)
				}
				return "", true
			default:
				return "", true
			}
		}
		// No args: show scoped models selector
		m.showScopedModelSelector()
		return "", true
	case "/tree":
		m.showSessionTree()
		return "", true
	case "/fork":
		if len(parts) > 1 && parts[1] != "" {
			m.forkFromEntry(parts[1])
			return "", true
		}
		m.showForkSelector()
		return "", true
	case "/thinking":
		m.cycleThinking()
		return "Thinking level: " + m.thinkingLevel, true
	case "/settings":
		m.showSettingsSelector()
		return "", true
	case "/theme":
		if len(parts) > 1 {
			name := strings.ToLower(parts[1])
			switch name {
			case "dark":
				m.ApplyTheme(DefaultTheme())
				return "", true
			case "light":
				m.ApplyTheme(LightTheme())
				return "", true
			default:
				return "Unknown theme: " + name + ". Available: dark, light", true
			}
		}
		m.showThemeSelector()
		return "", true
		case "/session":
			if m.session != nil {
				var sb strings.Builder

				// Header
				sb.WriteString("Session Info\n\n")

				// Name (if set)
				if name := m.session.GetSessionName(); name != "" {
					sb.WriteString("Name: " + name + "\n")
				}

				// File
				filePath := "In-memory"
				if m.sessMgr != nil {
					if fp := m.sessMgr.SessionFilePath(m.session.CWD, m.session.ID); fp != "" {
						filePath = fp
					}
				}
				sb.WriteString("File: " + filePath + "\n")

				// ID
				sb.WriteString("ID: " + m.session.ID + "\n\n")

				// Message counts
				userCount, assistantCount, toolCallCount, toolResultCount := 0, 0, 0, 0
				for _, e := range m.session.Entries {
					switch e.Role {
					case "user": userCount++
					case "assistant":
						assistantCount++
						if len(e.ToolCalls) > 0 {
							toolCallCount += len(e.ToolCalls)
						}
					case "tool": toolResultCount++
					}
				}
				sb.WriteString("Messages\n")
				sb.WriteString("User: " + commaInt(userCount) + "\n")
				sb.WriteString("Assistant: " + commaInt(assistantCount) + "\n")
				sb.WriteString("Tool Calls: " + commaInt(toolCallCount) + "\n")
				sb.WriteString("Tool Results: " + commaInt(toolResultCount) + "\n")
				sb.WriteString("Total: " + commaInt(len(m.session.Entries)) + "\n\n")

				// Token usage
				if m.lastStatus.TokensIn+m.lastStatus.TokensOut > 0 {
					sb.WriteString("Tokens\n")
					sb.WriteString("Input: " + commaInt(m.lastStatus.TokensIn) + "\n")
					sb.WriteString("Output: " + commaInt(m.lastStatus.TokensOut) + "\n")
					if m.lastStatus.TokensCacheR > 0 {
						sb.WriteString("Cache Read: " + commaInt(m.lastStatus.TokensCacheR) + "\n")
					}
					if m.lastStatus.TokensCacheW > 0 {
						sb.WriteString("Cache Write: " + commaInt(m.lastStatus.TokensCacheW) + "\n")
					}
					sb.WriteString("Total: " + commaInt(m.lastStatus.TokensIn+m.lastStatus.TokensOut+m.lastStatus.TokensCacheR+m.lastStatus.TokensCacheW) + "\n")
				}

				// Context usage
			if m.lastStatus.ContextUsed > 0 {
				sb.WriteString("\nContext\n")
				sb.WriteString(fmt.Sprintf("Used: %.1f%%\n", m.lastStatus.ContextUsed*100))
			}

			// Cost
			if m.lastStatus.TotalCost > 0 {
				sb.WriteString("\nCost\n")
				sb.WriteString(fmt.Sprintf("Total: $%.4f", m.lastStatus.TotalCost))
			}
			return sb.String(), true
			}
			return "No active session", true

	case "/copy":
		// Copy last assistant message to system clipboard
		if m.session == nil || len(m.session.Entries) == 0 {
			return "No agent messages to copy yet.", true
		}
		// Find last assistant message
		var lastText string
		for i := len(m.session.Entries) - 1; i >= 0; i-- {
			if m.session.Entries[i].Role == "assistant" {
				var contentBlocks []struct {
					Type string `json:"type"`
					Text string `json:"text"`
				}
				if err := json.Unmarshal(m.session.Entries[i].Content, &contentBlocks); err == nil {
					for _, block := range contentBlocks {
						if block.Type == "text" && block.Text != "" {
							lastText = block.Text
						}
					}
				}
				break
			}
		}
		if lastText == "" {
			return "No agent messages to copy yet.", true
		}
		if err := copyToClipboard(lastText); err != nil {
			return "Failed to copy: " + err.Error(), true
		}
		return "Copied last agent message to clipboard", true
		case "/debug":
			return m.handleDebugCommand(), true
		case "/share":
			go m.handleShare()
			return "Sharing session...", true
		case "/login":
			m.showLoginDialog()
			return "", true
		case "/logout":
			m.showLogoutDialog()
			return "", true
		}

	// Check extension-registered commands first
	if m.extRunner != nil {
		cmdName := strings.ToLower(parts[0])
		if handler := extensions.GetSlashCommand(cmdName); handler != nil {
			extCtx := extensions.NewExtensionContext(
				m.sessMgr, nil, nil, nil, m.session.CWD,
				m.extensionBridge,
			)
			result, err := handler(parts, extCtx)
			if err != nil {
				return "Extension error: " + err.Error(), true
			}
			return result, true
		}
	}

	// Forward to commands.Handle for all other commands
	ctx := &commands.Context{
		CWD:              m.session.CWD,
		SessionDir:       m.sessMgr.Dir,
		SettingsDir:      m.sessMgr.Dir, // approximate
		CurrentSessionID: m.session.ID,
		Model:            m.agent.Loop().Model,
		SystemPrompt:     m.agent.Loop().SystemPrompt,
		SessionName:      m.session.GetSessionName(),
		TotalCost:        m.lastStatus.TotalCost,
	}
	if m.lastStatus.TokensIn+m.lastStatus.TokensOut > 0 {
		ctx.TokenUsage = &commands.TokenUsage{
			Input:      m.lastStatus.TokensIn,
			Output:     m.lastStatus.TokensOut,
			CacheRead:  m.lastStatus.TokensCacheR,
			CacheWrite: m.lastStatus.TokensCacheW,
			Total:      m.lastStatus.TokensIn + m.lastStatus.TokensOut,
		}
	}
	result, err := commands.Handle(text, ctx)
	if err != nil {
		// Show user-facing errors as system messages instead of falling through to AI agent.
		// This prevents slash commands like /compact (when no messages) from being sent to the LLM.
		return err.Error(), true
	}

	// Handle sentinel return values from commands
	switch {
	case strings.HasPrefix(result, "COMPACT:"):
		m.triggerManualCompaction()
		return "", true
	case strings.HasPrefix(result, "FORK:"):
		return "", true
	case strings.HasPrefix(result, "CLONE:"):
		return "", true
	case strings.HasPrefix(result, "RESUME:"):
		// Extract session ID from "RESUME: <id>" sentinel
		sid := strings.TrimPrefix(result, "RESUME:")
		sid = strings.TrimSpace(sid)
		if sid != "" {
			m.switchToSession(sid)
		}
		return "", true
	case result == "RESUME_SELECTOR":
		m.showSessionSelector()
		return "", true
	case strings.HasPrefix(result, "IMPORT:"):
		return "", true
	case result == "NEW_SESSION":
		return "Start a new session with /new", true
	case result == "RELOAD":
		// TS pi-mono: guard checks before reload
		if m.streaming {
			m.chat.AppendWarning("Wait for the current response to finish before reloading.")
			return "", true
		}
		if m.compacting {
			m.chat.AppendWarning("Wait for compaction to finish before reloading.")
			return "", true
		}
		m.reload()
		m.chat.AppendSystem("Reloaded settings, keybindings, and theme")
		m.showPostReloadDiagnostics()
		return "", true
	case result == "QUIT":
		m.quitting = true
		return "", true
	}
	return result, true
}

// ─── Thinking Level Cycling ─────────────────────────────────────────────────

var thinkingLevels = []string{"off", "minimal", "low", "medium", "high", "xhigh"}

// supportsThinking checks whether a model ID supports extended thinking/reasoning.
