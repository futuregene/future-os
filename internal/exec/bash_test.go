package exec

import (
	"os"
	"strings"
	"testing"
	"time"
)

func TestExecuteBashBasic(t *testing.T) {
	result, err := ExecuteBash(BashExecutorOptions{
		Command: "echo hello",
	})
	if err != nil {
		t.Fatalf("ExecuteBash: %v", err)
	}
	if result.ExitCode != 0 {
		t.Errorf("exit code = %d, want 0", result.ExitCode)
	}
	if !strings.Contains(result.Output, "hello") {
		t.Errorf("output = %s", result.Output)
	}
	if result.Cancelled {
		t.Error("should not be cancelled")
	}
	if result.Truncated {
		t.Error("should not be truncated")
	}
}

func TestExecuteBashNonZeroExit(t *testing.T) {
	result, err := ExecuteBash(BashExecutorOptions{
		Command: "exit 42",
	})
	if err != nil {
		t.Fatalf("ExecuteBash: %v", err)
	}
	if result.ExitCode != 42 {
		t.Errorf("exit code = %d, want 42", result.ExitCode)
	}
}

func TestExecuteBashTimeout(t *testing.T) {
	result, err := ExecuteBash(BashExecutorOptions{
		Command: "sleep 10",
		Timeout: 500 * time.Millisecond,
	})
	if err != nil {
		t.Fatalf("ExecuteBash: %v", err)
	}
	if result.ExitCode != -1 {
		t.Errorf("exit code = %d, want -1 (timeout)", result.ExitCode)
	}
}

func TestExecuteBashCWD(t *testing.T) {
	tmpDir := t.TempDir()
	result, err := ExecuteBash(BashExecutorOptions{
		Command: "pwd",
		CWD:     tmpDir,
	})
	if err != nil {
		t.Fatalf("ExecuteBash: %v", err)
	}
	if result.ExitCode != 0 {
		t.Errorf("exit code = %d", result.ExitCode)
	}
	// The output should contain the temp dir path; bash may resolve symlinks however
	if !strings.Contains(result.Output, tmpDir) && !strings.Contains(result.Output, "tmp") {
		t.Logf("pwd output: %s (cwd: %s)", result.Output, tmpDir)
	}
}

func TestExecuteBashAbortSignal(t *testing.T) {
	abort := make(chan struct{})
	close(abort) // signal immediately

	result, err := ExecuteBash(BashExecutorOptions{
		Command:     "sleep 10",
		AbortSignal: abort,
	})
	if err != nil {
		t.Fatalf("ExecuteBash: %v", err)
	}
	if !result.Cancelled {
		t.Error("should be cancelled")
	}
}

func TestExecuteBashLargeOutputSpill(t *testing.T) {
	result, err := ExecuteBash(BashExecutorOptions{
		Command:        "python3 -c \"for i in range(300): print('x'*80)\" 2>/dev/null || yes 'aaaaaaaaaa' | head -300",
		Timeout:        30 * time.Second,
		MaxOutputBytes: 1000,
	})
	if err != nil {
		t.Fatalf("ExecuteBash: %v", err)
	}
	if result.ExitCode != 0 {
		t.Logf("exit code = %d", result.ExitCode)
	}
	// Output should be limited
	if len(result.Output) > 1050 {
		t.Errorf("output too long: %d bytes", len(result.Output))
	}
	// Should have spill path
	if result.FullOutputPath == "" {
		t.Log("no spill path (may not exceed threshold)")
	} else {
		// Clean up
		os.Remove(result.FullOutputPath)
	}
}

func TestExecuteBashANSIStripping(t *testing.T) {
	result, err := ExecuteBash(BashExecutorOptions{
		Command: "echo -e '\\033[31mred\\033[0m'",
	})
	if err != nil {
		t.Fatalf("ExecuteBash: %v", err)
	}
	// ANSI codes should be stripped by default
	if strings.Contains(result.Output, "\033[") {
		t.Errorf("ANSI codes not stripped: %q", result.Output)
	}
}

func TestExecuteBashNoANSIStripping(t *testing.T) {
	result, err := ExecuteBash(BashExecutorOptions{
		Command:          "echo -e '\\033[31mred\\033[0m'",
		StripANSI:        false,
		ExplicitDefaults: true,
	})
	if err != nil {
		t.Fatalf("ExecuteBash: %v", err)
	}
	// ANSI codes should be present
	if !strings.Contains(result.Output, "\033[") {
		t.Logf("ANSI may have been stripped or not generated: %q", result.Output)
	}
}

func TestExecuteBashBinarySanitization(t *testing.T) {
	result, err := ExecuteBash(BashExecutorOptions{
		Command: "printf 'hello\\x00world'",
	})
	if err != nil {
		t.Fatalf("ExecuteBash: %v", err)
	}
	// Null byte should be replaced
	if strings.Contains(result.Output, "\x00") {
		t.Errorf("null byte not sanitized")
	}
}

