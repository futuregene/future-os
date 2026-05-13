// Package tui provides the interactive terminal UI for xihu using Bubble Tea.
package tui

import (

	"fmt"
	"path/filepath"
	"strings"

	"github.com/huichen/xihu/internal/modelregistry"
	"github.com/huichen/xihu/internal/tui/components"
	"github.com/huichen/xihu/pkg/types"

)

// ─── Message Types ─────────────────────────────────────────────────────────

// StreamTextMsg is a chunk of streamed text from the LLM.

// applyScopedModels expands ScopedModels patterns against availableModels
// and populates the scopedModels map with matching exact model IDs.
// Patterns like "openai/*" or "anthropic/*" are supported via filepath.Match.
func (m *AppModel) applyScopedModels(patterns []string) {
	m.scopedModels = make(map[string]bool)
	for _, pat := range patterns {
		// If pattern contains glob characters, expand against available models
		if strings.ContainsAny(pat, "*?[") {
			for _, model := range m.availableModels {
				if matched, _ := filepath.Match(pat, model); matched {
					m.scopedModels[model] = true
				}
			}
		} else {
			// Exact model ID
			m.scopedModels[pat] = true
		}
	}
}
func supportsThinking(modelID string) bool {
	for _, m := range modelregistry.BuiltinModels() {
		if m.ID == modelID {
			return m.Reasoning
		}
	}
	return false
}

// showThemeSelector shows a theme picker overlay (TS pi-mono: /theme).
func (m *AppModel) showModelSelector() {
	if len(m.availableModels) == 0 {
		m.chat.AppendSystem("Only showing models from configured providers. Use /login to add providers.")
		return
	}

	// Build model info lookup from builtins
	modelInfoMap := make(map[string]types.Model)
	for _, info := range modelregistry.BuiltinModels() {
		modelInfoMap[info.ID] = info
	}

	// Helper to build SelectorItems from a model list
	buildItems := func(modelList []string) []components.SelectorItem {
		items := make([]components.SelectorItem, 0, len(modelList))
		for _, model := range modelList {
			name, provider := parseModelString(model)
			isCurrent := model == m.agent.Loop().Model || name == m.agent.Loop().Model
			label := name
			if isCurrent {
				label = "→ " + name + " ✓"
			}
			desc := "[" + provider + "]"
			caps := ""
			if info, ok := modelInfoMap[name]; ok {
				if info.ContextWindow > 0 {
					desc += fmt.Sprintf("  %dK ctx", info.ContextWindow/1000)
				}
				caps := ""
				if info.Reasoning {
					caps += "T"
				}
				// tools check: all models support tools by default unless explicitly marked
				caps += "🔧"
				for _, t := range info.InputTypes {
					if t == "image" {
						caps += "👁"
						break
					}
				}
				if info.Cost.Input > 0 {
					desc += fmt.Sprintf("  $%.1f/$%.1f", info.Cost.Input*10, info.Cost.Output*10)
				}
			} else if info, ok := modelInfoMap[model]; ok {
				if info.ContextWindow > 0 {
					desc += fmt.Sprintf("  %dK ctx", info.ContextWindow/1000)
				}
				caps := ""
				if info.Reasoning {
					caps += "T"
				}
				caps += "🔧"
				for _, t := range info.InputTypes {
					if t == "image" {
						caps += "👁"
						break
					}
				}
				if info.Cost.Input > 0 {
					desc += fmt.Sprintf("  $%.1f/$%.1f", info.Cost.Input*10, info.Cost.Output*10)
				}
			}
			if caps != "" {
				desc += "  " + caps
			}
			if isCurrent {
				desc += " current"
			}
			items = append(items, components.SelectorItem{
				Label:       label,
				Description: desc,
				Value:       model,
			})
		}
		return items
	}

	// Determine if scoped models exist (TS pi-mono: Tab toggles all/scoped)
	hasScoped := len(m.scopedModels) > 0
	scopeAll := !hasScoped // start scoped if scoped models exist

	// Build the scoped model list (only models in scopedModels set)
	scopedList := make([]string, 0, len(m.scopedModels))
	for _, model := range m.availableModels {
		if m.scopedModels[model] {
			scopedList = append(scopedList, model)
		}
	}

	allItems := buildItems(m.availableModels)
	scopedItems := buildItems(scopedList)

	var showOverlay func()
	showOverlay = func() {
		var items []components.SelectorItem
		var title string
		if scopeAll {
			items = allItems
			title = "Models — Scope: all"
		} else {
			items = scopedItems
			title = "Models — Scope: scoped"
		}
		if hasScoped {
			title += "  Tab=toggle scope"
		}

		h := len(items) + 5
		if h > 20 {
			h = 20
		}
		if h < 5 {
			h = 5
		}

		m.overlay.ShowSelectorStayOnSelect(title, items, func(value string) {
			if value != "" && m.program != nil {
				m.program.Send(components.SelectorChosenMsg{Value: value})
			}
		}, func(key string) bool {
			if key == "tab" && hasScoped {
				scopeAll = !scopeAll
				showOverlay()
				return true
			}
			return false
		}, 60, h)
		m.overlay.SetNoMatchText("No matching models")
		// Set selection info to show model name at bottom (TS pi-mono: "Model Name: GPT-4o")
		m.overlay.SetSelectionInfoFunc(func(idx int, item components.SelectorItem) string {
			modelID := item.Value
			name, _ := parseModelString(modelID)
			if info, ok := modelInfoMap[name]; ok && info.Name != "" {
				return "Model Name: " + info.Name
			}
			if info, ok := modelInfoMap[modelID]; ok && info.Name != "" {
				return "Model Name: " + info.Name
			}
			return "Model Name: " + name
		})
	}
	showOverlay()
}

