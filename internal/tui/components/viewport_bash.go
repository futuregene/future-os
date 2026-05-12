package components

import (
	"strings"
	"path/filepath"

)

func classifyCompactRead(args string) (kind, label string) {
	path := extractJSONField(args, "file_path")
	if path == "" {
		path = extractJSONField(args, "path")
	}
	if path == "" {
		return "", ""
	}

	base := filepath.Base(path)
	if base == "SKILL.md" {
		parent := filepath.Base(filepath.Dir(path))
		if parent == "" || parent == "." {
			parent = base
		}
		return "skill", parent
	}

	if compactReadFileNames[base] {
		return "resource", base
	}

	// Check for pi docs: README.md, docs/*, examples/*
	slashPath := filepath.ToSlash(path)
	if base == "README.md" || strings.HasPrefix(slashPath, "docs/") || strings.HasPrefix(slashPath, "examples/") {
		return "docs", slashPath
	}

	return "", ""
}

// markCompactRead marks a tool_call entry for compact rendering if applicable (TS pi-mono: compact read call).
func (c *ChatViewport) markCompactRead(idx int) {
	if idx < 0 || idx >= len(c.entries) {
		return
	}
	e := &c.entries[idx]
	if e.ToolName != "read" {
		return
	}
	kind, label := classifyCompactRead(e.ToolArgs)
	if kind != "" {
		e.CompactReadKind = kind
		e.CompactReadLabel = label
		e.Expanded = false // collapsed by default for system files
	}
}

// CompleteToolCall finalizes a pending tool_call entry's arguments in-place.
// If no matching pending entry exists, it creates a new one (fallback).
func (c *ChatViewport) AddBashExecution(command string, excluded bool) int {
	c.mu.Lock()
	defer c.mu.Unlock()
	idx := len(c.entries)
	c.entries = append(c.entries, ChatEntry{
		Type:         "bash",
		BashCommand:  command,
		BashRunning:  true,
		BashExcluded: excluded,
		Expanded:     false,
	})
	if c.vp.AtBottom() {
		c.autoScroll = true
	}
	return idx
}

// AppendBashOutput appends output lines to the last bash execution entry.
// Strips ANSI escape sequences from output matching TS pi-mono stripAnsi behavior.
func (c *ChatViewport) AppendBashOutput(lines string) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if len(c.entries) == 0 {
		return
	}
	last := &c.entries[len(c.entries)-1]
	if last.Type != "bash" {
		return
	}
		// Strip ANSI codes and normalize line endings (TS pi-mono: stripAnsi)
		clean := stripAnsiCodes(lines)
		clean = strings.ReplaceAll(clean, "\r\n", "\n")
		clean = strings.ReplaceAll(clean, "\r", "\n")
		for _, line := range strings.Split(clean, "\n") {
		last.BashLines = append(last.BashLines, line)
	}
	if c.vp.AtBottom() {
		c.autoScroll = true
	}
}

// CompleteBash marks the last bash execution as complete with an exit code.
func (c *ChatViewport) CompleteBash(exitCode int, cancelled bool) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if len(c.entries) == 0 {
		return
	}
	last := &c.entries[len(c.entries)-1]
	if last.Type != "bash" {
		return
	}
	last.BashRunning = false
	last.BashExitCode = exitCode
	if cancelled {
		last.BashExitCode = -1
	}
	// Ensure viewport scrolls to show completion status
	c.autoScroll = true
}

// SetBashTruncation sets truncation info on the last bash entry (TS pi-mono: truncation warning inline in border).
func (c *ChatViewport) SetBashTruncation(truncated bool, fullOutputPath string) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if len(c.entries) == 0 {
		return
	}
	last := &c.entries[len(c.entries)-1]
	if last.Type != "bash" {
		return
	}
	last.BashTruncated = truncated
	last.BashFullOutputPath = fullOutputPath
}

// AppendError adds an error message.
