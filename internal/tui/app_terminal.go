// Package tui provides the interactive terminal UI for xihu using Bubble Tea.
package tui

import (

	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"




)

// ─── Message Types ─────────────────────────────────────────────────────────

// StreamTextMsg is a chunk of streamed text from the LLM.

func (m *AppModel) startProgress() {
	if !m.terminalProgress {
		return
	}
	m.stopProgress() // cancel any existing keepalive
	m.progressCancel = make(chan struct{})
	fmt.Fprint(os.Stdout, "\x1b]9;4;3\x07")
	// Keepalive: re-send every 1s to prevent terminal timeout
	go func(cancel <-chan struct{}) {
		ticker := time.NewTicker(1 * time.Second)
		defer ticker.Stop()
		for {
			select {
			case <-cancel:
				return
			case <-ticker.C:
				fmt.Fprint(os.Stdout, "\x1b]9;4;3\x07")
			}
		}
	}(m.progressCancel)
}

// stopProgress writes OSC 9;4;0 to reset the terminal progress bar.
// (TS pi-mono: terminal.ts setProgress(false))
func (m *AppModel) stopProgress() {
	if m.progressCancel != nil {
		close(m.progressCancel)
		m.progressCancel = nil
	}
	fmt.Fprint(os.Stdout, "\x1b]9;4;0\x07")
}

// setTerminalTitle writes the OSC title sequence to set the terminal title.
// (TS pi-mono: terminal.ts setTitle() — \x1b]0;{title}\x07)
func (m *AppModel) setTerminalTitle() {
	if m.session == nil {
		return
	}
	name := m.session.GetSessionName()
	if name == "" {
		name = m.session.ID
	}
	// Use basename only (TS pi-mono: terminal title shows cwdBasename)
	cwd := filepath.Base(m.session.CWD)
	title := fmt.Sprintf("xihu - %s - %s", name, cwd)
	fmt.Fprintf(os.Stdout, "\x1b]0;%s\x07", title)
}

// showLoginDialog displays an interactive auth provider selector.
// (TS pi-mono: login-dialog.ts showAuth with provider list)
func (m *AppModel) openExternalEditor() string {
	editor := os.Getenv("VISUAL")
	if editor == "" {
		editor = os.Getenv("EDITOR")
	}
	if editor == "" {
		m.chat.AppendWarning("No editor configured. Set $VISUAL or $EDITOR environment variable.")
		return ""
	}

	tmpDir := os.TempDir()
	f, err := os.CreateTemp(tmpDir, "xihu-edit-*.md")
	if err != nil {
		m.chat.AppendSystem("Error: " + err.Error())
		return ""
	}
	defer os.Remove(f.Name())

	// Pre-fill with current input text
	currentInput := m.input.Value()
	if currentInput != "" {
		f.WriteString(currentInput)
	}
	f.Close()

	// Suspend Bubble Tea, run editor, resume
	cmd := exec.Command(editor, f.Name())
	cmd.Stdin = os.Stdin
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	if err := cmd.Run(); err != nil {
		// pi-mono: non-zero exit keeps original text silently
		return ""
	}

	content, err := os.ReadFile(f.Name())
	if err != nil {
		m.chat.AppendSystem("Error reading file: " + err.Error())
		return ""
	}
	text := string(content)
	// Strip only trailing newline (pi-mono: editors add trailing \n)
	text = strings.TrimSuffix(text, "\n")
	if strings.TrimSpace(text) == "" {
		return ""
	}
	return text
}

// extractBashCommand extracts the "command" field from JSON tool arguments.
