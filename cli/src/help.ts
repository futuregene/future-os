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
  future account profile
  future account balance [--json]
  future account recharge --amount <yuan> --channel <alipay|wechat>
  future run [options] [@files...] [message...]
  future tools list
  future tools call <name> [--args '<json>' | --stdin]
  future skills list
  future skills install <name> [--version <ver>] [--scope <app|project|global>]
  future skills uninstall <name> [--scope <app|project|global>]
  future skills install-builtin [--scope <app|project|global>]
  future docker [--fix]
  future tui [tui options]

Run 'future run --help' for run command options.
`);
}
