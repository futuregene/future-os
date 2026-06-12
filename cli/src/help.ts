export function printHelp(): void {
  console.log(`Future OS CLI

Usage:
  future auth login
  future auth status
  future auth logout
  future agent start
  future agent stop
  future agent restart
  future agent status
  future channel start
  future channel stop
  future channel restart
  future channel status
  future tools list
  future tools call <name> [--args '<json>' | --stdin]
  future skills list
  future skills install <name>
  future skills update <name>
  future skills uninstall <name>
  future tui [tui options]
`);
}
