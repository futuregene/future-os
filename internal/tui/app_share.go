// Package tui provides the interactive terminal UI for xihu using Bubble Tea.
package tui

import (

	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"strings"
	"time"


	"github.com/huichen/xihu/internal/prompt"


)

// ─── Message Types ─────────────────────────────────────────────────────────

// StreamTextMsg is a chunk of streamed text from the LLM.

func (m *AppModel) handleShare() {
	// Check if gh CLI is available
	if _, err := exec.LookPath("gh"); err != nil {
		if m.program != nil {
			m.program.Send(ShareResultMsg{Error: "GitHub CLI (gh) is not installed. Install it from https://cli.github.com/"})
		}
		return
	}

	// Check if gh CLI is logged in (TS pi-mono)
	if err := exec.Command("gh", "auth", "status").Run(); err != nil {
		if m.program != nil {
			m.program.Send(ShareResultMsg{Error: "GitHub CLI is not logged in. Run 'gh auth login' first."})
		}
		return
	}

	// Build HTML export
	html := m.buildSessionHTML()
	if html == "" {
		if m.program != nil {
			m.program.Send(ShareResultMsg{Error: "No session content to share"})
		}
		return
	}

	// Write to temp file
	tmpFile, err := os.CreateTemp("", "xihu-session-*.html")
	if err != nil {
		if m.program != nil {
			m.program.Send(ShareResultMsg{Error: "Failed to export session: " + err.Error()})
		}
		return
	}
	tmpPath := tmpFile.Name()
	if _, err := tmpFile.WriteString(html); err != nil {
		tmpFile.Close()
		os.Remove(tmpPath)
		if m.program != nil {
			m.program.Send(ShareResultMsg{Error: "Failed to export session: " + err.Error()})
		}
		return
	}
	tmpFile.Close()
	defer os.Remove(tmpPath)

	// Create secret gist
	cmd := exec.Command("gh", "gist", "create", "--public=false", tmpPath)
	output, err := cmd.Output()
	if err != nil {
		var errMsg string
		if exitErr, ok := err.(*exec.ExitError); ok {
			errMsg = "Failed to create gist: " + string(exitErr.Stderr)
		} else {
			errMsg = "Failed to create gist: " + err.Error()
		}
		if m.program != nil {
			m.program.Send(ShareResultMsg{Error: errMsg})
		}
		return
	}

	// Parse gist URL from output (TS pi-mono: extract gist ID for preview URL)
	gistURL := strings.TrimSpace(string(output))
	if gistURL == "" {
		if m.program != nil {
			m.program.Send(ShareResultMsg{Error: "Failed to parse gist ID from gh output"})
		}
		return
	}

	// Extract gist ID and build preview URL (TS pi-mono: getShareViewerUrl)
	previewURL := ""
	if idx := strings.LastIndex(gistURL, "/"); idx >= 0 {
		gistID := gistURL[idx+1:]
		previewURL = "https://pi.dev/session/#" + gistID
	}

	if m.program != nil {
		m.program.Send(ShareResultMsg{GistURL: gistURL, PreviewURL: previewURL})
	}
}

