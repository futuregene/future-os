package rpcclient

import (
	"bufio"
	"context"
	"errors"
	"fmt"
	"io"
	"os/exec"
	"strings"
	"sync"
	"syscall"
	"time"
)

// =============================================================================
// Client — mirrors pi-mono RpcClient class
// =============================================================================

// Client provides a typed Go API for interacting with xihu in RPC mode.
// It matches the pi-mono RpcClient class exactly in method names, signatures,
// and behavior.
//
// The client spawns `xihu --mode rpc` as a subprocess, sends JSONL commands
// on stdin, and receives JSONL responses/events on stdout.
type Client struct {
	opts   Options
	cmd    *exec.Cmd
	stdin  io.WriteCloser
	stdout io.ReadCloser
	reader *jsonlReader

	requestID uint64
	stderr    strings.Builder
	stderrMu  sync.Mutex

	// Event listeners unsubscribe functions
	unsubscribers []func()

	// Shutdown coordination
	ctx    context.Context
	cancel context.CancelFunc
	done   chan struct{}
	mu     sync.Mutex
}

// New creates a new RPC client with the given options.
// Mirrors: new RpcClient(options)
func New(opts Options) *Client {
	return &Client{
		opts:   opts,
		reader: newJSONLReader(),
		done:   make(chan struct{}),
	}
}

// =============================================================================
// Lifecycle — mirrors RpcClient.start() / stop()
// =============================================================================

// Start spawns the xihu agent subprocess in RPC mode.
// Mirrors: RpcClient.start()
func (c *Client) Start(ctx context.Context) error {
	c.mu.Lock()
	defer c.mu.Unlock()

	if c.cmd != nil {
		return errors.New("client already started")
	}

	c.ctx, c.cancel = context.WithCancel(ctx)

	cliPath := c.opts.CliPath
	if cliPath == "" {
		cliPath = "xihu"
	}

	args := []string{"--mode", "rpc"}
	if c.opts.Provider != "" {
		args = append(args, "--provider", c.opts.Provider)
	}
	if c.opts.Model != "" {
		args = append(args, "--model", c.opts.Model)
	}
	if c.opts.APIKey != "" {
		args = append(args, "--api-key", c.opts.APIKey)
	}
	args = append(args, c.opts.Args...)

	c.cmd = exec.CommandContext(c.ctx, cliPath, args...)
	if c.opts.Cwd != "" {
		c.cmd.Dir = c.opts.Cwd
	}
	if len(c.opts.Env) > 0 {
		c.cmd.Env = c.opts.Env
	}

	var err error
	c.stdin, err = c.cmd.StdinPipe()
	if err != nil {
		return fmt.Errorf("stdin pipe: %w", err)
	}
	c.stdout, err = c.cmd.StdoutPipe()
	if err != nil {
		return fmt.Errorf("stdout pipe: %w", err)
	}

	// Capture stderr
	stderrPipe, err := c.cmd.StderrPipe()
	if err != nil {
		return fmt.Errorf("stderr pipe: %w", err)
	}

	if err := c.cmd.Start(); err != nil {
		return fmt.Errorf("start agent: %w", err)
	}

	// Collect stderr in background
	go func() {
		buf := make([]byte, 4096)
		for {
			n, err := stderrPipe.Read(buf)
			if n > 0 {
				c.stderrMu.Lock()
				c.stderr.Write(buf[:n])
				c.stderrMu.Unlock()
			}
			if err != nil {
				return
			}
		}
	}()

	// Start reading JSONL lines from stdout
	scanner := bufio.NewScanner(c.stdout)
	scanner.Split(bufio.ScanLines)
	go func() {
		c.reader.startReading(scanner)
		close(c.done)
	}()

	// Wait briefly for process to initialize
	time.Sleep(100 * time.Millisecond)

	if c.cmd.ProcessState != nil && c.cmd.ProcessState.Exited() {
		return fmt.Errorf("agent process exited immediately with code %d. Stderr: %s",
			c.cmd.ProcessState.ExitCode(), c.GetStderr())
	}

	return nil
}

// Stop gracefully stops the agent subprocess.
// Mirrors: RpcClient.stop()
func (c *Client) Stop() error {
	c.mu.Lock()
	defer c.mu.Unlock()

	if c.cmd == nil || c.cmd.Process == nil {
		return nil
	}

	// Cancel the context to signal shutdown
	if c.cancel != nil {
		c.cancel()
	}

	// Try SIGTERM first
	_ = c.cmd.Process.Signal(syscall.SIGTERM)

	// Wait for process to exit with timeout
	done := make(chan error, 1)
	go func() {
		done <- c.cmd.Wait()
	}()

	select {
	case <-done:
	case <-time.After(1 * time.Second):
		// Force kill on timeout
		_ = c.cmd.Process.Kill()
		<-done
	}

	c.cmd = nil
	c.stdin = nil
	c.stdout = nil
	return nil
}

// GetStderr returns collected stderr output.
// Mirrors: RpcClient.getStderr()
func (c *Client) GetStderr() string {
	c.stderrMu.Lock()
	defer c.stderrMu.Unlock()
	return c.stderr.String()
}

// =============================================================================
// Event Subscription — mirrors RpcClient.onEvent()
// =============================================================================

// OnEvent subscribes to agent events. Returns an unsubscribe function.
// Mirrors: RpcClient.onEvent(listener) => unsubscribe
func (c *Client) OnEvent(listener EventHandler) func() {
	return c.reader.addListener(listener)
}

