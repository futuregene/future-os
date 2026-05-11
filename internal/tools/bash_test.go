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
	if !strings.Contains(result, "hello") {
		t.Errorf("expected output to contain 'hello', got: %s", result)
	}
	if strings.Contains(result, "exit code:") {
		t.Errorf("should not contain exit code header (TS pi-mono aligned), got: %s", result)
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
	_, err := tool.Handler(args)
	if err == nil {
		t.Fatal("expected error for timeout, got nil")
	}
	if !strings.Contains(err.Error(), "timed out") {
		t.Errorf("expected 'timed out' in error, got: %v", err)
	}
	fmt.Println("=== TestBashToolTimeout ===")
	fmt.Println(err)
}

func TestBashToolNonZeroExit(t *testing.T) {
	tool := BashTool()
	args, _ := json.Marshal(map[string]interface{}{"command": "exit 42"})
	_, err := tool.Handler(args)
	if err == nil {
		t.Fatal("expected error for nonzero exit, got nil")
	}
	if !strings.Contains(err.Error(), "exited with code 42") {
		t.Errorf("expected 'exited with code 42' in error, got: %v", err)
	}
	fmt.Println("=== TestBashToolNonZeroExit ===")
	fmt.Println(err)
}

func TestBashToolLargeOutputSpill(t *testing.T) {
	tool := BashTool()
	args, _ := json.Marshal(map[string]interface{}{
		"command": "python3 -c \"for i in range(2000): print('x' * 80)\" 2>/dev/null || yes 'aaaaaaaaaa' | head -2000",
	})
	result, err := tool.Handler(args)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	// Output should be capped at 50000 bytes
	if len(result) > 51000 {
		t.Errorf("result too long: %d bytes (expected <= ~51000)", len(result))
	}
	fmt.Printf("Total result length: %d bytes\n", len(result))
}

func TestBashToolNoTimeout(t *testing.T) {
	tool := BashTool()
	args, _ := json.Marshal(map[string]interface{}{"command": "echo no_timeout_works"})
	result, err := tool.Handler(args)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if !strings.Contains(result, "no_timeout_works") {
		t.Errorf("expected output to contain 'no_timeout_works', got: %s", result)
	}
	fmt.Println("=== TestBashToolNoTimeout ===")
	fmt.Println(result)
}
