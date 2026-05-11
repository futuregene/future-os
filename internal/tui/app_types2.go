// Package tui provides the interactive terminal UI for xihu using Bubble Tea.
package tui

import (

	"time"

	tea "github.com/charmbracelet/bubbletea"

	"github.com/huichen/xihu/internal/extensions"


)

// ─── Message Types ─────────────────────────────────────────────────────────

// StreamTextMsg is a chunk of streamed text from the LLM.

type refreshSettingsMsg struct{}

// refreshModelSelectorMsg is an internal message to refresh the model selector.
type refreshModelSelectorMsg struct{}

// ─── Extension UI Bridge ────────────────────────────────────────────────────

// extensionSelectMsg is sent to show an extension selector dialog.
type extensionSelectMsg struct {
	title   string
	options []string
	timeout time.Duration
	respCh  chan extensionUIResponse
}

// extensionInputMsg is sent to show an extension input dialog.
type extensionInputMsg struct {
	title       string
	placeholder string
	timeout     time.Duration
	respCh      chan extensionUIResponse
}

// extensionEditorMsg is sent to show an extension editor dialog.
type extensionEditorMsg struct {
	title   string
	prefill string
	respCh  chan extensionUIResponse
}

// extensionUIResponse carries the result of an extension UI dialog.
type extensionUIResponse struct {
	value string
	err   error
}

// tuiExtensionBridge implements extensions.ExtensionUI using the Bubble Tea overlay system.
type tuiExtensionBridge struct {
	program       *tea.Program
	inputRegistry *terminalInputRegistry
}

type appendSystemMsg string

// appendErrorMsg appends an error message to the chat from any goroutine.
type appendErrorMsg string

// appendWarningMsg appends a warning message to the chat from any goroutine.
type appendWarningMsg string

type extensionStatusMsg struct {
	key  string
	text string
}

// extensionSetTitleMsg sets the terminal window/tab title.
type extensionSetTitleMsg struct {
	title string
}

// extensionHiddenThinkingLabelMsg sets the label for hidden thinking blocks.
type extensionHiddenThinkingLabelMsg struct {
	label string
}

// extensionWorkingMessageMsg sets the working message shown during streaming.
type extensionWorkingMessageMsg struct {
	message string
}

// extensionWorkingVisibleMsg shows or hides the working loader during streaming.
type extensionWorkingVisibleMsg struct {
	visible bool
}

// extensionWorkingIndicatorMsg sets custom spinner frames for the streaming loader.
type extensionWorkingIndicatorMsg struct {
	frames     []string
	intervalMs int
}

// extensionEditorTextMsg sets or retrieves editor text.
type extensionEditorTextMsg struct {
	text   string
	isSet  bool
	respCh chan string // used for GetEditorText
}

// extensionPasteMsg pastes text into the main editor.
type extensionPasteMsg struct {
	text string
}

// extensionWidgetMsg sets or clears an extension widget.
type extensionWidgetMsg struct {
	key       string
	content   string // empty to remove
	placement string // "aboveEditor" or "belowEditor"
}

type extensionGetAllThemesMsg struct {
	respCh chan []extensions.ThemeInfo
}

// extensionGetCurrentThemeNameMsg is sent to get the current theme name.
type extensionGetCurrentThemeNameMsg struct {
	respCh chan string
}

// extensionSetThemeMsg is sent to switch the current theme.
type extensionSetThemeMsg struct {
	name   string
	respCh chan error
}

// extensionGetToolsExpandedMsg is sent to get the tools expansion state.
type extensionGetToolsExpandedMsg struct {
	respCh chan bool
}

// extensionSetToolsExpandedMsg is sent to set the tools expansion state.
type extensionSetToolsExpandedMsg struct {
	expanded bool
}

// extensionCustomMsg is sent to show an extension custom dialog.
type extensionCustomMsg struct {
	title   string
	content string
	buttons []extensions.CustomButton
	timeout time.Duration
	respCh  chan extensionUIResponse
}

