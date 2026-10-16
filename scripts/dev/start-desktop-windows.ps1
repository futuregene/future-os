# Desktop owns its Agent. No stale PID cleanup, raw TCP readiness or SQL edits.
$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$desktopDir = Join-Path $repoRoot 'desktop'
$mode = $env:FUTURE_DESKTOP_AGENT_MODE
if (-not $mode) { $mode = 'auto' }
if ($mode -cnotin @('auto', 'external')) { throw 'FUTURE_DESKTOP_AGENT_MODE must be auto or external' }
$env:FUTURE_DESKTOP_AGENT_MODE = $mode
$devPort = 5173
if ($env:DESKTOP_DEV_PORT) {
    if ($env:DESKTOP_DEV_PORT -notmatch '^[0-9]{1,5}$') { throw 'DESKTOP_DEV_PORT must be a port number' }
    $devPort = [int]$env:DESKTOP_DEV_PORT
}
if ($devPort -lt 1 -or $devPort -gt 65535) { throw 'DESKTOP_DEV_PORT must be a port number' }
$cliTarget = $env:CARGO_TARGET_DIR
if (-not $cliTarget) { $cliTarget = Join-Path $repoRoot 'target' }
if (-not [IO.Path]::IsPathRooted($cliTarget)) { $cliTarget = Join-Path $repoRoot $cliTarget }
$cliTarget = [IO.Path]::GetFullPath($cliTarget)
if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR = $cliTarget }
Write-Host "Workspace: $repoRoot"
Write-Host "Agent mode: $mode (auto: external or embedded; external: attach only)"
Write-Host "Endpoint: $($env:FUTURE_AGENT_GRPC_ADDR); Desktop performs compatibility and lock checks"
Write-Host "Desktop dev port: $devPort"
Write-Host "CLI artifact: $(Join-Path $cliTarget 'debug\future.exe')"
if ($env:DRY_RUN -eq '1') { Write-Host 'DRY_RUN=1; no writes, builds, probes or launches.'; exit 0 }
$configFile = $null
Push-Location $repoRoot
try {
    $logDir = Join-Path $repoRoot '.logs'
    [void](New-Item -ItemType Directory -Force $logDir)
    $cacheLog = Join-Path $logDir 'rust-cache-preflight.log'
    [IO.File]::WriteAllText($cacheLog, '')
    & node (Join-Path $PSScriptRoot 'start-desktop-rust-cache.mjs') $cacheLog $cliTarget
    if ($LASTEXITCODE -ne 0) { throw 'Cargo compiler metadata recovery failed; see rust-cache-preflight.log' }
    if (-not (Test-Path (Join-Path $desktopDir 'node_modules'))) {
        & npm.cmd ci
        if ($LASTEXITCODE -ne 0) { throw 'npm ci failed' }
    }
    if ($env:BUILD_CLI -ne '0') {
        & cargo build -p future-cli
        if ($LASTEXITCODE -ne 0) { Write-Warning 'CLI build failed; CLI-dependent features require a matching future binary.' }
    }
    $triple = ((& rustc -Vv) | Select-String '^host: ').ToString().Substring(6)
    $binDir = Join-Path $desktopDir 'src-tauri\binaries'
    [void](New-Item -ItemType Directory -Force $binDir)
    $staged = Join-Path $binDir "future-$triple.exe"
    $cli = Join-Path $cliTarget 'debug\future.exe'
    if ((Test-Path $cli) -and (Get-Item $cli).Length -gt 0) {
        Copy-Item -Force $cli $staged
        $env:PATH = (Split-Path $cli) + ';' + $env:PATH
    } elseif (-not (Test-Path $staged) -or (Get-Item $staged).Length -eq 0) {
        Write-Warning 'No CLI artifact; only CLI-dependent features are unavailable.'
        [IO.File]::WriteAllBytes($staged, [byte[]]@())
    }
    Set-Location $desktopDir
    if ($env:RUN_CHECKS -eq '1') {
        & npm.cmd run lint
        if ($LASTEXITCODE -ne 0) { throw 'lint failed' }
        & npm.cmd run stylelint
        if ($LASTEXITCODE -ne 0) { throw 'stylelint failed' }
        & cargo check --manifest-path src-tauri/Cargo.toml
        if ($LASTEXITCODE -ne 0) { throw 'cargo check failed' }
    }
    $configFile = [IO.Path]::GetTempFileName()
    $config = @{build = @{devUrl = "http://127.0.0.1:$devPort"; beforeDevCommand = "npm run dev -- --strictPort --port $devPort"}} | ConvertTo-Json -Depth 4
    [IO.File]::WriteAllText($configFile, $config, [System.Text.UTF8Encoding]::new($false))
    Write-Host 'Starting Desktop. Quit in the app for coordinated cleanup; Ctrl-C interrupts the dev toolchain. External Agents are never stopped.'
    & npm.cmd run tauri:dev -- --config $configFile
    $result = $LASTEXITCODE
} finally {
    if ($configFile) { Remove-Item -Force $configFile -ErrorAction SilentlyContinue }
    Pop-Location
}
exit $result
