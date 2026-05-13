//go:build ignore

package main

import (
	"fmt"
	"github.com/huichen/xihu/internal/settings"
)

func main() {
	cfg, err := settings.LoadAll()
	if err != nil {
		fmt.Println("Error:", err)
		return
	}
	fmt.Printf("Model: %s\n", cfg.DefaultModel)
	fmt.Printf("Provider: %s\n", cfg.DefaultProvider)
	fmt.Printf("Thinking: %s\n", cfg.DefaultThinkingLevel)
	fmt.Printf("Theme: %s\n", cfg.Theme)
}
