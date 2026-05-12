// Package config provides resource configuration types and helpers for xihu.
package config

import (
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/huichen/xihu/internal/settings"
	"github.com/huichen/xihu/internal/skills"
)

// ResourceType identifies the kind of resource being managed.
type ResourceType string

const (
	ResourceSkills     ResourceType = "skills"
	ResourceExtensions ResourceType = "extensions"
	ResourcePrompts    ResourceType = "prompts"
	ResourceThemes     ResourceType = "themes"
)

// ResourceLabels maps resource types to display labels.
var ResourceLabels = map[ResourceType]string{
	ResourceSkills:     "Skills",
	ResourceExtensions: "Extensions",
	ResourcePrompts:    "Prompts",
	ResourceThemes:     "Themes",
}

// ResourceItem represents a single discovered resource with its enabled state.
type ResourceItem struct {
	Path        string
	Enabled     bool
	Name        string
	Type        ResourceType
	Scope       string
	GroupKey    string
	SubgroupKey string
}

// ResourceGroup groups items by origin/scope.
type ResourceGroup struct {
	Key       string
	Label     string
	Scope     string
	Subgroups []ResourceSubgroup
}

// ResourceSubgroup groups items by resource type within a group.
type ResourceSubgroup struct {
	Type  ResourceType
	Label string
	Items []ResourceItem
}

// ResolveResources discovers all resources across all configured directories
// and returns them grouped by scope and type.
func ResolveResources(cwd string, globalSettings, projectSettings *settings.Settings) ([]ResourceGroup, []ResourceItem) {
	home, _ := os.UserHomeDir()

	type discoveryDir struct {
		path  string
		scope string
		group string
	}

	skillDirs := []discoveryDir{
		{filepath.Join(home, ".xihu", "skills"), "user", "User (~/.xihu/)"},
		{filepath.Join(cwd, ".xihu", "skills"), "project", "Project (.xihu/)"},
		{filepath.Join(home, ".agents", "skills"), "agents", "Agents (~/.agents/)"},
	}

	extDirs := []discoveryDir{
		{filepath.Join(home, ".xihu", "extensions"), "user", "User (~/.xihu/)"},
		{filepath.Join(cwd, ".xihu", "extensions"), "project", "Project (.xihu/)"},
	}

	promptDirs := []discoveryDir{
		{filepath.Join(home, ".xihu", "prompts"), "user", "User (~/.xihu/)"},
		{filepath.Join(cwd, ".xihu", "prompts"), "project", "Project (.xihu/)"},
	}

	themeDirs := []discoveryDir{
		{filepath.Join(home, ".xihu", "themes"), "user", "User (~/.xihu/)"},
		{filepath.Join(cwd, ".xihu", "themes"), "project", "Project (.xihu/)"},
	}

	var allItems []ResourceItem

	for _, d := range skillDirs {
		items := discoverSkillDir(d.path, d.scope, d.group, globalSettings, projectSettings)
		allItems = append(allItems, items...)
	}

	for _, d := range extDirs {
		items := discoverFileDir(d.path, d.scope, d.group, ResourceExtensions, globalSettings, projectSettings)
		allItems = append(allItems, items...)
	}

	for _, d := range promptDirs {
		items := discoverFileDir(d.path, d.scope, d.group, ResourcePrompts, globalSettings, projectSettings)
		allItems = append(allItems, items...)
	}

	for _, d := range themeDirs {
		items := discoverFileDir(d.path, d.scope, d.group, ResourceThemes, globalSettings, projectSettings)
		allItems = append(allItems, items...)
	}

	// Build groups
	groupMap := make(map[string]*ResourceGroup)
	for _, item := range allItems {
		gk := item.GroupKey
		if _, exists := groupMap[gk]; !exists {
			groupMap[gk] = &ResourceGroup{
				Key:   gk,
				Label: item.Scope,
				Scope: item.Scope,
			}
		}
		group := groupMap[gk]

		var sg *ResourceSubgroup
		for i := range group.Subgroups {
			if group.Subgroups[i].Type == item.Type {
				sg = &group.Subgroups[i]
				break
			}
		}
		if sg == nil {
			group.Subgroups = append(group.Subgroups, ResourceSubgroup{
				Type:  item.Type,
				Label: ResourceLabels[item.Type],
			})
			sg = &group.Subgroups[len(group.Subgroups)-1]
		}
		sg.Items = append(sg.Items, item)
	}

	groups := make([]ResourceGroup, 0, len(groupMap))
	for _, g := range groupMap {
		typeOrder := map[ResourceType]int{
			ResourceExtensions: 0,
			ResourceSkills:     1,
			ResourcePrompts:    2,
			ResourceThemes:     3,
		}
		sort.Slice(g.Subgroups, func(i, j int) bool {
			return typeOrder[g.Subgroups[i].Type] < typeOrder[g.Subgroups[j].Type]
		})
		for i := range g.Subgroups {
			sort.Slice(g.Subgroups[i].Items, func(a, b int) bool {
				return g.Subgroups[i].Items[a].Name < g.Subgroups[i].Items[b].Name
			})
		}
		groups = append(groups, *g)
	}

	scopeOrder := map[string]int{"user": 0, "project": 1, "agents": 2}
	sort.Slice(groups, func(i, j int) bool {
		return scopeOrder[groups[i].Scope] < scopeOrder[groups[j].Scope]
	})

	return groups, allItems
}

