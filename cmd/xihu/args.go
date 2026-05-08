package main

import (
	"fmt"
	"os"
	"strings"
)

// Args holds all parsed CLI arguments.
type Args struct {
	// Provider & Model
	Provider string
	Model    string
	APIKey   string

	// System Prompt
	SystemPrompt       string
	AppendSystemPrompt []string

	// Mode
	Mode      string // "text", "json", "rpc"
	Print     bool   // -p: non-interactive, process and exit
	NoSession bool   // don't save session

	// Session
	Continue   bool
	Resume     bool
	Session    string // specific session ID or path
	Fork       string // fork from session
	SessionDir string

	// Tools
	NoTools        bool
	NoBuiltinTools bool
	Tools          []string // allowlist

	// Resources
	Extensions        []string
	NoExtensions      bool
	Skills            []string
	NoSkills          bool
	PromptTemplates   []string
	NoPromptTemplates bool
	Themes            []string
	NoThemes          bool

	// Behavior
	Thinking       string // off, minimal, low, medium, high, xhigh
	ScopedModels   []string
	NoContextFiles bool
	Export         string
	ListModels     string // "" = not requested, "true" = all, "pattern" = search
	Verbose        bool
	Offline        bool

	// Meta
	Help    bool
	Version bool

	// Positional
	Messages  []string
	FileArgs  []string // from @file
	Diagnostics []string

	// Unknown/extension flags
	UnknownFlags map[string]string
}

// validThinkingLevels are the accepted values for --thinking.
var validThinkingLevels = map[string]bool{
	"off": true, "minimal": true, "low": true, "medium": true, "high": true, "xhigh": true,
}

// validModes are the accepted values for --mode.
var validModes = map[string]bool{
	"text": true, "json": true, "rpc": true,
}

// parseArgs parses os.Args[1:] into an Args struct.
func parseArgs(raw []string) *Args {
	a := &Args{
		UnknownFlags: make(map[string]string),
		Mode:         "text",
	}

	i := 0
	for i < len(raw) {
		arg := raw[i]

		switch arg {
		case "--help", "-h":
			a.Help = true
		case "--version", "-v":
			a.Version = true
		case "--mode":
			i++
			if i < len(raw) && validModes[raw[i]] {
				a.Mode = raw[i]
			}
		case "--continue", "-c":
			a.Continue = true
		case "--resume", "-r":
			a.Resume = true
		case "--provider":
			i++
			if i < len(raw) {
				a.Provider = raw[i]
			}
		case "--model":
			i++
			if i < len(raw) {
				a.Model = raw[i]
			}
		case "--api-key":
			i++
			if i < len(raw) {
				a.APIKey = raw[i]
			}
		case "--system-prompt":
			i++
			if i < len(raw) {
				a.SystemPrompt = raw[i]
			}
		case "--append-system-prompt":
			i++
			if i < len(raw) {
				a.AppendSystemPrompt = append(a.AppendSystemPrompt, raw[i])
			}
		case "--no-session":
			a.NoSession = true
		case "--session":
			i++
			if i < len(raw) {
				a.Session = raw[i]
			}
		case "--fork":
			i++
			if i < len(raw) {
				a.Fork = raw[i]
			}
		case "--session-dir":
			i++
			if i < len(raw) {
				a.SessionDir = raw[i]
			}
		case "--models":
			i++
			if i < len(raw) {
				a.ScopedModels = splitAndTrim(raw[i], ",")
			}
		case "--no-tools", "-nt":
			a.NoTools = true
		case "--no-builtin-tools", "-nbt":
			a.NoBuiltinTools = true
		case "--tools", "-t":
			i++
			if i < len(raw) {
				a.Tools = splitAndTrim(raw[i], ",")
			}
		case "--thinking":
			i++
			if i < len(raw) {
				level := raw[i]
				if validThinkingLevels[level] {
					a.Thinking = level
				} else {
					a.Diagnostics = append(a.Diagnostics,
						fmt.Sprintf("Invalid thinking level \"%s\". Valid: off, minimal, low, medium, high, xhigh", level))
				}
			}
		case "--print", "-p":
			a.Print = true
			// -p can optionally take the message as the next non-flag arg
			if i+1 < len(raw) && !strings.HasPrefix(raw[i+1], "-") && !strings.HasPrefix(raw[i+1], "@") {
				i++
				a.Messages = append(a.Messages, raw[i])
			}
		case "--export":
			i++
			if i < len(raw) {
				a.Export = raw[i]
			}
		case "--extension", "-e":
			i++
			if i < len(raw) {
				a.Extensions = append(a.Extensions, raw[i])
			}
		case "--no-extensions", "-ne":
			a.NoExtensions = true
		case "--skill":
			i++
			if i < len(raw) {
				a.Skills = append(a.Skills, raw[i])
			}
		case "--no-skills", "-ns":
			a.NoSkills = true
		case "--prompt-template":
			i++
			if i < len(raw) {
				a.PromptTemplates = append(a.PromptTemplates, raw[i])
			}
		case "--no-prompt-templates", "-np":
			a.NoPromptTemplates = true
		case "--theme":
			i++
			if i < len(raw) {
				a.Themes = append(a.Themes, raw[i])
			}
		case "--no-themes":
			a.NoThemes = true
		case "--no-context-files", "-nc":
			a.NoContextFiles = true
		case "--list-models":
			if i+1 < len(raw) && !strings.HasPrefix(raw[i+1], "-") && !strings.HasPrefix(raw[i+1], "@") {
				i++
				a.ListModels = raw[i]
			} else {
				a.ListModels = "true"
			}
		case "--verbose":
			a.Verbose = true
		case "--offline":
			a.Offline = true
		default:
			if strings.HasPrefix(arg, "@") {
				a.FileArgs = append(a.FileArgs, arg[1:])
			} else if strings.HasPrefix(arg, "--") {
				// Unknown --flag or --flag=value
				eq := strings.Index(arg, "=")
				if eq >= 0 {
					a.UnknownFlags[arg[2:eq]] = arg[eq+1:]
				} else {
					name := arg[2:]
					if i+1 < len(raw) && !strings.HasPrefix(raw[i+1], "-") && !strings.HasPrefix(raw[i+1], "@") {
						i++
						a.UnknownFlags[name] = raw[i]
					} else {
						a.UnknownFlags[name] = "true"
					}
				}
			} else if strings.HasPrefix(arg, "-") {
				a.Diagnostics = append(a.Diagnostics, "Unknown option: "+arg)
			} else {
				a.Messages = append(a.Messages, arg)
			}
		}
		i++
	}
	return a
}

