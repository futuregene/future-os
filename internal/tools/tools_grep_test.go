package tools

import (
	"encoding/json"
	"fmt"
	"testing"
)

func TestGrepToolBasic(t *testing.T) {
	gt := GrepTool()

	// Test 1: basic search
	t.Run("basic", func(t *testing.T) {
		args, _ := json.Marshal(map[string]interface{}{
			"pattern": "GrepTool",
			"path":    ".",
			"glob":    "*.go",
		})
		result, err := gt.Handler(args)
		if err != nil {
			t.Fatalf("Error: %v", err)
		}
		if result == "No matches found\n" {
			t.Fatal("Expected matches but got none")
		}
		fmt.Println("=== basic ===")
		fmt.Print(result)
	})

	// Test 2: ignoreCase
	t.Run("ignoreCase", func(t *testing.T) {
		args, _ := json.Marshal(map[string]interface{}{
			"pattern":    "greptool",
			"path":       ".",
			"glob":       "*.go",
			"ignoreCase": true,
		})
		result, err := gt.Handler(args)
		if err != nil {
			t.Fatalf("Error: %v", err)
		}
		fmt.Println("=== ignoreCase ===")
		fmt.Print(result)
	})

	// Test 3: literal
	t.Run("literal", func(t *testing.T) {
		args, _ := json.Marshal(map[string]interface{}{
			"pattern": "GrepTool()",
			"path":    ".",
			"glob":    "*.go",
			"literal": true,
		})
		result, err := gt.Handler(args)
		if err != nil {
			t.Fatalf("Error: %v", err)
		}
		fmt.Println("=== literal ===")
		fmt.Print(result)
	})

	// Test 4: context
	t.Run("context", func(t *testing.T) {
		args, _ := json.Marshal(map[string]interface{}{
			"pattern": "GrepTool",
			"path":    "tools.go",
			"context": 2,
		})
		result, err := gt.Handler(args)
		if err != nil {
			t.Fatalf("Error: %v", err)
		}
		fmt.Println("=== context ===")
		fmt.Print(result)
	})

	// Test 5: limit
	t.Run("limit", func(t *testing.T) {
		args, _ := json.Marshal(map[string]interface{}{
			"pattern": "func",
			"path":    "tools.go",
			"limit":   3,
		})
		result, err := gt.Handler(args)
		if err != nil {
			t.Fatalf("Error: %v", err)
		}
		fmt.Println("=== limit ===")
		fmt.Print(result)
	})

	// Test 6: no match
	t.Run("no_match", func(t *testing.T) {
		args, _ := json.Marshal(map[string]interface{}{
			"pattern": "XYZZY_NO_MATCH_PATTERN_999",
			"path":    "tools.go", // search only in tools.go, avoid test file
		})
		result, err := gt.Handler(args)
		if err != nil {
			t.Fatalf("Error: %v", err)
		}
		if result != "No matches found\n" {
			t.Fatalf("Expected 'No matches found' but got: %s", result)
		}
		fmt.Println("=== no_match (OK) ===")
	})

	// Test 7: limit overflow note
	t.Run("limit_overflow", func(t *testing.T) {
		args, _ := json.Marshal(map[string]interface{}{
			"pattern": ".",
			"path":    "tools.go",
			"limit":   3,
		})
		result, err := gt.Handler(args)
		if err != nil {
			t.Fatalf("Error: %v", err)
		}
		fmt.Println("=== limit_overflow ===")
		fmt.Print(result)
	})
}
