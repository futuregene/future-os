package extensions

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strings"
)

// ---------------------------------------------------------------------------
// Extension descriptor (extension.json)
// ---------------------------------------------------------------------------

// ExtensionManifest describes an extension via a JSON config file.
// This allows simple extensions to be defined without writing Go plugins.
type ExtensionManifest struct {
	// Name is the unique extension name (required).
	Name string `json:"name"`

	// Description is a human-readable description.
	Description string `json:"description,omitempty"`

	// Version is the extension version.
	Version string `json:"version,omitempty"`

	// Tools defines tools provided by this extension.
	Tools []ExtensionToolDef `json:"tools,omitempty"`

	// SlashCommands defines slash commands provided by this extension.
	SlashCommands []ExtensionSlashCommandDef `json:"slash_commands,omitempty"`

	// Prompts defines prompt templates provided by this extension.
	Prompts []ExtensionPromptDef `json:"prompts,omitempty"`
}

// ExtensionToolDef describes a tool in the extension manifest.
type ExtensionToolDef struct {
	// Name is the tool function name (required).
	Name string `json:"name"`

	// Description is a human-readable description of the tool.
	Description string `json:"description"`

	// Parameters is the JSON Schema for the tool's parameters.
	// If empty, a default schema is used.
	Parameters json.RawMessage `json:"parameters,omitempty"`
}

// ExtensionSlashCommandDef describes a slash command in the extension manifest.
// Config-based slash commands are limited to displaying static text or running
// a simple built-in action. For complex logic, use a Go plugin.
type ExtensionSlashCommandDef struct {
	// Command is the slash command including leading "/" (e.g. "/hello").
	Command string `json:"command"`

	// Description is shown in help text.
	Description string `json:"description,omitempty"`

	// Message is the static text to display when the command is invoked.
	Message string `json:"message,omitempty"`

	// Action is a named built-in action. Supported: "echo".
	Action string `json:"action,omitempty"`
}

// ExtensionPromptDef describes a prompt template in the extension manifest.
type ExtensionPromptDef struct {
	// Name is the prompt name for lookup.
	Name string `json:"name"`

	// Template is the prompt template text.
	Template string `json:"template"`
}

// ---------------------------------------------------------------------------
// ConfigExtension — wraps a manifest and implements the Extension interface
// ---------------------------------------------------------------------------

// ConfigExtension is an Extension backed by an ExtensionManifest.
// It registers tools, slash commands, and prompts from the manifest.
type ConfigExtension struct {
	Manifest ExtensionManifest
}

// Name returns the extension name from the manifest.
func (e *ConfigExtension) Name() string {
	return e.Manifest.Name
}

// Init registers all tools, slash commands, and prompts from the manifest.
func (e *ConfigExtension) Init(ctx ExtensionContext) error {
	// Register tools
	for _, t := range e.Manifest.Tools {
		if t.Name == "" {
			ctx.Logger.Warn("extension %q: skipping tool with empty name", e.Manifest.Name)
			continue
		}
		params := t.Parameters
		if len(params) == 0 {
			params = json.RawMessage(`{"type":"object","properties":{}}`)
		}
		// Config-based tools have no dynamic handler; they are declarative only.
		// The handler returns a message indicating this is a config-based tool.
		handler := func(args json.RawMessage) (string, error) {
			return fmt.Sprintf("[config-based tool %q invoked]", t.Name), nil
		}
		if err := ctx.RegisterTool(t.Name, handler, t.Description, params); err != nil {
			return fmt.Errorf("extension %q: register tool %q: %w", e.Manifest.Name, t.Name, err)
		}
		ctx.Logger.Info("extension %q: registered tool %q", e.Manifest.Name, t.Name)
	}

	// Register slash commands
	for _, c := range e.Manifest.SlashCommands {
		if c.Command == "" {
			ctx.Logger.Warn("extension %q: skipping slash command with empty name", e.Manifest.Name)
			continue
		}
		handler := func(args []string, _ ExtensionContext) (string, error) {
			if c.Message != "" {
				return c.Message, nil
			}
			return fmt.Sprintf("[%s]", c.Description), nil
		}
		if err := ctx.RegisterSlashCommand(c.Command, handler); err != nil {
			return fmt.Errorf("extension %q: register slash command %q: %w", e.Manifest.Name, c.Command, err)
		}
		ctx.Logger.Info("extension %q: registered slash command %q", e.Manifest.Name, c.Command)
	}

	// Register prompts
	for _, p := range e.Manifest.Prompts {
		if p.Name == "" {
			ctx.Logger.Warn("extension %q: skipping prompt with empty name", e.Manifest.Name)
			continue
		}
		if err := ctx.RegisterPrompt(p.Name, p.Template); err != nil {
			return fmt.Errorf("extension %q: register prompt %q: %w", e.Manifest.Name, p.Name, err)
		}
		ctx.Logger.Info("extension %q: registered prompt %q", e.Manifest.Name, p.Name)
	}

	return nil
}

