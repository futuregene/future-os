// Package exec provides a standalone Bash executor with ANSI stripping,
// binary sanitization, tail truncation, process tree killing, and
// AbortSignal support.
package exec

import (
	"bytes"
	"context"
	"fmt"
	"os"
	"os/exec"
	"regexp"
	"strings"
	"syscall"
	"time"
)

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

// BashResult holds the structured result of a bash command execution.
type BashResult struct {
	// Output is the post-processed combined stdout+stderr (ANSI stripped,
	// binary-sanitized, tail-truncated).
	Output string

	// ExitCode is the exit code of the process, or -1 if the process was
	// terminated by a signal or the context was cancelled.
	ExitCode int

	// Cancelled is true when the command was aborted via AbortSignal.
	Cancelled bool

	// Truncated is true when the original output exceeded MaxOutputBytes
	// and was tail-truncated.
	Truncated bool

	// FullOutputPath is the path to a temp file containing the complete
	// (raw, unsanitized) output when it exceeds the spill threshold (10000 bytes).
	FullOutputPath string
}

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

// BashExecutorOptions configures a bash command execution.
//
// Zero-value booleans are treated as follows: when ExplicitDefaults is false,
// StripANSI and SanitizeBinary both default to true. Set ExplicitDefaults to
// true to take the literal boolean values as-is.
type BashExecutorOptions struct {
	// Command is the shell command to execute (required).
	Command string

	// CWD is the working directory for the command (default: current dir).
	CWD string

	// Timeout is the maximum duration for the command (default: 120s).
	Timeout time.Duration

	// Env is the environment for the command. If nil, uses GetShellEnv().
	Env []string

	// ShellPath is the path to the shell binary (default: "bash").
	ShellPath string

	// AbortSignal is an optional channel that, when closed, cancels the
	// command execution immediately.
	AbortSignal <-chan struct{}

	// MaxOutputBytes is the maximum number of output bytes kept in
	// Result.Output via tail truncation (default: 50000).
	MaxOutputBytes int

	// StripANSI controls whether ANSI escape sequences are stripped from
	// the output. When ExplicitDefaults is false, defaults to true.
	StripANSI bool

	// SanitizeBinary controls whether null bytes and non-printable
	// characters are replaced. When ExplicitDefaults is false, defaults to true.
	SanitizeBinary bool

	// ExplicitDefaults, when true, skips the default application for
	// StripANSI and SanitizeBinary, taking their literal values as-is.
	// This allows disabling ANSI stripping or binary sanitization.
	ExplicitDefaults bool
}

// applyDefaults fills in zero-value fields with sensible defaults.
func (o *BashExecutorOptions) applyDefaults() {
	if o.Timeout <= 0 {
		o.Timeout = 120 * time.Second
	}
	if o.ShellPath == "" {
		o.ShellPath = "bash"
	}
	if o.MaxOutputBytes <= 0 {
		o.MaxOutputBytes = 50000
	}
	if o.Env == nil {
		o.Env = GetShellEnv()
	}
	if !o.ExplicitDefaults {
		o.StripANSI = true
		o.SanitizeBinary = true
	}
}

// ---------------------------------------------------------------------------
// ANSI escape sequence regex
// ---------------------------------------------------------------------------

// ansiRegex matches ANSI escape sequences (CSI sequences).
// Pattern: ESC [ <parameter bytes> <intermediate bytes> <final byte>
// This matches: \x1b\[[0-?]*[ -/]*[@-~]
//
// Matches sequences like:
//
//	\x1b[0m          (reset)
//	\x1b[1;31m       (bold red)
//	\x1b[?25l        (hide cursor)
//	\x1b[38;5;196m   (256-color foreground)
var ansiRegex = regexp.MustCompile(`\x1b\[[0-?]*[ -/]*[@-~]`)

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

