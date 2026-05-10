package sdk

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os/exec"
	"strings"
	"sync"
	"sync/atomic"
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
	opts   ClientOptions
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

// NewClient creates a new RPC client with the given options.
// Mirrors: new RpcClient(options)
func NewClient(opts ClientOptions) *Client {
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

// =============================================================================
// Command Methods — each mirrors a method on pi-mono RpcClient
// =============================================================================

// Prompt sends a prompt to the agent. Returns immediately after sending;
// use OnEvent() to receive streaming events, WaitForIdle() to wait.
// Mirrors: RpcClient.prompt(message, images?)
func (c *Client) Prompt(message string, images ...ImageContent) error {
	return c.send(rpcCommand{
		Type:    "prompt",
		Message: message,
		Images:  images,
	})
}

// Steer queues a steering message to interrupt the agent mid-run.
// Mirrors: RpcClient.steer(message, images?)
func (c *Client) Steer(message string, images ...ImageContent) error {
	return c.send(rpcCommand{
		Type:    "steer",
		Message: message,
		Images:  images,
	})
}

// FollowUp queues a follow-up message to be processed after agent finishes.
// Mirrors: RpcClient.followUp(message, images?)
func (c *Client) FollowUp(message string, images ...ImageContent) error {
	return c.send(rpcCommand{
		Type:    "follow_up",
		Message: message,
		Images:  images,
	})
}

// Abort aborts the current agent operation.
// Mirrors: RpcClient.abort()
func (c *Client) Abort() error {
	return c.send(rpcCommand{Type: "abort"})
}

// NewSession starts a new session.
// Mirrors: RpcClient.newSession(parentSession?)
func (c *Client) NewSession(parentSession ...string) (*NewSessionResult, error) {
	ps := ""
	if len(parentSession) > 0 {
		ps = parentSession[0]
	}
	resp, err := c.sendAndWait(rpcCommand{Type: "new_session", ParentSession: ps})
	if err != nil {
		return nil, err
	}
	var data NewSessionResult
	if err := json.Unmarshal(resp.Data, &data); err != nil {
		return nil, err
	}
	return &data, nil
}

// NewSessionResult matches pi-mono's new_session response.
type NewSessionResult struct {
	Cancelled bool `json:"cancelled"`
}

// GetState returns the current session state.
// Mirrors: RpcClient.getState()
func (c *Client) GetState() (*SessionState, error) {
	resp, err := c.sendAndWait(rpcCommand{Type: "get_state"})
	if err != nil {
		return nil, err
	}
	var data SessionState
	if err := json.Unmarshal(resp.Data, &data); err != nil {
		return nil, err
	}
	return &data, nil
}

// SetModel sets the model by provider and ID.
// Mirrors: RpcClient.setModel(provider, modelId)
func (c *Client) SetModel(provider, modelID string) (*SetModelResult, error) {
	resp, err := c.sendAndWait(rpcCommand{
		Type:     "set_model",
		Provider: provider,
		ModelID:  modelID,
	})
	if err != nil {
		return nil, err
	}
	var data SetModelResult
	if err := json.Unmarshal(resp.Data, &data); err != nil {
		return nil, err
	}
	return &data, nil
}

// SetModelResult matches pi-mono's set_model response data.
type SetModelResult struct {
	Provider string `json:"provider"`
	ID       string `json:"id"`
}

// CycleModel cycles to the next model.
// Mirrors: RpcClient.cycleModel()
func (c *Client) CycleModel() (*CycleModelResult, error) {
	resp, err := c.sendAndWait(rpcCommand{Type: "cycle_model"})
	if err != nil {
		return nil, err
	}
	if string(resp.Data) == "null" || len(resp.Data) == 0 {
		return nil, nil
	}
	var data CycleModelResult
	if err := json.Unmarshal(resp.Data, &data); err != nil {
		return nil, err
	}
	return &data, nil
}

// CycleModelResult matches pi-mono's cycle_model response data.
type CycleModelResult struct {
	Model         ModelResult `json:"model"`
	ThinkingLevel string      `json:"thinkingLevel"`
	IsScoped      bool        `json:"isScoped"`
}

// ModelResult is the model info in cycle_model response.
type ModelResult struct {
	Provider string `json:"provider"`
	ID       string `json:"id"`
}

// GetAvailableModels returns the list of available models.
// Mirrors: RpcClient.getAvailableModels()
func (c *Client) GetAvailableModels() ([]ModelInfo, error) {
	resp, err := c.sendAndWait(rpcCommand{Type: "get_available_models"})
	if err != nil {
		return nil, err
	}
	var wrapper struct {
		Models []ModelInfo `json:"models"`
	}
	if err := json.Unmarshal(resp.Data, &wrapper); err != nil {
		return nil, err
	}
	return wrapper.Models, nil
}

// SetThinkingLevel sets the thinking level.
// Mirrors: RpcClient.setThinkingLevel(level)
func (c *Client) SetThinkingLevel(level string) error {
	return c.send(rpcCommand{Type: "set_thinking_level", Level: level})
}

// CycleThinkingLevel cycles to the next thinking level.
// Mirrors: RpcClient.cycleThinkingLevel()
func (c *Client) CycleThinkingLevel() (*CycleThinkingResult, error) {
	resp, err := c.sendAndWait(rpcCommand{Type: "cycle_thinking_level"})
	if err != nil {
		return nil, err
	}
	if string(resp.Data) == "null" || len(resp.Data) == 0 {
		return nil, nil
	}
	var data CycleThinkingResult
	if err := json.Unmarshal(resp.Data, &data); err != nil {
		return nil, err
	}
	return &data, nil
}

// CycleThinkingResult matches pi-mono's cycle_thinking_level response data.
type CycleThinkingResult struct {
	Level string `json:"level"`
}

// SetSteeringMode sets the steering mode ("all" or "one-at-a-time").
// Mirrors: RpcClient.setSteeringMode(mode)
func (c *Client) SetSteeringMode(mode string) error {
	return c.send(rpcCommand{Type: "set_steering_mode", Mode: mode})
}

// SetFollowUpMode sets the follow-up mode ("all" or "one-at-a-time").
// Mirrors: RpcClient.setFollowUpMode(mode)
func (c *Client) SetFollowUpMode(mode string) error {
	return c.send(rpcCommand{Type: "set_follow_up_mode", Mode: mode})
}

// Compact triggers session compaction.
// Mirrors: RpcClient.compact(customInstructions?)
func (c *Client) Compact(customInstructions ...string) (*CompactionResult, error) {
	ci := ""
	if len(customInstructions) > 0 {
		ci = customInstructions[0]
	}
	resp, err := c.sendAndWait(rpcCommand{
		Type:              "compact",
		CustomInstructions: ci,
	})
	if err != nil {
		return nil, err
	}
	var data CompactionResult
	if err := json.Unmarshal(resp.Data, &data); err != nil {
		return nil, err
	}
	return &data, nil
}

// SetAutoCompaction enables or disables auto-compaction.
// Mirrors: RpcClient.setAutoCompaction(enabled)
func (c *Client) SetAutoCompaction(enabled bool) error {
	return c.send(rpcCommand{Type: "set_auto_compaction", Enabled: enabled})
}

// SetAutoRetry enables or disables auto-retry.
// Mirrors: RpcClient.setAutoRetry(enabled)
func (c *Client) SetAutoRetry(enabled bool) error {
	return c.send(rpcCommand{Type: "set_auto_retry", Enabled: enabled})
}

// AbortRetry aborts an in-progress retry.
// Mirrors: RpcClient.abortRetry()
func (c *Client) AbortRetry() error {
	return c.send(rpcCommand{Type: "abort_retry"})
}

// Bash executes a bash command in the agent's environment.
// Mirrors: RpcClient.bash(command)
func (c *Client) Bash(command string) (*BashResult, error) {
	resp, err := c.sendAndWait(rpcCommand{Type: "bash", Command: command})
	if err != nil {
		return nil, err
	}
	var data BashResult
	if err := json.Unmarshal(resp.Data, &data); err != nil {
		return nil, err
	}
	return &data, nil
}

// AbortBash aborts a running bash command.
// Mirrors: RpcClient.abortBash()
func (c *Client) AbortBash() error {
	return c.send(rpcCommand{Type: "abort_bash"})
}

// GetSessionStats returns session usage statistics.
// Mirrors: RpcClient.getSessionStats()
func (c *Client) GetSessionStats() (*SessionStats, error) {
	resp, err := c.sendAndWait(rpcCommand{Type: "get_session_stats"})
	if err != nil {
		return nil, err
	}
	var data SessionStats
	if err := json.Unmarshal(resp.Data, &data); err != nil {
		return nil, err
	}
	return &data, nil
}

// ExportHTML exports the session to HTML.
// Mirrors: RpcClient.exportHtml(outputPath?)
func (c *Client) ExportHTML(outputPath ...string) (*ExportHTMLResult, error) {
	op := ""
	if len(outputPath) > 0 {
		op = outputPath[0]
	}
	resp, err := c.sendAndWait(rpcCommand{Type: "export_html", OutputPath: op})
	if err != nil {
		return nil, err
	}
	var data ExportHTMLResult
	if err := json.Unmarshal(resp.Data, &data); err != nil {
		return nil, err
	}
	return &data, nil
}

// ExportHTMLResult matches pi-mono's export_html response data.
type ExportHTMLResult struct {
	Path string `json:"path"`
}

// SwitchSession switches to a different session file.
// Mirrors: RpcClient.switchSession(sessionPath)
func (c *Client) SwitchSession(sessionPath string) (*SwitchSessionResult, error) {
	resp, err := c.sendAndWait(rpcCommand{Type: "switch_session", SessionPath: sessionPath})
	if err != nil {
		return nil, err
	}
	var data SwitchSessionResult
	if err := json.Unmarshal(resp.Data, &data); err != nil {
		return nil, err
	}
	return &data, nil
}

// SwitchSessionResult matches pi-mono's switch_session response.
type SwitchSessionResult struct {
	Cancelled bool `json:"cancelled"`
}

// Fork forks from a specific message.
// Mirrors: RpcClient.fork(entryId)
func (c *Client) Fork(entryID string) (*ForkResult, error) {
	resp, err := c.sendAndWait(rpcCommand{Type: "fork", EntryID: entryID})
	if err != nil {
		return nil, err
	}
	var data ForkResult
	if err := json.Unmarshal(resp.Data, &data); err != nil {
		return nil, err
	}
	return &data, nil
}

// ForkResult matches pi-mono's fork response data.
type ForkResult struct {
	Text      string `json:"text"`
	Cancelled bool   `json:"cancelled"`
}

// Clone clones the current active branch into a new session.
// Mirrors: RpcClient.clone()
func (c *Client) Clone() (*CloneResult, error) {
	resp, err := c.sendAndWait(rpcCommand{Type: "clone"})
	if err != nil {
		return nil, err
	}
	var data CloneResult
	if err := json.Unmarshal(resp.Data, &data); err != nil {
		return nil, err
	}
	return &data, nil
}

// CloneResult matches pi-mono's clone response.
type CloneResult struct {
	Cancelled bool `json:"cancelled"`
}

// GetForkMessages returns messages available for forking.
// Mirrors: RpcClient.getForkMessages()
func (c *Client) GetForkMessages() ([]ForkMessage, error) {
	resp, err := c.sendAndWait(rpcCommand{Type: "get_fork_messages"})
	if err != nil {
		return nil, err
	}
	var wrapper struct {
		Messages []ForkMessage `json:"messages"`
	}
	if err := json.Unmarshal(resp.Data, &wrapper); err != nil {
		return nil, err
	}
	return wrapper.Messages, nil
}

// GetLastAssistantText returns the text of the last assistant message.
// Mirrors: RpcClient.getLastAssistantText()
func (c *Client) GetLastAssistantText() (*string, error) {
	resp, err := c.sendAndWait(rpcCommand{Type: "get_last_assistant_text"})
	if err != nil {
		return nil, err
	}
	var wrapper struct {
		Text *string `json:"text"`
	}
	if err := json.Unmarshal(resp.Data, &wrapper); err != nil {
		return nil, err
	}
	return wrapper.Text, nil
}

// SetSessionName sets the session display name.
// Mirrors: RpcClient.setSessionName(name)
func (c *Client) SetSessionName(name string) error {
	return c.send(rpcCommand{Type: "set_session_name", Name: name})
}

// GetMessages returns all messages in the session.
// Mirrors: RpcClient.getMessages()
func (c *Client) GetMessages() ([]json.RawMessage, error) {
	resp, err := c.sendAndWait(rpcCommand{Type: "get_messages"})
	if err != nil {
		return nil, err
	}
	var wrapper struct {
		Messages []json.RawMessage `json:"messages"`
	}
	if err := json.Unmarshal(resp.Data, &wrapper); err != nil {
		return nil, err
	}
	return wrapper.Messages, nil
}

// GetCommands returns available slash commands.
// Mirrors: RpcClient.getCommands()
func (c *Client) GetCommands() ([]SlashCommandInfo, error) {
	resp, err := c.sendAndWait(rpcCommand{Type: "get_commands"})
	if err != nil {
		return nil, err
	}
	var wrapper struct {
		Commands []SlashCommandInfo `json:"commands"`
	}
	if err := json.Unmarshal(resp.Data, &wrapper); err != nil {
		return nil, err
	}
	return wrapper.Commands, nil
}

// =============================================================================
// Helper Methods — mirrors RpcClient helpers
// =============================================================================

// WaitForIdle waits for the agent to become idle (agent_end event).
// Mirrors: RpcClient.waitForIdle(timeout)
func (c *Client) WaitForIdle(timeoutMs int) error {
	ctx, cancel := context.WithTimeout(context.Background(), time.Duration(timeoutMs)*time.Millisecond)
	defer cancel()

	ch := make(chan struct{}, 1)
	unsub := c.OnEvent(func(raw json.RawMessage) {
		var event struct {
			Type string `json:"type"`
		}
		if json.Unmarshal(raw, &event) == nil && event.Type == "agent_end" {
			select {
			case ch <- struct{}{}:
			default:
			}
		}
	})
	defer unsub()

	select {
	case <-ch:
		return nil
	case <-ctx.Done():
		if errors.Is(ctx.Err(), context.DeadlineExceeded) {
			return fmt.Errorf("timeout waiting for agent to become idle. Stderr: %s", c.GetStderr())
		}
		return ctx.Err()
	}
}

// CollectEvents collects events until agent becomes idle.
// Mirrors: RpcClient.collectEvents(timeout)
func (c *Client) CollectEvents(timeoutMs int) ([]json.RawMessage, error) {
	ctx, cancel := context.WithTimeout(context.Background(), time.Duration(timeoutMs)*time.Millisecond)
	defer cancel()

	var events []json.RawMessage
	ch := make(chan struct{}, 1)

	unsub := c.OnEvent(func(raw json.RawMessage) {
		events = append(events, raw)
		var event struct {
			Type string `json:"type"`
		}
		if json.Unmarshal(raw, &event) == nil && event.Type == "agent_end" {
			select {
			case ch <- struct{}{}:
			default:
			}
		}
	})
	defer unsub()

	select {
	case <-ch:
		return events, nil
	case <-ctx.Done():
		if errors.Is(ctx.Err(), context.DeadlineExceeded) {
			return nil, fmt.Errorf("timeout collecting events. Stderr: %s", c.GetStderr())
		}
		return nil, ctx.Err()
	}
}

// PromptAndWait sends a prompt and waits for completion, returning all events.
// Mirrors: RpcClient.promptAndWait(message, images?, timeout)
func (c *Client) PromptAndWait(message string, timeoutMs int, images ...ImageContent) ([]json.RawMessage, error) {
	eventsCh := make(chan struct {
		events []json.RawMessage
		err    error
	}, 1)

	go func() {
		events, err := c.CollectEvents(timeoutMs)
		eventsCh <- struct {
			events []json.RawMessage
			err    error
		}{events, err}
	}()

	if err := c.Prompt(message, images...); err != nil {
		return nil, err
	}

	result := <-eventsCh
	return result.events, result.err
}

// =============================================================================
// Internal: send / sendAndWait — mirrors RpcClient private send / getData
// =============================================================================

// send sends a command without waiting for response (fire-and-forget).
// Mirrors: private send() for commands that don't return data.
func (c *Client) send(cmd rpcCommand) error {
	if c.stdin == nil {
		return errors.New("client not started")
	}

	id := fmt.Sprintf("req_%d", atomic.AddUint64(&c.requestID, 1))
	cmd.ID = id

	line, err := serializeJSONLine(cmd)
	if err != nil {
		return err
	}

	c.mu.Lock()
	defer c.mu.Unlock()

	if c.stdin == nil {
		return errors.New("client not started")
	}

	_, err = fmt.Fprint(c.stdin, line)
	return err
}

// sendAndWait sends a command and waits for the response.
// Mirrors: private send() that returns Promise<RpcResponse> + getData().
func (c *Client) sendAndWait(cmd rpcCommand) (*rpcResponse, error) {
	if c.stdin == nil {
		return nil, errors.New("client not started")
	}

	id := fmt.Sprintf("req_%d", atomic.AddUint64(&c.requestID, 1))
	cmd.ID = id

	// Register pending BEFORE writing to avoid race condition
	respCh := c.reader.registerPending(id)
	defer c.reader.deregisterPending(id)

	line, err := serializeJSONLine(cmd)
	if err != nil {
		return nil, err
	}

	c.mu.Lock()
	if c.stdin == nil {
		c.mu.Unlock()
		return nil, errors.New("client not started")
	}
	_, err = fmt.Fprint(c.stdin, line)
	c.mu.Unlock()

	if err != nil {
		return nil, err
	}

	// Wait for response with timeout (30s like TypeScript)
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()

	select {
	case resp := <-respCh:
		if resp == nil {
			return nil, errors.New("client shut down")
		}
		if !resp.Success {
			return nil, fmt.Errorf("rpc error: %s", resp.Error)
		}
		return resp, nil
	case <-ctx.Done():
		return nil, fmt.Errorf("timeout waiting for response to %s. Stderr: %s", cmd.Type, c.GetStderr())
	}
}
