package extensions

import "time"

// ---------------------------------------------------------------------------
// ExtensionUI — UI methods available to extensions at runtime
// ---------------------------------------------------------------------------

// TerminalInputResult is returned by TerminalInputHandler.
type TerminalInputResult struct {
	Consume bool   // if true, the input is consumed and not processed further
	Data    string // if non-empty, replaces the original input string
}

// TerminalInputHandler processes raw terminal input before normal key handling.
// Return nil to pass the input through normally.
type TerminalInputHandler func(data string) *TerminalInputResult

// ExtensionUIDialogOptions configures an extension UI dialog.
type ExtensionUIDialogOptions struct {
	Timeout time.Duration
	Signal  chan struct{} // close to programmatically dismiss
}

// ExtensionUI provides UI interaction methods for extensions.
// Each method blocks until the user responds or the dialog is dismissed.
type ExtensionUI interface {
	// Select shows a list selector and returns the user's choice.
	Select(title string, options []string, opts *ExtensionUIDialogOptions) (string, error)

	// Confirm shows a confirmation dialog. Returns true if confirmed.
	Confirm(title, message string, opts *ExtensionUIDialogOptions) (bool, error)

	// Input shows a text input dialog and returns the user's input.
	Input(title, placeholder string, opts *ExtensionUIDialogOptions) (string, error)

	// Editor shows a multi-line editor and returns the user's text.
	Editor(title, prefill string) (string, error)

	// Notify shows a notification (info, warning, error).
	Notify(message string, notifyType string)

	// SetStatus sets a status line in the footer. Pass empty text to clear.
	SetStatus(key, text string)

	// SetTitle sets the terminal window/tab title.
	SetTitle(title string)

	// SetHiddenThinkingLabel sets the label for hidden thinking blocks.
	// Pass empty string to restore the default ("Thinking…").
	SetHiddenThinkingLabel(label string)

	// SetWorkingMessage sets the working message shown during streaming.
	// Pass empty string to restore the default ("Generating…").
	SetWorkingMessage(message string)

	// SetWorkingVisible shows or hides the working loader during streaming.
	SetWorkingVisible(visible bool)

	// SetWorkingIndicator sets the spinner frames for the streaming loader.
	// Pass nil or empty slice to restore default spinner.
	SetWorkingIndicator(frames []string, intervalMs int)

	// OnTerminalInput registers a raw terminal input handler.
	// The handler is called for every keypress before normal processing.
	// Return &TerminalInputResult{Consume: true} to stop further processing.
	// Returns an unsubscribe function. Call it to remove the handler.
	OnTerminalInput(handler TerminalInputHandler) (unsubscribe func())

	// PasteToEditor pastes text into the main editor with paste handling.
	PasteToEditor(text string)

	// SetEditorText sets the text content of the main editor.
	SetEditorText(text string)

	// GetEditorText returns the current text in the main editor.
	GetEditorText() string

	// SetWidget sets or removes a widget rendered above or below the editor.
	// key uniquely identifies the widget. content is the rendered widget text
	// (multiple lines joined by \n). Pass empty content to remove the widget.
	// placement is "aboveEditor" or "belowEditor".
	SetWidget(key, content, placement string)

	// Custom shows a custom dialog with title, content text, and action buttons.
	// Returns the value of the selected button, or an error if cancelled.
	Custom(title, content string, buttons []CustomButton, opts *ExtensionUIDialogOptions) (string, error)

	// GetAllThemes returns all available themes with their names and file paths.
	// Built-in themes have empty paths.
	GetAllThemes() []ThemeInfo

	// SetTheme applies a theme by name. Returns an error if the theme is not found.
	SetTheme(name string) error

	// GetCurrentThemeName returns the name of the currently active theme.
	GetCurrentThemeName() string

	// GetToolsExpanded returns whether tool outputs are currently expanded.
	GetToolsExpanded() bool

	// SetToolsExpanded sets the tool output expansion state.
	SetToolsExpanded(expanded bool)

	// AddAutocompleteProvider registers an autocomplete provider function.
	// The provider is called with the current query prefix and returns candidate strings.
	AddAutocompleteProvider(provider AutocompleteProvider)

	// SetFooter replaces the footer with a custom component factory.
	// factory is a func() FooterComponent. Pass nil to restore the default footer.
	// FooterComponent must implement: View(width int) string.
	SetFooter(factory interface{})

	// SetHeader replaces the header with a custom component factory.
	// factory is a func() HeaderComponent. Pass nil to restore the default header.
	// HeaderComponent must implement: View(width int) string.
	SetHeader(factory interface{})

	// GetTheme loads a theme by name. Returns nil if not found.
	GetTheme(name string) interface{}

	// SetEditorComponent replaces the entire editor component.
	// factory is a func() EditorComponent. Pass nil to restore the default editor.
	// EditorComponent must implement:
	//   Init() tea.Cmd
	//   Update(msg tea.Msg) (tea.Model, tea.Cmd)
	//   View() string
	//   Value() string
	//   SetValue(string)
	//   Reset()
	//   Focus() tea.Cmd
	//   Blur()
	//   SetWidth(int), SetHeight(int), Height() int, Empty() bool
	SetEditorComponent(factory interface{})

	// GetEditorComponent returns the current custom editor factory, or nil.
	GetEditorComponent() interface{}
}

