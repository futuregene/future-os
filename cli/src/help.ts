export function printHelp(): void {
  console.log(`Future OS CLI

Usage:
  future auth login
  future auth status
  future auth logout
  future account profile
  future account balance [--json]
  future account recharge --amount <yuan> --channel <alipay|wechat>
  future run [options] [@files...] [message...]
  future tools list
  future tools call <name> [--args '<json>' | --stdin] [--input <path>] [--mask <path>]
  future skills list
  future skills install <name> [--version <ver>] [--scope <app|project|global>]
  future skills uninstall <name> [--scope <app|project|global>]
  future skills install-builtin [--scope <app|project|global>]
  future doctor [--fix]

Run 'future run --help' for run command options.
`);
}
