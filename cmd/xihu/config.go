package main

import (
	"fmt"
	"os"

	"github.com/huichen/xihu/internal/config"
	"github.com/huichen/xihu/internal/settings"
	"github.com/huichen/xihu/internal/tui"
)

// runConfigCommand handles the "xihu config" subcommand.
func runConfigCommand(cwd string) {
	// Load settings
	globalPath, projectPath := settings.GetDefaultPaths()
	globalSettings, gErr := settings.LoadSettings(globalPath)
	if gErr != nil {
		fmt.Fprintf(os.Stderr, "Warning: could not load global settings: %v\n", gErr)
		globalSettings = &settings.Settings{}
	}
	projectSettings, pErr := settings.LoadSettings(projectPath)
	if pErr != nil {
		fmt.Fprintf(os.Stderr, "Warning: could not load project settings: %v\n", pErr)
		projectSettings = &settings.Settings{}
	}

	// Resolve resources
	groups, allItems := config.ResolveResources(cwd, globalSettings, projectSettings)

	if len(allItems) == 0 {
		fmt.Println("No resources found.")
		fmt.Println()
		fmt.Println("Resources are discovered from:")
		fmt.Println("  ~/.xihu/skills/      User skills")
		fmt.Println("  .xihu/skills/        Project skills")
		fmt.Println("  ~/.agents/skills/    Agents skills")
		fmt.Println()
		fmt.Println("Create SKILL.md files in these directories to add skills.")
		os.Exit(0)
	}

	// Run the config selector TUI
	if err := tui.RunConfigSelector(groups, allItems); err != nil {
		fmt.Fprintf(os.Stderr, "Error: %v\n", err)
		os.Exit(1)
	}
}