// Deinit is a no-op for config-based extensions.
func (e *ConfigExtension) Deinit() error {
	return nil
}

// ---------------------------------------------------------------------------
// Loading functions
// ---------------------------------------------------------------------------

// LoadExtensions scans the given paths for extensions and loads them.
// Each path can be:
//   - A directory: scanned for extension.json and *.so files
//   - A .json file: parsed as an ExtensionManifest
//   - A .so file: loaded as a Go plugin (linux/macOS only)
//
// Returns the list of loaded Extension instances. Extensions that fail to
// load are skipped with a warning logged.
func LoadExtensions(paths []string, logger Logger) ([]Extension, error) {
	var extensions []Extension
	seen := make(map[string]bool)

	for _, path := range paths {
		info, err := os.Stat(path)
		if err != nil {
			logger.Warn("skipping extension path %q: %v", path, err)
			continue
		}

		if info.IsDir() {
			loaded, err := scanDirectory(path, logger)
			if err != nil {
				logger.Warn("scanning directory %q: %v", path, err)
			}
			for _, ext := range loaded {
				if seen[ext.Name()] {
					logger.Warn("skipping duplicate extension %q from %q", ext.Name(), path)
					continue
				}
				seen[ext.Name()] = true
				extensions = append(extensions, ext)
			}
		} else {
			ext, err := loadExtensionFile(path, logger)
			if err != nil {
				logger.Warn("loading extension %q: %v", path, err)
				continue
			}
			if ext == nil {
				continue
			}
			if seen[ext.Name()] {
				logger.Warn("skipping duplicate extension %q from %q", ext.Name(), path)
				continue
			}
			seen[ext.Name()] = true
			extensions = append(extensions, ext)
		}
	}

	return extensions, nil
}

// scanDirectory scans a directory for extension.json and *.so files.
func scanDirectory(dir string, logger Logger) ([]Extension, error) {
	entries, err := os.ReadDir(dir)
	if err != nil {
		return nil, fmt.Errorf("read directory %q: %w", dir, err)
	}

	var extensions []Extension

	for _, entry := range entries {
		if entry.IsDir() {
			continue
		}
		name := entry.Name()
		fullPath := filepath.Join(dir, name)

		switch {
		case name == "extension.json":
			ext, err := loadManifestFile(fullPath, logger)
			if err != nil {
				logger.Warn("loading manifest %q: %v", fullPath, err)
				continue
			}
			extensions = append(extensions, ext)

		case strings.HasSuffix(name, ".so"):
			ext, err := loadPluginFile(fullPath, logger)
			if err != nil {
				logger.Warn("loading plugin %q: %v", fullPath, err)
				continue
			}
			if ext != nil {
				extensions = append(extensions, ext)
			}
		}
	}

	return extensions, nil
}

// loadExtensionFile loads a single file as either a manifest or plugin.
func loadExtensionFile(path string, logger Logger) (Extension, error) {
	ext := filepath.Ext(path)
	switch ext {
	case ".json":
		return loadManifestFile(path, logger)
	case ".so":
		return loadPluginFile(path, logger)
	default:
		return nil, fmt.Errorf("unsupported extension file type: %s", ext)
	}
}

// loadManifestFile reads and parses an extension.json file.
func loadManifestFile(path string, logger Logger) (*ConfigExtension, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("read manifest: %w", err)
	}

	var manifest ExtensionManifest
	if err := json.Unmarshal(data, &manifest); err != nil {
		return nil, fmt.Errorf("parse manifest: %w", err)
	}

	if manifest.Name == "" {
		// Derive name from directory
		manifest.Name = filepath.Base(filepath.Dir(path))
		if manifest.Name == "." || manifest.Name == "/" {
			manifest.Name = "unnamed"
		}
	}

	logger.Info("loaded extension manifest %q from %s", manifest.Name, path)
	return &ConfigExtension{Manifest: manifest}, nil
}

