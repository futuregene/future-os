export function printHelp(): void {
  console.log(`Future OS CLI — agent gateway for the Future Agent gRPC server (default 127.0.0.1:50051).

Usage:
  future <group> <command> [options] [args...]

Groups:
  auth      Authentication & API key management
  account   Platform account info
  run       Send a prompt to the agent (one-shot, non-interactive)
  skills    Install & manage agent skills
  tools     List, describe, and call platform & browser tools
  models    List available AI models from the agent
  agent     Show running agent status
  session   List, inspect, rename, and delete agent sessions
  doctor    Environment diagnostic

Quick start:
  future auth login                          Sign in to the Future platform
  future run "Explain this project"          One-shot agent prompt
  future run @README.md "Summarize this"     Include files in prompt
  future skills install-builtin              Install all built-in skills
  future doctor                              Check everything is working

Run 'future <group> --help' for per-group details.
  future run --help          All run options (model, fork, thinking, tools, etc.)
  future auth --help         Auth subcommands
  future account --help      Account subcommands
  future skills --help       Skills subcommands
  future tools --help        Tool subcommands
  future models --help       Model listing options
  future session --help      Session management options
  future --version           Print version and exit`);
}
