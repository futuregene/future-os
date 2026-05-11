package rpcclient

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"sync/atomic"
	"time"
)
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
