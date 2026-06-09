export function printHelp() {
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
  future tui [tui options]
`);
}