// ThemeInfo describes an available theme for GetAllThemes.
type ThemeInfo struct {
	Name string
	Path string // empty for built-in themes
}

// CustomButton represents an action button in a custom dialog.
type CustomButton struct {
	Label string // display text
	Value string // value returned when selected
}

// noopUI is a no-op ExtensionUI used when no TUI is available.
type noopUI struct{}

func (n *noopUI) Select(string, []string, *ExtensionUIDialogOptions) (string, error) { return "", nil }
func (n *noopUI) Confirm(string, string, *ExtensionUIDialogOptions) (bool, error)   { return false, nil }
func (n *noopUI) GetAllThemes() []ThemeInfo                                          { return nil }
func (n *noopUI) SetTheme(string) error                                              { return nil }
func (n *noopUI) GetCurrentThemeName() string                                        { return "" }
func (n *noopUI) GetToolsExpanded() bool                                             { return false }
func (n *noopUI) SetToolsExpanded(bool)                                              {}
func (n *noopUI) AddAutocompleteProvider(AutocompleteProvider)                       {}
func (n *noopUI) SetFooter(interface{})                                              {}
func (n *noopUI) SetHeader(interface{})                                              {}
func (n *noopUI) GetTheme(string) interface{}                                        { return nil }
func (n *noopUI) SetEditorComponent(interface{})                                     {}
func (n *noopUI) GetEditorComponent() interface{}                                    { return nil }
func (n *noopUI) Input(string, string, *ExtensionUIDialogOptions) (string, error)    { return "", nil }
func (n *noopUI) Editor(string, string) (string, error)                              { return "", nil }
func (n *noopUI) Notify(string, string)                                              {}
func (n *noopUI) SetStatus(string, string)                                           {}
func (n *noopUI) SetTitle(string)                                                    {}
func (n *noopUI) SetHiddenThinkingLabel(string)                                      {}
func (n *noopUI) SetWorkingMessage(string)                                           {}
func (n *noopUI) SetWorkingVisible(bool)                                             {}
func (n *noopUI) SetWorkingIndicator([]string, int)                                  {}
func (n *noopUI) OnTerminalInput(TerminalInputHandler) func()                        { return func() {} }
func (n *noopUI) PasteToEditor(string)                                               {}
func (n *noopUI) SetEditorText(string)                                               {}
func (n *noopUI) GetEditorText() string                                              { return "" }
func (n *noopUI) SetWidget(string, string, string)                                   {}
func (n *noopUI) Custom(string, string, []CustomButton, *ExtensionUIDialogOptions) (string, error) {
	return "", nil
}

// NoopUI is a no-op ExtensionUI that returns empty/zero values.
var NoopUI ExtensionUI = &noopUI{}
