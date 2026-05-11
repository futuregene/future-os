// Package tui provides the interactive terminal UI for xihu using Bubble Tea.
package tui

import (



	"github.com/huichen/xihu/internal/extensions"


)

// ─── Message Types ─────────────────────────────────────────────────────────

// StreamTextMsg is a chunk of streamed text from the LLM.

func (b *tuiExtensionBridge) Select(title string, options []string, opts *extensions.ExtensionUIDialogOptions) (string, error) {
	respCh := make(chan extensionUIResponse, 1)
	msg := extensionSelectMsg{
		title:   title,
		options: options,
		respCh:  respCh,
	}
	if opts != nil && opts.Timeout > 0 {
		msg.timeout = opts.Timeout
	}
	b.program.Send(msg)
	resp := <-respCh
	return resp.value, resp.err
}

func (b *tuiExtensionBridge) Confirm(title, message string, opts *extensions.ExtensionUIDialogOptions) (bool, error) {
	val, err := b.Select(title+"\n"+message, []string{"Yes", "No"}, opts)
	return val == "Yes", err
}

func (b *tuiExtensionBridge) Input(title, placeholder string, opts *extensions.ExtensionUIDialogOptions) (string, error) {
	respCh := make(chan extensionUIResponse, 1)
	msg := extensionInputMsg{
		title:       title,
		placeholder: placeholder,
		respCh:      respCh,
	}
	if opts != nil && opts.Timeout > 0 {
		msg.timeout = opts.Timeout
	}
	b.program.Send(msg)
	resp := <-respCh
	return resp.value, resp.err
}

func (b *tuiExtensionBridge) Editor(title, prefill string) (string, error) {
	respCh := make(chan extensionUIResponse, 1)
	b.program.Send(extensionEditorMsg{
		title:   title,
		prefill: prefill,
		respCh:  respCh,
	})
	resp := <-respCh
	return resp.value, resp.err
}

func (b *tuiExtensionBridge) Notify(message string, notifyType string) {
	switch notifyType {
	case "error":
		b.program.Send(appendErrorMsg(message))
	case "warning":
		b.program.Send(appendWarningMsg(message))
	default:
		b.program.Send(appendSystemMsg(message))
	}
}

// appendSystemMsg appends a system message to the chat from any goroutine.
func (b *tuiExtensionBridge) SetStatus(key, text string) {
	b.program.Send(extensionStatusMsg{key: key, text: text})
}

func (b *tuiExtensionBridge) SetTitle(title string) {
	b.program.Send(extensionSetTitleMsg{title: title})
}

func (b *tuiExtensionBridge) SetHiddenThinkingLabel(label string) {
	b.program.Send(extensionHiddenThinkingLabelMsg{label: label})
}

func (b *tuiExtensionBridge) SetWorkingMessage(message string) {
	b.program.Send(extensionWorkingMessageMsg{message: message})
}

func (b *tuiExtensionBridge) SetWorkingVisible(visible bool) {
	b.program.Send(extensionWorkingVisibleMsg{visible: visible})
}

func (b *tuiExtensionBridge) SetWorkingIndicator(frames []string, intervalMs int) {
	b.program.Send(extensionWorkingIndicatorMsg{frames: frames, intervalMs: intervalMs})
}

func (b *tuiExtensionBridge) PasteToEditor(text string) {
	b.program.Send(extensionPasteMsg{text: text})
}

func (b *tuiExtensionBridge) SetEditorText(text string) {
	b.program.Send(extensionEditorTextMsg{text: text, isSet: true})
}

func (b *tuiExtensionBridge) GetEditorText() string {
	respCh := make(chan string, 1)
	b.program.Send(extensionEditorTextMsg{respCh: respCh})
	return <-respCh
}

func (b *tuiExtensionBridge) OnTerminalInput(handler extensions.TerminalInputHandler) func() {
	id := b.inputRegistry.add(handler)
	return func() { b.inputRegistry.remove(id) }
}

// extensionStatusMsg updates an extension status line in the footer.
func (b *tuiExtensionBridge) SetWidget(key, content, placement string) {
	b.program.Send(extensionWidgetMsg{key: key, content: content, placement: placement})
}

// extensionGetAllThemesMsg is sent to get all available themes.
func (b *tuiExtensionBridge) Custom(title, content string, buttons []extensions.CustomButton, opts *extensions.ExtensionUIDialogOptions) (string, error) {
	respCh := make(chan extensionUIResponse, 1)
	msg := extensionCustomMsg{
		title:   title,
		content: content,
		buttons: buttons,
		respCh:  respCh,
	}
	if opts != nil && opts.Timeout > 0 {
		msg.timeout = opts.Timeout
	}
	b.program.Send(msg)
	resp := <-respCh
	return resp.value, resp.err
}

func (b *tuiExtensionBridge) GetAllThemes() []extensions.ThemeInfo {
	respCh := make(chan []extensions.ThemeInfo, 1)
	b.program.Send(extensionGetAllThemesMsg{respCh: respCh})
	return <-respCh
}

func (b *tuiExtensionBridge) GetCurrentThemeName() string {
	respCh := make(chan string, 1)
	b.program.Send(extensionGetCurrentThemeNameMsg{respCh: respCh})
	return <-respCh
}

func (b *tuiExtensionBridge) SetTheme(name string) error {
	respCh := make(chan error, 1)
	b.program.Send(extensionSetThemeMsg{name: name, respCh: respCh})
	return <-respCh
}

func (b *tuiExtensionBridge) GetToolsExpanded() bool {
	respCh := make(chan bool, 1)
	b.program.Send(extensionGetToolsExpandedMsg{respCh: respCh})
	return <-respCh
}

func (b *tuiExtensionBridge) AddAutocompleteProvider(provider extensions.AutocompleteProvider) {
	extensions.AddAutocompleteProvider(provider)
}

func (b *tuiExtensionBridge) SetToolsExpanded(expanded bool) {
	b.program.Send(extensionSetToolsExpandedMsg{expanded: expanded})
}

func (b *tuiExtensionBridge) SetFooter(factory interface{}) {
	b.program.Send(extSetFooterMsg{factory: nil})
	// Cast factory to FooterFactory if possible
	if f, ok := factory.(func() FooterComponent); ok {
		b.program.Send(extSetFooterMsg{factory: FooterFactory(f)})
	}
}

func (b *tuiExtensionBridge) SetHeader(factory interface{}) {
	if f, ok := factory.(func() HeaderComponent); ok {
		b.program.Send(extSetHeaderMsg{factory: HeaderFactory(f)})
	} else {
		b.program.Send(extSetHeaderMsg{factory: nil})
	}
}

func (b *tuiExtensionBridge) GetTheme(name string) interface{} {
	// Theme loading via program message
	return nil
}

func (b *tuiExtensionBridge) SetEditorComponent(factory interface{}) {
	if f, ok := factory.(func() EditorComponent); ok {
		b.program.Send(extSetEditorMsg{factory: EditorFactory(f)})
	} else {
		b.program.Send(extSetEditorMsg{factory: nil})
	}
}

func (b *tuiExtensionBridge) GetEditorComponent() interface{} {
	respCh := make(chan EditorFactory, 1)
	b.program.Send(extGetEditorMsg{respCh: respCh})
	return <-respCh
}

// cycleThinking cycles through thinking levels: off → minimal → low → medium → high → xhigh → off.
