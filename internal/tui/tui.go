package tui

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"strings"

	tea "github.com/charmbracelet/bubbletea"
	"golang.org/x/term"

	"github.com/huichen/xihu/internal/agent"
	"github.com/huichen/xihu/internal/extensions"
	"github.com/huichen/xihu/internal/prompt"
	"github.com/huichen/xihu/internal/session"
	"github.com/huichen/xihu/internal/settings"
	"github.com/huichen/xihu/internal/skills"
	"github.com/huichen/xihu/internal/tui/components"
	"github.com/huichen/xihu/pkg/types"
)

// Run launches the TUI if stdin is a terminal; otherwise falls back to CLI mode.
func Run(
	agt *agent.Loop,
	sessMgr *session.Manager,
	sess *session.Session,
	initialPrompt string,
	modelStr, baseURL string,
	skillList []skills.Skill,
	extensions []string,
	thinkingLevel string,
	availableModels []string,
	cfg *settings.Settings,
	extRunner *extensions.ExtensionRunner,
	promptTemplates []prompt.PromptTemplate,
	contextFiles []string,
) error {
	if !term.IsTerminal(int(os.Stdin.Fd())) {
		return runCLI(agt, sess, initialPrompt)
	}
	return runBubbleTea(agt, sessMgr, sess, initialPrompt, modelStr, baseURL, skillList, extensions, thinkingLevel, availableModels, cfg, extRunner, promptTemplates, contextFiles)
}

// runBubbleTea launches the Bubble Tea interactive TUI.
func runBubbleTea(
	agt *agent.Loop,
	sessMgr *session.Manager,
	sess *session.Session,
	initialPrompt string,
	modelStr, baseURL string,
	skillList []skills.Skill,
	extensions []string,
	thinkingLevel string,
	availableModels []string,
	cfg *settings.Settings,
	extRunner *extensions.ExtensionRunner,
	promptTemplates []prompt.PromptTemplate,
	contextFiles []string,
) error {
	theme := DefaultTheme()
	app := NewAppModel(agt, sessMgr, sess, theme, modelStr, skillList, extensions, thinkingLevel, availableModels, cfg, promptTemplates, contextFiles)

	p := tea.NewProgram(
		&app,
		tea.WithAltScreen(),
		tea.WithMouseCellMotion(),
	)

	// Set program reference so goroutines can send messages
	app.program = p

	// Set up extension UI bridge for runtime extension dialogs
	app.extensionBridge = &tuiExtensionBridge{program: p, inputRegistry: app.inputRegistry}
	app.extensionStatuses = make(map[string]string)

	// Store extension runner for command dispatch
	app.extRunner = extRunner

	// If we have an initial prompt, send it as a message
	if initialPrompt != "" {
		go func() {
			p.Send(components.SubmitMsg(initialPrompt))
		}()
	}

	_, err := p.Run()
	return err
}

// runCLI runs in non-interactive CLI mode (stdin is not a TTY).
func runCLI(agt *agent.Loop, sess *session.Session, initialPrompt string) error {
	if initialPrompt == "" {
		return fmt.Errorf("no prompt provided in CLI mode")
	}

	var messages []types.Message
	if len(sess.Entries) > 0 {
		messages = session.BuildContext(sess.Entries)
	}
	messages = append(messages, newUserMsg(initialPrompt))

	ctx := context.Background()
	result, _, err := agt.RunStreamingWithMessages(ctx, messages, func(text string) {
		fmt.Print(text)
	})
	if err != nil {
		return err
	}
	_ = result
	fmt.Println()
	return nil
}

var _ = strings.Repeat

func newUserMsg(content string) types.Message {
	tc := types.TextContent{Type: "text", Text: content}
	b, _ := json.Marshal([]types.TextContent{tc})
	return types.Message{Role: "user", Content: b}
}