// ExecuteBash runs a bash command with the given options and returns a
// structured result. It handles:
//   - Context-based timeout
//   - AbortSignal channel cancellation
//   - Combined stdout+stderr capture
//   - ANSI escape sequence stripping (default on)
//   - Binary output sanitization: null bytes → U+FFFD, control chars → ?
//   - Tail truncation to MaxOutputBytes
//   - Spilling full raw output to a temp file when > 10000 bytes
//   - Process tree killing via Setpgid (process group)
func ExecuteBash(opts BashExecutorOptions) (*BashResult, error) {
	opts.applyDefaults()

	// Build context with timeout
	ctx, cancel := context.WithTimeout(context.Background(), opts.Timeout)
	defer cancel()

	// Handle AbortSignal: if provided, start a goroutine that cancels
	// the context when the signal channel is closed.
	if opts.AbortSignal != nil {
		go func() {
			select {
			case <-opts.AbortSignal:
				cancel()
			case <-ctx.Done():
				// Context already done (timeout or other cancellation)
			}
		}()
	}

	// Resolve CWD
	cwd := opts.CWD
	if cwd == "" {
		var err error
		cwd, err = os.Getwd()
		if err != nil {
			cwd = "."
		}
	}

	// Build the command
	cmd := exec.CommandContext(ctx, opts.ShellPath, "-c", opts.Command)
	cmd.Dir = cwd
	cmd.Env = opts.Env

	// Run the process in its own process group so we can kill the entire
	// tree (process + children) on timeout/cancellation.
	cmd.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}

	// Capture combined stdout + stderr
	var stdoutBuf bytes.Buffer
	cmd.Stdout = &stdoutBuf
	cmd.Stderr = &stdoutBuf

	// Run and collect exit status
	runErr := cmd.Run()

	// Determine exit code
	exitCode := 0
	cancelled := false

	if cmd.ProcessState != nil {
		exitCode = cmd.ProcessState.ExitCode()
	} else if runErr != nil {
		exitCode = -1
	}

	// Detect timeout
	if ctx.Err() == context.DeadlineExceeded {
		if exitCode == 0 {
			exitCode = -1
		}
	}

	// Detect AbortSignal cancellation
	if opts.AbortSignal != nil {
		select {
		case <-opts.AbortSignal:
			cancelled = true
		default:
		}
	}
	if !cancelled && ctx.Err() == context.Canceled && opts.AbortSignal != nil {
		cancelled = true
	}

	// Get raw combined output
	rawOutput := stdoutBuf.Bytes()

	// --- Post-processing pipeline ---

	// 1. Spill to temp file if output exceeds threshold
	const spillThreshold = 10000
	var fullOutputPath string
	if len(rawOutput) > spillThreshold {
		tmpFile, tmpErr := os.CreateTemp("", "pi-bash-*.txt")
		if tmpErr == nil {
			if _, writeErr := tmpFile.Write(rawOutput); writeErr != nil {
				tmpFile.Close()
				os.Remove(tmpFile.Name())
			} else {
				tmpFile.Close()
				fullOutputPath = tmpFile.Name()
			}
		}
	}

	// 2. Strip ANSI escape sequences
	processed := rawOutput
	if opts.StripANSI {
		processed = ansiRegex.ReplaceAll(processed, nil)
	}

	// 3. Sanitize binary output
	if opts.SanitizeBinary {
		processed = sanitizeBinary(processed)
	}

	// 4. Tail truncation: keep only the last MaxOutputBytes bytes
	truncated := false
	if len(processed) > opts.MaxOutputBytes {
		processed = processed[len(processed)-opts.MaxOutputBytes:]
		truncated = true
	}

	return &BashResult{
		Output:         string(processed),
		ExitCode:       exitCode,
		Cancelled:      cancelled,
		Truncated:      truncated,
		FullOutputPath: fullOutputPath,
	}, nil
}

// ---------------------------------------------------------------------------
// Output sanitization
// ---------------------------------------------------------------------------

// sanitizeBinary replaces null bytes with U+FFFD (Unicode replacement
// character) and non-printable control characters (other than \t, \n, \r)
// with '?'. DEL (0x7f) is also replaced with '?'.
func sanitizeBinary(data []byte) []byte {
	// Pre-allocate for typical case (few replacements needed)
	result := make([]byte, 0, len(data))
	for _, b := range data {
		switch {
		case b == 0:
			// Null byte → replacement character
			result = append(result, []byte("\uFFFD")...)
		case b < 0x20 && b != '\t' && b != '\n' && b != '\r':
			// Other C0 control characters → '?'
			result = append(result, '?')
		case b == 0x7f:
			// DEL → '?'
			result = append(result, '?')
		default:
			result = append(result, b)
		}
	}
	return result
}

// ---------------------------------------------------------------------------
// Process tree killing
// ---------------------------------------------------------------------------

// KillProcessTree sends SIGKILL to the entire process group identified by
// pid. On Unix, passing a negative PID signals the process group whose ID
// is |pid|. This ensures that not just the shell process but all its child
// processes are terminated.
//
// This is typically called with the PID of a process started with
// Setpgid: true (as done by ExecuteBash).
func KillProcessTree(pid int) error {
	if pid <= 0 {
		return fmt.Errorf("kill process tree: invalid pid %d", pid)
	}
	// Negative PID signals the process group
	return syscall.Kill(-pid, syscall.SIGKILL)
}

// ---------------------------------------------------------------------------
// Environment helpers
// ---------------------------------------------------------------------------

// GetShellEnv returns the current process environment with a fallback PATH
// if none is set. The returned slice is a copy of os.Environ() and is safe
// to mutate.
func GetShellEnv() []string {
	env := os.Environ()
	hasPath := false
	for _, e := range env {
		if strings.HasPrefix(e, "PATH=") {
			hasPath = true
			break
		}
	}
	if !hasPath {
		env = append(env, "PATH=/usr/local/bin:/usr/bin:/bin")
	}
	return env
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

// FormatBashResult formats a BashResult into a human-readable string
// suitable for returning to an LLM as tool output.
//
// Format:
//
//	exit code: <code> [flags]
//	[full output at <path>]
//	<output>
func FormatBashResult(r *BashResult) string {
	var sb strings.Builder

	// Line 1: exit code + flags
	sb.WriteString(fmt.Sprintf("exit code: %d", r.ExitCode))
	if r.Cancelled {
		sb.WriteString(" (cancelled)")
	}
	if r.Truncated {
		sb.WriteString(" (truncated)")
	}
	sb.WriteByte('\n')

	// Line 2 (optional): spill path
	if r.FullOutputPath != "" {
		sb.WriteString(fmt.Sprintf("[full output at %s]\n", r.FullOutputPath))
	}

	// Remaining: output
	sb.WriteString(r.Output)

	return sb.String()
}
