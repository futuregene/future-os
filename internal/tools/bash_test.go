package tools

import (
	"encoding/json"
	"fmt"
	"strings"
	"testing"
)

func TestBashToolNormal(t *testing.T) {
	tool := BashTool()
	args, _ := json.Marshal(map[string]interface{}{"command": "echo hello"})
	result, err := tool.Handler(args)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if !strings.Contains(result, "exit code: 0") {
		t.Errorf("expected exit code 0, got: %s", result)
	}
	if !strings.Contains(result, "hello") {
		t.Errorf("expected output to contain 'hello', got: %s", result)
	}
	fmt.Println("=== TestBashToolNormal ===")
	fmt.Println(result)
}

func TestBashToolTimeout(t *testing.T) {
	tool := BashTool()
	args, _ := json.Marshal(map[string]interface{}{
		"command": "sleep 3; echo done",
		"timeout": 2,
	})
	result, err := tool.Handler(args)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if !strings.Contains(result, "exit code: -1") {
		t.Errorf("expected exit code -1 for timeout, got: %s", result)
	}
	fmt.Println("=== TestBashToolTimeout ===")
	fmt.Println(result)
}

func TestBashToolNonZeroExit(t *testing.T) {
	tool := BashTool()
	args, _ := json.Marshal(map[string]interface{}{"command": "exit 42"})
	result, err := tool.Handler(args)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if !strings.Contains(result, "exit code: 42") {
		t.Errorf("expected exit code 42, got: %s", result)
	}
	fmt.Println("=== TestBashToolNonZeroExit ===")
	fmt.Println(result)
}

func TestBashToolLargeOutputSpill(t *testing.T) {
	tool := BashTool()
	// Generate ~160K of output (exceeds both spillThreshold=10K and tailBytes=50K)
	args, _ := json.Marshal(map[string]interface{}{
		"command": "python3 -c \"for i in range(2000): print('x' * 80)\" 2>/dev/null || yes 'aaaaaaaaaa' | head -2000",
	})
	result, err := tool.Handler(args)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	fmt.Println("=== TestBashToolLargeOutputSpill ===")
	// Show first few lines
	lines := strings.SplitN(result, "\n", 6)
	for _, l := range lines {
		fmt.Println(l)
	}
	if !strings.Contains(result, "exit code: 0") {
		t.Errorf("expected exit code 0, got first line: %s", strings.SplitN(result, "\n", 2)[0])
	}
	if !strings.Contains(result, "[full output at") {
		t.Errorf("expected spill file path, got: %s", result[:200])
	}
	// Output should be capped at 50000 bytes + overhead
	if len(result) > 51000 {
		t.Errorf("result too long: %d bytes (expected <= ~51000)", len(result))
	}
	fmt.Printf("Total result length: %d bytes\n", len(result))
}

func TestBashToolNoTimeout(t *testing.T) {
	// Verify that when timeout is not specified, the command still runs (no default timeout)
	tool := BashTool()
	args, _ := json.Marshal(map[string]interface{}{"command": "echo no_timeout_works"})
	result, err := tool.Handler(args)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if !strings.Contains(result, "exit code: 0") {
		t.Errorf("expected exit code 0, got: %s", result)
	}
	fmt.Println("=== TestBashToolNoTimeout ===")
	fmt.Println(result)
}
