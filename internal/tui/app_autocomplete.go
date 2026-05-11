// Package tui provides the interactive terminal UI for xihu using Bubble Tea.
package tui

import (

	"encoding/json"
	"strings"


	"github.com/huichen/xihu/internal/tui/components"


)

// ─── Message Types ─────────────────────────────────────────────────────────

// StreamTextMsg is a chunk of streamed text from the LLM.

func fuzzyMatchScore(query, text string) (bool, float64) {
	ql := strings.ToLower(query)
	tl := strings.ToLower(text)
	if len(ql) == 0 {
		return true, 0
	}
	if len(ql) > len(tl) {
		return false, 0
	}
	qi := 0
	score := 0.0
	lastMatch := -1
	consecutive := 0
	for i := 0; i < len(tl) && qi < len(ql); i++ {
		if tl[i] == ql[qi] {
			if lastMatch == i-1 {
				consecutive++
				score -= float64(consecutive) * 5
			} else {
				consecutive = 0
				if lastMatch >= 0 {
					score += float64(i-lastMatch-1) * 2
				}
			}
			// Reward word boundaries
			if i == 0 || tl[i-1] == '/' || tl[i-1] == '-' || tl[i-1] == '_' || tl[i-1] == ' ' {
				score -= 10
			}
			score += float64(i) * 0.1
			lastMatch = i
			qi++
		}
	}
	if qi < len(ql) {
		return false, 0
	}
	if ql == tl {
		score -= 100
	}
	return true, score
}

// getArgCompletions returns argument-specific completions for a slash command.
// (TS pi-mono: SlashCommand.getArgumentCompletions)
func (m *AppModel) getArgCompletions(cmdName, argPrefix string) []components.SlashCommand {
	switch cmdName {
	case "model":
		// Complete model names
		var results []components.SlashCommand
		for _, model := range m.availableModels {
			if argPrefix == "" || strings.HasPrefix(strings.ToLower(model), strings.ToLower(argPrefix)) {
				results = append(results, components.SlashCommand{
					Name:        model,
					Description: "switch to model",
				})
			}
		}
		return results

	case "theme":
		// Complete theme names
		themes := []string{"dark", "light"}
		customPaths, _ := DiscoverThemes("")
		for _, p := range customPaths {
			t, err := LoadTheme(p)
			if err == nil && t.Name != "" && t.Name != "dark" && t.Name != "light" {
				themes = append(themes, t.Name)
			}
		}
		var results []components.SlashCommand
		for _, t := range themes {
			if argPrefix == "" || strings.HasPrefix(strings.ToLower(t), strings.ToLower(argPrefix)) {
				results = append(results, components.SlashCommand{
					Name:        t,
					Description: "apply theme",
				})
			}
		}
		return results

	case "thinking":
		levels := []string{"off", "minimal", "low", "medium", "high", "xhigh"}
		var results []components.SlashCommand
		for _, l := range levels {
			if argPrefix == "" || strings.HasPrefix(l, strings.ToLower(argPrefix)) {
				results = append(results, components.SlashCommand{
					Name:        l,
					Description: "set thinking level",
				})
			}
		}
		return results

	case "scoped-models":
		// Subcommands: enable, disable, clear, list
		subs := []struct{ name, desc string }{
			{"enable", "enable a model for cycling"},
			{"disable", "disable a model from cycling"},
			{"clear", "clear all scoped models"},
			{"list", "list current scoped models"},
		}
		// Check if there's another space (subcommand already typed)
		if subIdx := strings.Index(argPrefix, " "); subIdx >= 0 {
			sub := strings.ToLower(argPrefix[:subIdx])
			modelPrefix := argPrefix[subIdx+1:]
			if sub == "enable" || sub == "disable" {
				var results []components.SlashCommand
				for _, model := range m.availableModels {
					if modelPrefix == "" || strings.HasPrefix(strings.ToLower(model), strings.ToLower(modelPrefix)) {
						results = append(results, components.SlashCommand{
							Name:        model,
							Description: sub + " model",
						})
					}
				}
				return results
			}
		}
		var results []components.SlashCommand
		for _, s := range subs {
			if argPrefix == "" || strings.HasPrefix(s.name, strings.ToLower(argPrefix)) {
				results = append(results, components.SlashCommand{
					Name:        s.name,
					Description: s.desc,
				})
			}
		}
		return results

	case "fork":
		// Complete entry IDs from current session
		var results []components.SlashCommand
		if m.session != nil {
			for _, entry := range m.session.Entries {
				if argPrefix == "" || strings.HasPrefix(strings.ToLower(entry.ID), strings.ToLower(argPrefix)) {
					role := entry.Role
					if role == "" {
						role = entry.Type
					}
					results = append(results, components.SlashCommand{
						Name:        entry.ID,
						Description: role + " entry",
					})
				}
			}
		}
		return results

	case "export":
		// No specific argument completions - file path is free text
		return nil

	case "name":
		// No specific argument - free text
		return nil

	case "resume":
		// Complete session IDs
		var results []components.SlashCommand
		if m.sessMgr != nil {
			sessions, err := m.sessMgr.List(m.session.CWD)
			if err == nil {
				for _, sess := range sessions {
					if argPrefix == "" || strings.HasPrefix(strings.ToLower(sess.ID), strings.ToLower(argPrefix)) {
						name := sess.GetSessionName()
						if name == "" {
							name = sess.ID
						}
						results = append(results, components.SlashCommand{
							Name:        sess.ID,
							Description: name,
						})
					}
				}
			}
		}
		return results
	}

	return nil
}

