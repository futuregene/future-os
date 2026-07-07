#requires -Version 5.1
<#
.SYNOPSIS
    Build the FutureOS Windows portable package locally — no GitHub Actions.

.DESCRIPTION
    Replicates the CI "Windows portable" pipeline (.github/workflows/build.yml):
      1. build the agent (release) and stage it as the Tauri sidecar,
      2. compile the CLI into a standalone .exe (bun --compile),
      3. build the GUI with Tauri (--no-bundle: just the .exe, no installer),
      4. assemble FutureOS.exe + future-agent.exe + future.exe + Readme.txt
         into FutureOS-portable-windows.zip.

    The resulting app needs the Microsoft Edge WebView2 runtime (ships with
    Windows 10/11). Keep FutureOS.exe and future-agent.exe together — the GUI
    starts the agent sidecar from its own directory.

.PARAMETER SkipDeps
    Skip `npm ci` in gui/ and cli/ (use when node_modules are already current).

.PARAMETER OutDir
    Directory to write the zip into. Defaults to the repository root.

.EXAMPLE
    pwsh scripts/build-windows-portable.ps1
    pwsh scripts/build-windows-portable.ps1 -SkipDeps -OutDir C:\builds
#>
[CmdletBinding()]
param(
    [switch]$SkipDeps,
    [string]$OutDir
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# Repo root = parent of this script's directory. Run everything from there so
# relative paths match the CI steps.
$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root

# $ErrorActionPreference="Stop" only stops on *cmdlet* errors, NOT on a native
# command (cargo/npm/bun/tauri) exiting non-zero — those would silently
# continue and produce a broken package. Run every external command through
# this so a failure aborts the build.
function Invoke-Native {
    param([Parameter(Mandatory)][scriptblock]$Command)
    & $Command
    if ($LASTEXITCODE -ne 0) {
        throw "Command failed (exit $LASTEXITCODE): $($Command.ToString().Trim())"
    }
}

function Require-Tool([string]$Cmd, [string]$Hint) {
    if (-not (Get-Command $Cmd -ErrorAction SilentlyContinue)) {
        throw "Missing required tool '$Cmd'. $Hint"
    }
}

Write-Host "==> Checking prerequisites" -ForegroundColor Cyan
Require-Tool node   "Install Node.js 24+ (https://nodejs.org)."
Require-Tool npm    "Comes with Node.js."
Require-Tool bun    "Install Bun (https://bun.sh) — compiles the CLI binary."
Require-Tool cargo  "Install Rust (https://rustup.rs)."
Require-Tool rustc  "Install Rust (https://rustup.rs)."
Require-Tool protoc "Install protobuf (e.g. 'choco install protoc') — gRPC codegen."

# Host target triple, e.g. x86_64-pc-windows-msvc. Tauri looks for the sidecar
# named future-agent-<triple>.exe (bundle.externalBin in tauri.conf.json).
$hostLine = (rustc -Vv) | Select-String '^host:'
if (-not $hostLine) { throw "Could not read host triple from 'rustc -Vv'." }
$triple = $hostLine.Line.Split(' ')[1]
Write-Host "    host triple: $triple"

# Resolve ONE version string and pin it for every sub-build (agent build.rs,
# CLI gen-version, GUI build.rs) so they all agree. version.mjs derives it from
# git: 0.0.0-<hash>+local[.dirty] locally, or the tag/CI value when set.
if (-not $env:FUTURE_VERSION) {
    $env:FUTURE_VERSION = (node scripts/version.mjs)
    if ($LASTEXITCODE -ne 0) { throw "scripts/version.mjs failed to resolve a version." }
}
Write-Host "    version: $($env:FUTURE_VERSION)"

if (-not $SkipDeps) {
    Write-Host "==> Installing npm dependencies (gui, cli)" -ForegroundColor Cyan
    Push-Location gui; try { Invoke-Native { npm ci } } finally { Pop-Location }
    Push-Location cli; try { Invoke-Native { npm ci } } finally { Pop-Location }
}

Write-Host "==> Building agent (release)" -ForegroundColor Cyan
Invoke-Native { cargo build --release --manifest-path agent/Cargo.toml }
New-Item -ItemType Directory -Force -Path gui/src-tauri/binaries | Out-Null
Copy-Item "agent/target/release/future-agent.exe" `
          "gui/src-tauri/binaries/future-agent-$triple.exe" -Force

Write-Host "==> Building CLI (standalone binary)" -ForegroundColor Cyan
Push-Location cli
try {
    Invoke-Native { npm run build }
    Invoke-Native { bun build --compile dist/index.js --outfile dist/future.exe --external chromium-bidi }
}
finally { Pop-Location }
# Stage the CLI as a Tauri sidecar (bundle.externalBin), same as the agent, so a
# full `tauri build` would bundle it into the installer. (This portable build
# copies from cli/dist directly below, but keep the staging consistent with CI.)
Copy-Item "cli/dist/future.exe" "gui/src-tauri/binaries/future-$triple.exe" -Force

Write-Host "==> Building GUI (Tauri, no installer)" -ForegroundColor Cyan
# --no-bundle: compile the frontend + release .exe but skip NSIS/MSI packaging.
Push-Location gui
try { Invoke-Native { npm run tauri:build -- --no-bundle } }
finally { Pop-Location }

Write-Host "==> Assembling portable package" -ForegroundColor Cyan
$stage = Join-Path $Root "futureos-portable"
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force -Path $stage | Out-Null

# The agent sidecar is renamed without the triple (as Tauri does on bundling)
# so the GUI finds it next to its own exe.
Copy-Item "gui/src-tauri/target/release/futureos.exe"       (Join-Path $stage "FutureOS.exe")     -Force
Copy-Item "gui/src-tauri/binaries/future-agent-$triple.exe" (Join-Path $stage "future-agent.exe") -Force
Copy-Item "cli/dist/future.exe"                         (Join-Path $stage "future.exe")   -Force
Copy-Item "docs/dist/readme-windows.txt"                    (Join-Path $stage "Readme.txt")       -Force

if (-not $OutDir) { $OutDir = $Root }
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$zip = Join-Path $OutDir "FutureOS-portable-windows.zip"
if (Test-Path $zip) { Remove-Item -Force $zip }
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip -Force
Remove-Item -Recurse -Force $stage

Write-Host ""
Write-Host "Done: $zip" -ForegroundColor Green
Write-Host "  Contents: FutureOS.exe, future-agent.exe, future.exe, Readme.txt"
Write-Host "  Requires the Microsoft Edge WebView2 runtime (bundled with Windows 10/11)."
