export function printHelp(): void {
  console.log(`Future OS CLI

Usage:
  future-cli auth login
  future-cli auth status
  future-cli auth logout
  future-cli agent start
  future-cli agent stop
  future-cli agent restart
  future-cli agent status
  future-cli channel start
  future-cli channel stop
  future-cli channel restart
  future-cli channel status
  future-cli tools list
  future-cli tools call <name> [--args '<json>' | --stdin]
  future-cli skills list
  future-cli skills get <name>
  future-cli tui [tui options]
`);
}