// splitAndTrim splits s by sep, trims whitespace, and filters empties.
func splitAndTrim(s, sep string) []string {
	parts := strings.Split(s, sep)
	var result []string
	for _, p := range parts {
		p = strings.TrimSpace(p)
		if p != "" {
			result = append(result, p)
		}
	}
	return result
}

// NoThinkOverride returns true if --thinking was explicitly set to override defaults.
func (a *Args) NoThinkOverride() bool {
	return a.Thinking == "off"
}

// printHelp prints the full help text.
func printHelp() {
	fmt.Print(`xihu — AI coding assistant with read, bash, edit, write tools

Usage:
  xihu [options] [@files...] [messages...]

Options:
  --provider <name>              Provider name (e.g. openai, anthropic)
  --model <pattern>              Model ID or pattern (e.g. gpt-4o, claude-sonnet)
  --api-key <key>                API key (defaults to LLM_API_KEY env var)
  --system-prompt <text>         Override system prompt
  --append-system-prompt <text>  Append text to system prompt (repeatable)
  --mode <mode>                  Output mode: text (default), json, rpc

  --print, -p                    Non-interactive: process prompt and exit
  --continue, -c                 Continue most recent session
  --resume, -r                   Select a session to resume
  --session <path|id>            Use specific session file or partial ID
  --fork <path|id>               Fork a session into a new session
  --session-dir <dir>            Directory for session storage
  --no-session                   Don't save session (ephemeral)

  --models <patterns>            Comma-separated model patterns for cycling
  --no-tools, -nt                Disable all tools
  --no-builtin-tools, -nbt       Disable built-in tools only
  --tools, -t <tools>            Comma-separated allowlist of tool names
  --thinking <level>             Thinking level: off, minimal, low, medium, high, xhigh

  --extension, -e <path>         Load an extension file (repeatable)
  --no-extensions, -ne           Disable extension discovery
  --skill <path>                 Load a skill file or dir (repeatable)
  --no-skills, -ns               Disable skills discovery
  --prompt-template <path>       Load a prompt template (repeatable)
  --no-prompt-templates, -np     Disable prompt template discovery
  --theme <path>                 Load a theme file or dir (repeatable)
  --no-themes                    Disable theme discovery
  --no-context-files, -nc        Disable AGENTS.md/CLAUDE.md loading

  --export <file>                Export session to HTML and exit
  --list-models [search]         List available models (with optional fuzzy search)
  --verbose                      Force verbose startup
  --offline                      Disable startup network operations
  --help, -h                     Show this help
  --version, -v                  Show version

Examples:
  # Interactive mode
  xihu

  # Interactive mode with initial prompt
  xihu "List all .go files in src/"

  # Include files in initial message
  xihu @prompt.md "Read this and explain"

  # Non-interactive mode (process and exit)
  xihu -p "List all .go files in src/"

  # Continue previous session
  xihu --continue "What did we discuss?"

  # Use different model
  xihu --provider openai --model gpt-4o-mini "Help me refactor"

  # Read-only mode (no file modifications)
  xihu --tools read,grep,find,ls -p "Review the code"

  # Export a session to HTML
  xihu --export session.jsonl output.html

Environment:
  LLM_API_KEY                     API key for LLM provider
  LLM_BASE_URL                    Base URL for API endpoint
  LLM_MODEL                       Default model name
  ANTHROPIC_API_KEY               Anthropic Claude API key
  OPENAI_API_KEY                  OpenAI API key
  COBALT_OFFLINE                  Disable network ops when set to 1/true
  COBALT_SESSION_DIR              Session storage directory

Built-in Tools:
  read   - Read file contents
  bash   - Execute shell commands
  edit   - Edit files with find/replace
  write  - Write files (creates/overwrites)
  grep   - Search file contents (ripgrep-backed)
  find   - Find files by glob pattern
  ls     - List directory contents
`)
	os.Exit(0)
}

// printVersion prints version info.
func printVersion() {
	fmt.Println("xihu v0.3.0")
	os.Exit(0)
}
