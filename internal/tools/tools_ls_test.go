package tools

import (
	"encoding/json"
	"strings"
	"testing"
)

func TestLsTool(t *testing.T) {
	tool := LsTool()

	// Test with a known directory that exists
	args, _ := json.Marshal(map[string]string{"path": "."})
	result, err := tool.Handler(args)
	if err != nil {
		t.Fatalf("LsTool failed: %v", err)
	}
	if !strings.Contains(result, "tools.go") {
		t.Errorf("expected tools.go in listing, got: %s", result)
	}
	if !strings.Contains(result, ".go") {
		t.Errorf("expected .go files in listing: %s", result)
	}
	t.Logf("Ls output:\n%s", result)
}

func TestFindTool(t *testing.T) {
	tool := FindTool()

	// Find all .go files
	args, _ := json.Marshal(map[string]string{"pattern": "*.go", "path": "."})
	result, err := tool.Handler(args)
	if err != nil {
		t.Fatalf("FindTool failed: %v", err)
	}
	if !strings.Contains(result, "tools.go") {
		t.Errorf("expected tools.go in results, got: %s", result)
	}
	if !strings.Contains(result, "tools_ls.go") {
		t.Errorf("expected tools_ls.go in results, got: %s", result)
	}
	if !strings.Contains(result, "tools_find.go") {
		t.Errorf("expected tools_find.go in results, got: %s", result)
	}
	t.Logf("Find output:\n%s", result)
}

func TestFindToolSkipsGit(t *testing.T) {
	tool := FindTool()

	// Find should skip .git directories
	args, _ := json.Marshal(map[string]string{"pattern": "*", "path": "."})
	result, err := tool.Handler(args)
	if err != nil {
		t.Fatalf("FindTool failed: %v", err)
	}
	if strings.Contains(result, ".git/") {
		t.Errorf("find should skip .git dirs, but found .git in: %s", result)
	}
	t.Logf("Find root output (first 500 chars):\n%.500s", result)
}

func TestLsNoPath(t *testing.T) {
	tool := LsTool()

	// Test with empty args (default path)
	result, err := tool.Handler([]byte("{}"))
	if err != nil {
		t.Fatalf("LsTool with empty args failed: %v", err)
	}
	if result == "" {
		t.Error("expected non-empty output for default path")
	}
	t.Logf("Ls default path output:\n%s", result)
}