func discoverSkillDir(dir, scope, groupLabel string, globalSettings, projectSettings *settings.Settings) []ResourceItem {
	dir = ExpandHome(dir)
	var items []ResourceItem

	entries, err := os.ReadDir(dir)
	if err != nil {
		return items
	}

	for _, entry := range entries {
		if !entry.IsDir() {
			continue
		}
		skillPath := filepath.Join(dir, entry.Name(), "SKILL.md")
		if _, err := os.Stat(skillPath); err != nil {
			continue
		}

		skill, ok := skills.ParseSkillFile(skillPath, scope)
		if !ok || skill.Name == "" {
			continue
		}

		enabled := isResourceEnabled(skillPath, ResourceSkills, globalSettings, projectSettings, scope)
		items = append(items, ResourceItem{
			Path:     skillPath,
			Enabled:  enabled,
			Name:     skill.Name,
			Type:     ResourceSkills,
			Scope:    scope,
			GroupKey: fmt.Sprintf("%s:%s", scope, groupLabel),
		})
	}
	return items
}

func discoverFileDir(dir, scope, groupLabel string, resType ResourceType, globalSettings, projectSettings *settings.Settings) []ResourceItem {
	dir = ExpandHome(dir)
	var items []ResourceItem

	entries, err := os.ReadDir(dir)
	if err != nil {
		return items
	}

	for _, entry := range entries {
		if entry.IsDir() {
			continue
		}

		name := entry.Name()
		ext := strings.ToLower(filepath.Ext(name))
		if resType == ResourceThemes && ext != ".json" {
			continue
		}

		fullPath := filepath.Join(dir, name)
		enabled := isResourceEnabled(fullPath, resType, globalSettings, projectSettings, scope)
		displayName := strings.TrimSuffix(name, ext)

		items = append(items, ResourceItem{
			Path:     fullPath,
			Enabled:  enabled,
			Name:     displayName,
			Type:     resType,
			Scope:    scope,
			GroupKey: fmt.Sprintf("%s:%s", scope, groupLabel),
		})
	}
	return items
}

func isResourceEnabled(path string, resType ResourceType, globalSettings, projectSettings *settings.Settings, scope string) bool {
	var s *settings.Settings
	if scope == "project" {
		s = projectSettings
	} else {
		s = globalSettings
	}
	if s == nil {
		return true
	}

	var patterns []string
	switch resType {
	case ResourceSkills:
		patterns = s.Skills
	case ResourceExtensions:
		patterns = s.Extensions
	case ResourcePrompts:
		patterns = s.Prompts
	case ResourceThemes:
		patterns = s.Themes
	default:
		return true
	}

	if len(patterns) == 0 {
		return true
	}

	// Generate relative path pattern for this resource
	resourcePattern := getResourcePattern(path, scope)

	for _, p := range patterns {
		prefix := ""
		pattern := p
		if strings.HasPrefix(p, "+") || strings.HasPrefix(p, "-") || strings.HasPrefix(p, "!") {
			prefix = p[:1]
			pattern = p[1:]
		}

		if matchPattern(resourcePattern, pattern) {
			if prefix == "-" || prefix == "!" {
				return false
			}
			return true
		}
	}

	return false
}