// loadPluginFile loads a Go plugin (.so) file.
// Only supported on linux and macOS (darwin). On other platforms, logs a
// warning and returns nil.
func loadPluginFile(path string, logger Logger) (Extension, error) {
	if runtime.GOOS != "linux" && runtime.GOOS != "darwin" {
		logger.Warn("Go plugins are not supported on %s (skipping %q)", runtime.GOOS, path)
		return nil, nil
	}

	// We use a build-tag guarded file for the actual plugin.Open call.
	// See plugin_loader.go (guarded by //go:build linux || darwin)
	// and plugin_loader_unsupported.go (guarded by //go:build !linux && !darwin).
	return loadGoPlugin(path, logger)
}

// ---------------------------------------------------------------------------
// Extension discovery — auto-detect extension paths from standard directories
// ---------------------------------------------------------------------------

// DiscoverExtensionPaths discovers extension paths from standard directories.
// Mirrors pi-mono's discoverAndLoadExtensions:
//   1. Global: ~/.xihu/extensions/
//   2. Project: <cwd>/.xihu/extensions/
//
// Returns a deduplicated list of absolute paths that can be passed to
// LoadExtensions. Never returns an error — missing directories are silently
// skipped.
func DiscoverExtensionPaths(cwd string) []string {
	var paths []string
	seen := make(map[string]bool)

	// 1. Global extensions directory
	homeDir, err := os.UserHomeDir()
	if err == nil {
		globalDir := filepath.Join(homeDir, ".xihu", "extensions")
		discovered := discoverDir(globalDir)
		for _, p := range discovered {
			if !seen[p] {
				seen[p] = true
				paths = append(paths, p)
			}
		}
	}

	// 2. Project-local extensions directory
	if cwd != "" {
		localDir := filepath.Join(cwd, ".xihu", "extensions")
		discovered := discoverDir(localDir)
		for _, p := range discovered {
			if !seen[p] {
				seen[p] = true
				paths = append(paths, p)
			}
		}
	}

	return paths
}

// discoverDir discovers extension paths within a single directory.
// It scans for:
//   - Direct files: *.json, *.so
//   - Subdirectories containing extension.json or *.so
func discoverDir(dir string) []string {
	entries, err := os.ReadDir(dir)
	if err != nil {
		return nil // directory doesn't exist — silently skip
	}

	var paths []string
	for _, entry := range entries {
		fullPath := filepath.Join(dir, entry.Name())
		if entry.IsDir() {
			// Check if subdirectory contains extension.json or *.so
			subPaths := discoverSubDir(fullPath)
			paths = append(paths, subPaths...)
		} else {
			ext := filepath.Ext(entry.Name())
			if ext == ".json" || ext == ".so" {
				paths = append(paths, fullPath)
			}
		}
	}
	return paths
}

// discoverSubDir discovers extension entry points within a subdirectory.
// Returns the subdirectory path itself if it contains extension artifacts;
// the caller (scanDirectory) will then handle the actual loading.
func discoverSubDir(dir string) []string {
	entries, err := os.ReadDir(dir)
	if err != nil {
		return nil
	}
	for _, entry := range entries {
		if entry.IsDir() {
			continue
		}
		name := entry.Name()
		if name == "extension.json" || strings.HasSuffix(name, ".so") {
			return []string{dir}
		}
	}
	return nil
}

// ---------------------------------------------------------------------------
// Inline extension loading — mirrors pi-mono loadExtensionFromFactory
// ---------------------------------------------------------------------------

// LoadExtensionFromFactory loads an extension from a factory function.
// This allows extensions to be defined inline (in code) without file I/O.
// Mirrors pi-mono's loadExtensionFromFactory.
func LoadExtensionFromFactory(factory ExtensionFactory, name string) Extension {
	return &factoryExtension{
		name:    name,
		factory: factory,
	}
}

// factoryExtension wraps an ExtensionFactory as an Extension.
type factoryExtension struct {
	name    string
	factory ExtensionFactory
	ext     Extension
}

func (f *factoryExtension) Name() string { return f.name }

func (f *factoryExtension) Init(ctx ExtensionContext) error {
	f.ext = f.factory()
	return f.ext.Init(ctx)
}

func (f *factoryExtension) Deinit() error {
	if f.ext != nil {
		return f.ext.Deinit()
	}
	return nil
}