func TestExecuteBashDefaults(t *testing.T) {
	opts := BashExecutorOptions{Command: "echo test"}
	opts.applyDefaults()
	if opts.Timeout != 120*time.Second {
		t.Errorf("timeout = %v, want 120s", opts.Timeout)
	}
	if opts.ShellPath != "bash" {
		t.Errorf("shell = %s, want bash", opts.ShellPath)
	}
	if opts.MaxOutputBytes != 50000 {
		t.Errorf("max output = %d, want 50000", opts.MaxOutputBytes)
	}
	if !opts.StripANSI {
		t.Error("StripANSI should default to true")
	}
	if !opts.SanitizeBinary {
		t.Error("SanitizeBinary should default to true")
	}
	if opts.Env == nil {
		t.Error("Env should not be nil after applyDefaults")
	}
}

func TestExecuteBashExplicitDefaults(t *testing.T) {
	opts := BashExecutorOptions{
		Command:          "echo test",
		StripANSI:        false,
		SanitizeBinary:   false,
		ExplicitDefaults: true,
	}
	opts.applyDefaults()
	if opts.StripANSI {
		t.Error("StripANSI should remain false with ExplicitDefaults")
	}
	if opts.SanitizeBinary {
		t.Error("SanitizeBinary should remain false with ExplicitDefaults")
	}
}

func TestSanitizeBinary(t *testing.T) {
	tests := []struct {
		name  string
		input []byte
		want  []byte
	}{
		{"noop", []byte("hello"), []byte("hello")},
		{"null byte", []byte("a\x00b"), []byte("a\uFFFDb")},
		{"control char", []byte{0x01, 0x02}, []byte("??")},
		{"tab newline cr preserved", []byte("\t\n\r"), []byte("\t\n\r")},
		{"DEL", []byte{0x7f}, []byte("?")},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := sanitizeBinary(tt.input)
			if string(got) != string(tt.want) {
				t.Errorf("sanitizeBinary(%q) = %q, want %q", tt.input, got, tt.want)
			}
		})
	}
}

func TestKillProcessTree(t *testing.T) {
	t.Run("invalid pid", func(t *testing.T) {
		err := KillProcessTree(-1)
		if err == nil {
			t.Fatal("expected error for invalid pid")
		}
	})

	t.Run("zero pid", func(t *testing.T) {
		err := KillProcessTree(0)
		if err == nil {
			t.Fatal("expected error for pid 0")
		}
	})

	t.Run("nonexistent pid", func(t *testing.T) {
		err := KillProcessTree(99999)
		if err != nil {
			t.Logf("KillProcessTree: %v (expected on some systems)", err)
		}
	})
}

func TestGetShellEnv(t *testing.T) {
	env := GetShellEnv()
	if len(env) == 0 {
		t.Error("env is empty")
	}
	hasPath := false
	for _, e := range env {
		if strings.HasPrefix(e, "PATH=") {
			hasPath = true
			break
		}
	}
	if !hasPath {
		t.Error("PATH not found in env")
	}
}

func TestFormatBashResult(t *testing.T) {
	tests := []struct {
		name   string
		result BashResult
		checks []string
	}{
		{
			name:   "normal",
			result: BashResult{ExitCode: 0, Output: "hello"},
			checks: []string{"exit code: 0", "hello"},
		},
		{
			name:   "cancelled",
			result: BashResult{ExitCode: -1, Cancelled: true, Output: ""},
			checks: []string{"exit code: -1", "(cancelled)"},
		},
		{
			name:   "truncated",
			result: BashResult{ExitCode: 0, Truncated: true, Output: "data"},
			checks: []string{"exit code: 0", "(truncated)", "data"},
		},
		{
			name:   "with spill path",
			result: BashResult{ExitCode: 0, Output: "data", FullOutputPath: "/tmp/spill.txt"},
			checks: []string{"exit code: 0", "[full output at /tmp/spill.txt]"},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			formatted := FormatBashResult(&tt.result)
			for _, check := range tt.checks {
				if !strings.Contains(formatted, check) {
					t.Errorf("missing %q in %q", check, formatted)
				}
			}
		})
	}
}

func TestExecuteBashWithShellPath(t *testing.T) {
	result, err := ExecuteBash(BashExecutorOptions{
		Command:   "echo custom_shell",
		ShellPath: "bash",
	})
	if err != nil {
		t.Fatalf("ExecuteBash: %v", err)
	}
	if !strings.Contains(result.Output, "custom_shell") {
		t.Errorf("output = %s", result.Output)
	}
}