func matchPattern(resourcePath, pattern string) bool {
	// Exact match
	if resourcePath == pattern {
		return true
	}
	// Glob match (e.g., skills/*/SKILL.md)
	matched, err := filepath.Match(pattern, resourcePath)
	if err == nil && matched {
		return true
	}
	// Match by filename only
	if filepath.Base(resourcePath) == pattern {
		return true
	}
	// Match by directory name (for skills with SKILL.md)
	if filepath.Base(resourcePath) == "SKILL.md" && filepath.Base(filepath.Dir(resourcePath)) == pattern {
		return true
	}
	// Substring match for partial paths
	if strings.Contains(resourcePath, pattern) {
		return true
	}
	return false
}

// ExpandHome expands a leading ~ to the user's home directory.
func ExpandHome(path string) string {
	if strings.HasPrefix(path, "~/") || path == "~" {
		home, err := os.UserHomeDir()
		if err != nil {
			return path
		}
		if path == "~" {
			return home
		}
		return filepath.Join(home, path[2:])
	}
	return path
}

// ToggleResource updates the settings to enable or disable a resource.
// Uses relative path patterns matching pi's getResourcePattern behavior.
func ToggleResource(path string, resType ResourceType, scope string, enabled bool) error {
	globalPath, projectPath := settings.GetDefaultPaths()
	var targetPath string
	if scope == "project" {
		targetPath = projectPath
	} else {
		targetPath = globalPath
	}

	s, err := settings.LoadSettings(targetPath)
	if err != nil {
		return fmt.Errorf("load settings: %w", err)
	}

	var patterns []string
	switch resType {
	case ResourceSkills:
		patterns = s.Skills
	case ResourceExtensions:
		patterns = s.Extensions
	case ResourcePrompts:
		patterns = s.Prompts
	case ResourceThemes:
		patterns = s.Themes
	}

	// Generate relative path pattern (matching pi's getResourcePattern)
	pattern := getResourcePattern(path, scope)

	// Remove existing patterns for this resource (support +, -, ! prefixes)
	updated := make([]string, 0, len(patterns))
	for _, p := range patterns {
		stripped := p
		if strings.HasPrefix(p, "+") || strings.HasPrefix(p, "-") || strings.HasPrefix(p, "!") {
			stripped = p[1:]
		}
		if stripped != pattern {
			updated = append(updated, p)
		}
	}

	if enabled {
		updated = append(updated, "+"+pattern)
	} else {
		updated = append(updated, "-"+pattern)
	}

	switch resType {
	case ResourceSkills:
		s.Skills = updated
	case ResourceExtensions:
		s.Extensions = updated
	case ResourcePrompts:
		s.Prompts = updated
	case ResourceThemes:
		s.Themes = updated
	}

	return settings.SaveSettings(targetPath, s)
}

// getResourcePattern returns the relative path pattern for a resource,
// matching pi's getResourcePattern behavior.
func getResourcePattern(path string, scope string) string {
	home, _ := os.UserHomeDir()
	var baseDir string
	if scope == "project" {
		cwd, _ := os.Getwd()
		baseDir = filepath.Join(cwd, ".xihu")
	} else {
		baseDir = filepath.Join(home, ".xihu")
	}
	rel, err := filepath.Rel(baseDir, path)
	if err != nil {
		if filepath.Base(path) == "SKILL.md" {
			return filepath.Base(filepath.Dir(path))
		}
		return filepath.Base(path)
	}
	return rel
}