// filterSlashCandidates filters SlashCommandsWithDesc by fuzzy-matched prefix.
func (m *AppModel) filterSlashCandidates(prefix string) []components.SlashCommand {
	all := components.SlashCommandsWithDesc()
	if prefix == "" {
		return all
	}
	type match struct {
		sc    components.SlashCommand
		score float64
	}
	var matches []match
	for _, sc := range all {
		name := sc.Name
		if strings.HasPrefix(name, "/") {
			name = name[1:]
		}
		if ok, score := fuzzyMatchScore(prefix, name); ok {
			matches = append(matches, match{sc: sc, score: score})
		}
	}
	// Sort by score (lower = better), then alphabetically
	for i := 0; i < len(matches); i++ {
		for j := i + 1; j < len(matches); j++ {
			if matches[i].score > matches[j].score ||
				(matches[i].score == matches[j].score && matches[i].sc.Name > matches[j].sc.Name) {
				matches[i], matches[j] = matches[j], matches[i]
			}
		}
	}
	result := make([]components.SlashCommand, len(matches))
	for i, m := range matches {
		result[i] = m.sc
	}
	return result
}

// Symbol represents a symbol/tag suggestion for # autocomplete mode.
type Symbol struct {
	Name        string
	Description string
}

// collectSymbols gathers symbols from recent session entries and context.
// (TS pi-mono: # symbol mode collects file paths, entry IDs, and tagged references)
func (m *AppModel) collectSymbols(prefix string) []Symbol {
	var symbols []Symbol
	seen := make(map[string]bool)

	// Collect file paths from recent session entries
	if m.session != nil {
		for _, entry := range m.session.Entries {
			var contentBlocks []struct {
				Type string `json:"type"`
				Text string `json:"text"`
			}
			if err := json.Unmarshal(entry.Content, &contentBlocks); err != nil {
				continue
			}
			for _, block := range contentBlocks {
				if block.Type == "text" && block.Text != "" {
					for _, word := range strings.Fields(block.Text) {
						trimmers := "`'\"()[]{}<>,"
						word = strings.Trim(word, trimmers)
						if strings.Contains(word, "/") || strings.Contains(word, ".") {
							if len(word) > 2 && len(word) < 120 && !seen[word] {
								if prefix == "" || strings.HasPrefix(strings.ToLower(word), strings.ToLower(prefix)) {
									symbols = append(symbols, Symbol{Name: word, Description: "referenced path"})
									seen[word] = true
								}
							}
						}
					}
				}
			}
		}
	}

	// Collect entry IDs for reference
	if m.session != nil {
		for _, entry := range m.session.Entries {
			if prefix == "" || strings.HasPrefix(strings.ToLower(entry.ID), strings.ToLower(prefix)) {
				role := entry.Role
				if role == "" {
					role = entry.Type
				}
				if !seen[entry.ID] {
					symbols = append(symbols, Symbol{Name: entry.ID, Description: role + " entry"})
					seen[entry.ID] = true
				}
			}
		}
	}

	return symbols
}

// updateAutocomplete updates the autocomplete component with candidate views.
func (m *AppModel) updateAutocomplete(candidates []components.SlashCommand, prefix string) {
	if len(candidates) == 0 {
		m.autocomplete.Hide()
		return
	}
	names := components.SlashCommandNames(candidates)
	descs := components.SlashCommandDescriptions(candidates)
	m.autocomplete.Show(names, descs, prefix)
}

// ─── Agent Integration ─────────────────────────────────────────────────────

// runAgent sends user input to the agent loop in a goroutine.
// It subscribes to the EventBus to receive thinking/tool/usage events
// and forwards them to the Bubble Tea program via Program.Send.
// myID is the stream identifier snapshot — events are dropped if streamID no longer matches.
