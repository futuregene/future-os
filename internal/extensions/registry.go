package extensions

import (
	"encoding/json"
	"fmt"
	"sync"

	"github.com/huichen/cobalt/pkg/types"
)

// ---------------------------------------------------------------------------
// SlashCommandHandler — called when a slash command is invoked
// ---------------------------------------------------------------------------

// SlashCommandHandler is a function that handles a slash command.
// args[0] is the command name (e.g. "/mycmd"), args[1:] are arguments.
// Returns the output string and an error. The output is shown to the user.
type SlashCommandHandler func(args []string, ctx ExtensionContext) (string, error)

// ---------------------------------------------------------------------------
// Registry — thread-safe storage for extension registrations
// ---------------------------------------------------------------------------

// Registry holds all tools, slash commands, and prompts registered by extensions.
// It is thread-safe and designed to be shared across all extensions via the
// ExtensionContext.
type Registry struct {
	mu sync.RWMutex

	tools         map[string]types.AgentTool
	slashCommands map[string]SlashCommandHandler
	prompts       map[string]string // name → template
}

// globalRegistry is the package-level registry shared by all extensions.
var globalRegistry = &Registry{
	tools:         make(map[string]types.AgentTool),
	slashCommands: make(map[string]SlashCommandHandler),
	prompts:       make(map[string]string),
}

// ---------------------------------------------------------------------------
// Tool registration
// ---------------------------------------------------------------------------

// RegisterTool registers a tool with the given name, handler, description, and
// JSON Schema parameters. If a tool with the same name already exists, an error
// is returned.
func (r *Registry) RegisterTool(name string, handler func(args json.RawMessage) (string, error), description string, parameters json.RawMessage) error {
	r.mu.Lock()
	defer r.mu.Unlock()

	if _, exists := r.tools[name]; exists {
		return fmt.Errorf("tool %q is already registered", name)
	}

	r.tools[name] = types.AgentTool{
		Def: types.ToolDef{
			Type: "function",
			Function: types.FunctionDef{
				Name:        name,
				Description: description,
				Parameters:  parameters,
			},
		},
		Handler: handler,
	}
	return nil
}

// GetAllTools returns a copy of all registered tools as a slice.
func (r *Registry) GetAllTools() []types.AgentTool {
	r.mu.RLock()
	defer r.mu.RUnlock()

	tools := make([]types.AgentTool, 0, len(r.tools))
	for _, t := range r.tools {
		tools = append(tools, t)
	}
	return tools
}

// GetTool returns a registered tool by name, or nil if not found.
func (r *Registry) GetTool(name string) *types.AgentTool {
	r.mu.RLock()
	defer r.mu.RUnlock()

	if t, ok := r.tools[name]; ok {
		return &t
	}
	return nil
}

// HasTool returns true if the named tool is registered.
func (r *Registry) HasTool(name string) bool {
	r.mu.RLock()
	defer r.mu.RUnlock()
	_, ok := r.tools[name]
	return ok
}

// ---------------------------------------------------------------------------
// Slash command registration
// ---------------------------------------------------------------------------

// RegisterSlashCommand registers a handler for a slash command. The command
// name should include the leading slash (e.g. "/mycmd"). Returns an error
// if the command is already registered.
func (r *Registry) RegisterSlashCommand(cmd string, handler SlashCommandHandler) error {
	r.mu.Lock()
	defer r.mu.Unlock()

	if _, exists := r.slashCommands[cmd]; exists {
		return fmt.Errorf("slash command %q is already registered", cmd)
	}

	r.slashCommands[cmd] = handler
	return nil
}

// GetAllSlashCommands returns a copy of all registered slash commands.
func (r *Registry) GetAllSlashCommands() map[string]SlashCommandHandler {
	r.mu.RLock()
	defer r.mu.RUnlock()

	cmds := make(map[string]SlashCommandHandler, len(r.slashCommands))
	for k, v := range r.slashCommands {
		cmds[k] = v
	}
	return cmds
}

// GetSlashCommand returns the handler for a slash command, or nil.
func (r *Registry) GetSlashCommand(cmd string) SlashCommandHandler {
	r.mu.RLock()
	defer r.mu.RUnlock()
	return r.slashCommands[cmd]
}

// HasSlashCommand returns true if the slash command is registered.
func (r *Registry) HasSlashCommand(cmd string) bool {
	r.mu.RLock()
	defer r.mu.RUnlock()
	_, ok := r.slashCommands[cmd]
	return ok
}

// ---------------------------------------------------------------------------
// Prompt registration
// ---------------------------------------------------------------------------

// RegisterPrompt registers a named prompt template. Prompts can be injected
// into the system prompt at runtime by name. Returns an error if the prompt
// name is already registered.
func (r *Registry) RegisterPrompt(name string, template string) error {
	r.mu.Lock()
	defer r.mu.Unlock()

	if _, exists := r.prompts[name]; exists {
		return fmt.Errorf("prompt %q is already registered", name)
	}

	r.prompts[name] = template
	return nil
}

// GetAllPrompts returns a copy of all registered prompt templates.
func (r *Registry) GetAllPrompts() map[string]string {
	r.mu.RLock()
	defer r.mu.RUnlock()

	prompts := make(map[string]string, len(r.prompts))
	for k, v := range r.prompts {
		prompts[k] = v
	}
	return prompts
}

// GetPrompt returns a registered prompt template by name, or empty string.
func (r *Registry) GetPrompt(name string) string {
	r.mu.RLock()
	defer r.mu.RUnlock()
	return r.prompts[name]
}

// ---------------------------------------------------------------------------
// Convenience package-level functions that operate on the global registry
// ---------------------------------------------------------------------------

// RegisterTool registers a tool in the global registry.
func RegisterTool(name string, handler func(args json.RawMessage) (string, error), description string, parameters json.RawMessage) error {
	return globalRegistry.RegisterTool(name, handler, description, parameters)
}

// RegisterSlashCommand registers a slash command in the global registry.
func RegisterSlashCommand(cmd string, handler SlashCommandHandler) error {
	return globalRegistry.RegisterSlashCommand(cmd, handler)
}

// RegisterPrompt registers a prompt template in the global registry.
func RegisterPrompt(name string, template string) error {
	return globalRegistry.RegisterPrompt(name, template)
}

// GetAllTools returns all tools from the global registry.
func GetAllTools() []types.AgentTool {
	return globalRegistry.GetAllTools()
}

// GetAllSlashCommands returns all slash commands from the global registry.
func GetAllSlashCommands() map[string]SlashCommandHandler {
	return globalRegistry.GetAllSlashCommands()
}

// GetAllPrompts returns all prompt templates from the global registry.
func GetAllPrompts() map[string]string {
	return globalRegistry.GetAllPrompts()
}