// cycleModelForward cycles to the next available model.
// If scoped models are set, only cycles through scoped models.
func (m *AppModel) cycleModelForward() {
	models := m.getCyclableModels()
	if len(models) == 0 {
		return
	}
	if len(models) == 1 {
		return
	}
	m.modelIndex = (m.modelIndex + 1) % len(models)
	newModel := models[m.modelIndex]
	// Also re-resolve provider (base URL, API key) for the new model
	if err := m.agent.Engine().SwitchModel(newModel); err != nil {
		m.chat.AppendSystem("Failed to switch model: " + err.Error())
		return
	}
	modelName, provider := parseModelString(newModel)
	m.footer.SetSession(m.session.CWD, getGitBranch(m.session.CWD), m.session.GetSessionName(), modelName, m.thinkingLevel, provider)
	m.footer.SetHasReasoning(supportsThinking(modelName))
	m.footer.SetEntryCount(len(m.session.Entries))
	m.input.SetBorderColor(m.theme.ThinkingBorderColor(m.thinkingLevel))
}


func (m *AppModel) cycleModelBackward() {
	models := m.getCyclableModels()
	if len(models) == 0 {
		return
	}
	if len(models) == 1 {
		return
	}
	m.modelIndex--
	if m.modelIndex < 0 {
		m.modelIndex = len(models) - 1
	}
	newModel := models[m.modelIndex]
	// Also re-resolve provider (base URL, API key) for the new model
	if err := m.agent.Engine().SwitchModel(newModel); err != nil {
		m.chat.AppendSystem("Failed to switch model: " + err.Error())
		return
	}
	modelName, provider := parseModelString(newModel)
	m.footer.SetSession(m.session.CWD, getGitBranch(m.session.CWD), m.session.GetSessionName(), modelName, m.thinkingLevel, provider)
	m.footer.SetHasReasoning(supportsThinking(modelName))
	m.footer.SetEntryCount(len(m.session.Entries))
	m.input.SetBorderColor(m.theme.ThinkingBorderColor(m.thinkingLevel))
}


func (m *AppModel) getCyclableModels() []string {
	if len(m.scopedModels) > 0 {
		// Use modelOrder if available for preferred cycling order
		var models []string
		if len(m.modelOrder) > 0 {
			for _, mdl := range m.modelOrder {
				if m.scopedModels[mdl] {
					models = append(models, mdl)
				}
			}
		} else {
			for _, mdl := range m.availableModels {
				if m.scopedModels[mdl] {
					models = append(models, mdl)
				}
			}
		}
		if len(models) > 0 {
			return models
		}
	}
	// Use modelOrder for all models if available
	if len(m.modelOrder) > 0 {
		return m.modelOrder
	}
	return m.availableModels
}

// switchToModel switches the agent to the specified model (from model selector).
func (m *AppModel) switchToModel(model string) {
	defer m.setTerminalTitle()
	// Also re-resolve provider (base URL, API key) for the new model
	if err := m.agent.Engine().SwitchModel(model); err != nil {
		m.chat.AppendSystem("Failed to switch model: " + err.Error())
		return
	}
	// Update model index
	for i, m2 := range m.availableModels {
		if m2 == model {
			m.modelIndex = i
			break
		}
	}
	modelName, provider := parseModelString(model)
	m.footer.SetSession(m.session.CWD, getGitBranch(m.session.CWD), m.session.GetSessionName(), modelName, m.thinkingLevel, provider)
	m.footer.SetHasReasoning(supportsThinking(modelName))
	m.footer.SetEntryCount(len(m.session.Entries))
	m.input.SetBorderColor(m.theme.ThinkingBorderColor(m.thinkingLevel))
	msg := "Switched to " + modelName
	if m.thinkingLevel != "" && m.thinkingLevel != "off" {
		msg += " (thinking: " + m.thinkingLevel + ")"
	}
	m.chat.AppendSystem(msg)
}


// showSessionTree opens an interactive session tree viewer (TS pi-mono: /tree).
// Supports fold/unfold, filter modes, search, and active path highlighting.
