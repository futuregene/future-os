package rpcclient

import (
	"encoding/json"
)

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