// buildSessionHTML creates an HTML representation of the current session.
func (m *AppModel) buildSessionHTML() string {
	if m.session == nil || len(m.session.Entries) == 0 {
		return ""
	}

	var sb strings.Builder
	sb.WriteString(`<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>xihu session`)
	if name := m.session.GetSessionName(); name != "" {
		sb.WriteString(" - " + name)
	}
	sb.WriteString(`</title>
<style>
body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; max-width: 900px; margin: 0 auto; padding: 20px; background: #1e1e2e; color: #cdd6f4; }
.header { border-bottom: 1px solid #45475a; padding-bottom: 10px; margin-bottom: 20px; }
.header h1 { color: #89b4fa; margin: 0; font-size: 1.5em; }
.header .meta { color: #6c7086; font-size: 0.85em; margin-top: 5px; }
.entry { margin-bottom: 15px; padding: 10px 15px; border-radius: 8px; }
.entry.user { background: #313244; border-left: 3px solid #a6e3a1; }
.entry.assistant { background: #1e1e2e; border-left: 3px solid #89b4fa; }
.entry.system { background: #181825; border-left: 3px solid #f9e2af; }
.entry.tool { background: #11111b; border-left: 3px solid #fab387; }
.role { font-weight: bold; font-size: 0.8em; text-transform: uppercase; margin-bottom: 5px; }
.user .role { color: #a6e3a1; }
.assistant .role { color: #89b4fa; }
.system .role { color: #f9e2af; }
.tool .role { color: #fab387; }
.content { white-space: pre-wrap; word-wrap: break-word; line-height: 1.5; }
pre { background: #11111b; padding: 10px; border-radius: 6px; overflow-x: auto; }
code { background: #45475a; padding: 2px 5px; border-radius: 3px; font-size: 0.9em; }
.footer { margin-top: 30px; padding-top: 10px; border-top: 1px solid #45475a; color: #6c7086; font-size: 0.85em; }
</style>
</head>
<body>
<div class="header">
<h1>xihu session`)
	if name := m.session.GetSessionName(); name != "" {
		sb.WriteString(": " + name)
	}
	sb.WriteString(`</h1>
<div class="meta">` + m.session.CWD + ` · ` + time.Now().Format(time.RFC3339) + `</div>
</div>
`)

	for _, entry := range m.session.Entries {
		role := entry.Role
		if role == "" {
			role = entry.Type
		}
		if role == "" {
			role = "unknown"
		}
		sb.WriteString(`<div class="entry ` + role + `">`)
		sb.WriteString(`<div class="role">` + role + `</div>`)

		var contentBlocks []struct {
			Type string `json:"type"`
			Text string `json:"text"`
		}
		if err := json.Unmarshal(entry.Content, &contentBlocks); err == nil {
			for _, block := range contentBlocks {
				if block.Type == "text" && block.Text != "" {
					escaped := strings.ReplaceAll(block.Text, "&", "&amp;")
					escaped = strings.ReplaceAll(escaped, "<", "&lt;")
					escaped = strings.ReplaceAll(escaped, ">", "&gt;")
					sb.WriteString(`<div class="content">` + escaped + `</div>`)
				}
			}
		}
			sb.WriteString("</div>\n")
	}

	sb.WriteString(`<div class="footer">Exported by xihu · ` + time.Now().Format(time.RFC3339) + `</div>
</body>
</html>`)

	return sb.String()
}

// ShareResultMsg carries the result of the /share command back to the TUI.
type ShareResultMsg struct {
	GistURL    string
	PreviewURL string
	Error      string
}

// handleDebugCommand dumps debug information to a log file (TS pi-mono: /debug).
func (m *AppModel) handleDebugCommand() string {
	home, _ := os.UserHomeDir()
	debugDir := home + "/.xihu"
	os.MkdirAll(debugDir, 0755)
	debugPath := debugDir + "/debug.log"

	var sb strings.Builder
	sb.WriteString(fmt.Sprintf("Debug output at %s\n", time.Now().Format(time.RFC3339)))
	sb.WriteString(fmt.Sprintf("Terminal: %dx%d\n", m.width, m.height))
	sb.WriteString(fmt.Sprintf("Model: %s\n", m.agent.Loop().Model))
	sb.WriteString(fmt.Sprintf("Session: %s\n", m.session.ID))
	sb.WriteString(fmt.Sprintf("Thinking level: %s\n", m.thinkingLevel))
	sb.WriteString(fmt.Sprintf("Streaming: %v  Compacting: %v\n", m.streaming, m.compacting))
	sb.WriteString(fmt.Sprintf("Entries: %d  Tokens in: %d  out: %d\n",
		len(m.session.Entries), m.lastStatus.TokensIn, m.lastStatus.TokensOut))
	sb.WriteString("\n=== Session entries ===\n")
	for i, entry := range m.session.Entries {
		content := string(entry.Content)
		if len(content) > 500 {
			content = content[:500] + "..."
		}
		sb.WriteString(fmt.Sprintf("[%d] type=%s id=%s\n  %s\n", i, entry.Type, entry.ID, content))
	}

	if err := os.WriteFile(debugPath, []byte(sb.String()), 0644); err != nil {
		return "Debug: failed to write " + debugPath + ": " + err.Error()
	}
	return "Debug log written\n" + debugPath
}

// findTemplate looks up a prompt template by name (with or without leading /).
func (m *AppModel) findTemplate(name string) *prompt.PromptTemplate {
	// Strip leading / if present
	name = strings.TrimPrefix(name, "/")
	for i := range m.promptTemplates {
		if m.promptTemplates[i].Name == name {
			return &m.promptTemplates[i]
		}
	}
	return nil
}

// splitSlashCommand splits "/name arg1 arg2" into ("name", ["arg1", "arg2"]).
