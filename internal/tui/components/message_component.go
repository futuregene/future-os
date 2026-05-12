package components

import (
	"github.com/charmbracelet/glamour"
	"github.com/charmbracelet/lipgloss"
)

// MessageComponent is the interface that each message-type component implements,
// matching TS pi-mono's pattern where each message type is its own component class.
// Each component encapsulates its own rendering logic for a single ChatEntry.
type MessageComponent interface {
	// Render produces the terminal output for this message entry.
	Render(entry ChatEntry, width int) string
}

// MessageComponentBase holds shared state that all message components need:
// styles, markdown renderer, and global toggles.
type MessageComponentBase struct {
	Width             int
	MdRenderer        *glamour.TermRenderer
	HideAllThinking   *bool
	HiddenThinkingLabel *string
	AllToolsExpanded  *bool
	ShowImages        *bool
	ImageWidthCells   *int
	ToolToggleKey     *string
	SpinnerFrame      *int

	// Styles (populated by SetTheme)
	AssistantStyle  lipgloss.Style
	ThinkingStyle   lipgloss.Style
	ThinkingDim     lipgloss.Style
	ToolStyle       lipgloss.Style
	ToolPendingBg   lipgloss.Style
	ToolSuccessBg   lipgloss.Style
	ToolErrorBg     lipgloss.Style
	ToolSuccess     lipgloss.Style
	ToolError       lipgloss.Style
	ErrorStyle      lipgloss.Style
	SystemStyle     lipgloss.Style
	WarningStyle    lipgloss.Style
	BashBorder      lipgloss.Style
	BashHeader      lipgloss.Style
	BashOutput      lipgloss.Style
	BashStatus      lipgloss.Style
	BashErrorStatus lipgloss.Style
	DiffAdd         lipgloss.Style
	DiffDel         lipgloss.Style
	DiffCtx         lipgloss.Style
	DiffHeader      lipgloss.Style
	CustomMessageBg    lipgloss.Style
	CustomMessageLabel lipgloss.Style
	CustomLabelStyle   lipgloss.Style
	CustomDimStyle     lipgloss.Style
	UserMessageBg      lipgloss.Style
	BorderStyle        lipgloss.Style
}

// DefaultComponentStyles returns a MessageComponentBase populated with default OneDark styles.
func DefaultComponentStyles() MessageComponentBase {
	hideThinking := false
	hiddenThinkingLabel := "Thinking..."
	allExpanded := false
	showImages := false
	imageWidth := 0
	toggleKey := "Ctrl+O"
	spinner := 0

	return MessageComponentBase{
		HideAllThinking:    &hideThinking,
		HiddenThinkingLabel: &hiddenThinkingLabel,
		AllToolsExpanded:   &allExpanded,
		ShowImages:         &showImages,
		ImageWidthCells:     &imageWidth,
		ToolToggleKey:      &toggleKey,
		SpinnerFrame:       &spinner,
		AssistantStyle: lipgloss.NewStyle().Foreground(lipgloss.Color("#abb2bf")),
		ThinkingStyle:  lipgloss.NewStyle().Foreground(lipgloss.Color("#c678dd")).Italic(true),
		ThinkingDim:    lipgloss.NewStyle().Foreground(lipgloss.Color("#5c6370")).Italic(true),
		ToolStyle:      lipgloss.NewStyle().Foreground(lipgloss.Color("#ffffff")),
		ToolPendingBg:  lipgloss.NewStyle().Background(lipgloss.Color("#282832")).Foreground(lipgloss.Color("#ffffff")),
		ToolSuccessBg:  lipgloss.NewStyle().Background(lipgloss.Color("#283228")).Foreground(lipgloss.Color("#ffffff")),
		ToolErrorBg:    lipgloss.NewStyle().Background(lipgloss.Color("#3c2828")).Foreground(lipgloss.Color("#ffffff")),
		ToolSuccess:    lipgloss.NewStyle().Foreground(lipgloss.Color("#98c379")),
		ToolError:      lipgloss.NewStyle().Foreground(lipgloss.Color("#e06c75")),
		ErrorStyle:     lipgloss.NewStyle().Foreground(lipgloss.Color("#e06c75")),
		SystemStyle:    lipgloss.NewStyle().Foreground(lipgloss.Color("#5c6370")).Italic(true),
		WarningStyle:   lipgloss.NewStyle().Foreground(lipgloss.Color("#e5c07b")),
		BashBorder:     lipgloss.NewStyle().Foreground(lipgloss.Color("#5c6370")),
		BashHeader:     lipgloss.NewStyle().Foreground(lipgloss.Color("#e5c07b")).Bold(true),
		BashOutput:     lipgloss.NewStyle().Foreground(lipgloss.Color("#6c7086")),
		BashStatus:     lipgloss.NewStyle().Foreground(lipgloss.Color("#98c379")),
		BashErrorStatus: lipgloss.NewStyle().Foreground(lipgloss.Color("#e06c75")),
		CustomMessageBg:    lipgloss.NewStyle().Background(lipgloss.Color("#2d2838")),
		CustomMessageLabel: lipgloss.NewStyle().Foreground(lipgloss.Color("#9575cd")).Bold(true),
		CustomLabelStyle:   lipgloss.NewStyle().Foreground(lipgloss.Color("#cba6f7")).Bold(true),
		CustomDimStyle:      lipgloss.NewStyle().Foreground(lipgloss.Color("#6c7086")),
		UserMessageBg:      lipgloss.NewStyle().Background(lipgloss.Color("#343541")),
		BorderStyle:        lipgloss.NewStyle().Foreground(lipgloss.Color("#5c6370")),
	}
}
