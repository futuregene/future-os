#requires -Version 5.1
<#
.SYNOPSIS
    Build the FutureOS Windows installers (NSIS setup + MSI) locally, optionally
    signed — no GitHub Actions.

.DESCRIPTION
    The installer counterpart to build-windows-portable.ps1 (which produces a zip
    of loose .exe files). Steps:
      1. build the agent (release) and stage it as the Tauri sidecar,
      2. compile the CLI into a standalone .exe (bun --compile), stage it too,
      3. sign both sidecars (-Sign only) — see below,
      4. build the GUI with Tauri, producing bundle/nsis/*.exe and bundle/msi/*.msi.

    Signing splits across two mechanisms, which is the whole reason this script
    exists separately:

      * The sidecars are signed here, before the bundle. Tauri copies externalBin
        binaries into the installer as-is, so anything not signed by now ships
        unsigned inside the installer.
      * The app .exe and the two installers are signed by the Tauri bundler
        itself, which calls scripts/sign-file.ps1 through bundle.windows.
        signCommand. We cannot sign those ourselves: they only exist part-way
        through the bundling process.

    signCommand is injected via a generated `tauri build --config` overlay rather
    than committed to tauri.conf.json, so unsigned builds (local dev, and CI on
    machines without the certificate) keep working untouched.

.PARAMETER SkipDeps
    Skip `npm ci` in gui/ and cli/ (use when node_modules are already current).

.PARAMETER Sign
    Authenticode-sign the sidecars, the app .exe and both installers. Opt-in: a
    plain local build stays unsigned so it needs no certificate.

    Requires a code signing certificate in the CurrentUser\My store. For Certum
    Code Signing in the Cloud that means SimplySign Desktop must be logged in
    (the session is short-lived — log in shortly before building) and the
    virtual card mounted.

.PARAMETER CertSubject
    Substring of the signing certificate's Subject, used to pick one when the
    store holds several code signing certificates. Unnecessary when there is
    exactly one. Match it against the real Subject — run
    `Get-ChildItem Cert:\CurrentUser\My -CodeSigningCert | Format-List Subject`
    to see it; a CA may romanize the company name rather than use it verbatim.

.PARAMETER TimestampUrl
    RFC 3161 timestamp server. Timestamping is what keeps signatures valid after
    the certificate expires, so it is on by default.

.EXAMPLE
    pwsh scripts/build-windows-installer.ps1
    pwsh scripts/build-windows-installer.ps1 -Sign
    pwsh scripts/build-windows-installer.ps1 -Sign -SkipDeps
#>
[CmdletBinding()]
param(
    [switch]$SkipDeps,
    [switch]$Sign,
    [string]$CertSubject,
    [string]$TimestampUrl = "http://time.certum.pl/"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root

. "$PSScriptRoot\lib\windows-signing.ps1"

# $ErrorActionPreference="Stop" only stops on *cmdlet* errors, NOT on a native
# command (cargo/npm/bun/tauri) exiting non-zero — those would silently continue
# and produce a broken package. Run every external command through this so a
# failure aborts the build.
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

# Resolve the certificate up front rather than at the signing step: the build
# takes tens of minutes, and a SimplySign session that isn't logged in should
# fail now, not after all that work.
$signTool = $null
$signThumbprint = $null
if ($Sign) {
    $signTool = Find-SignTool
    $cert = Resolve-SigningCert $CertSubject
    $signThumbprint = $cert.Thumbprint
    Write-Host "    signtool: $signTool"
    Write-Host "    cert    : $($cert.Subject)"
    Write-Host "    expires : $($cert.NotAfter.ToString('yyyy-MM-dd'))"
}

# Host target triple, e.g. x86_64-pc-windows-msvc. Tauri looks for the sidecar
# named future-agent-<triple>.exe (bundle.externalBin in tauri.conf.json).
$hostLine = (rustc -Vv) | Select-String '^host:'
if (-not $hostLine) { throw "Could not read host triple from 'rustc -Vv'." }
$triple = $hostLine.Line.Split(' ')[1]
Write-Host "    host triple: $triple"

# Resolve ONE version string and pin it for every sub-build (agent build.rs,
# CLI gen-version, GUI build.rs) so they all agree.
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
Copy-Item "cli/dist/future.exe" "gui/src-tauri/binaries/future-$triple.exe" -Force

# Sign the sidecars now: the bundler embeds them as-is, so this is the last
# moment they can be signed without unpacking the installer afterwards.
if ($Sign) {
    Write-Host "==> Signing sidecars" -ForegroundColor Cyan
    foreach ($exe in @("future-agent-$triple.exe", "future-$triple.exe")) {
        $p = Join-Path "$Root\gui\src-tauri\binaries" $exe
        Invoke-SignFile -SignTool $signTool -Thumbprint $signThumbprint -Path $p -TimestampUrl $TimestampUrl
        Write-Host "    signed: $exe"
    }
}

Write-Host "==> Building GUI installers (Tauri)" -ForegroundColor Cyan
$overlay = $null
$tauriArgs = @()
if ($Sign) {
    # Hand the bundler a signCommand pointing back at scripts/sign-file.ps1.
    # Absolute paths throughout: the bundler's working directory is its own
    # business, and this file is generated fresh per build anyway.
    #
    # Object notation, not the string form: Tauri splits the string form on
    # spaces, which would corrupt any path containing one.
    #
    # Reuse the PowerShell host running this script rather than assuming pwsh is
    # on PATH inside the bundler's environment.
    $psExe = (Get-Process -Id $PID).Path
    $signScript = Join-Path $PSScriptRoot "sign-file.ps1"
    $overlayObj = @{
        bundle = @{
            windows = @{
                signCommand = @{
                    cmd  = $psExe
                    args = @(
                        "-NoProfile", "-ExecutionPolicy", "Bypass",
                        "-File", $signScript,
                        "-Thumbprint", $signThumbprint,
                        "-TimestampUrl", $TimestampUrl,
                        "-Path", "%1"
                    )
                }
            }
        }
    }
    $overlay = Join-Path ([System.IO.Path]::GetTempPath()) "futureos-sign-overlay-$PID.json"
    $overlayObj | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $overlay -Encoding utf8
    Write-Host "    signCommand overlay: $overlay"
    $tauriArgs = @("--config", $overlay)
}

Push-Location gui
try { Invoke-Native { npm run tauri:build -- @tauriArgs } }
finally {
    Pop-Location
    if ($overlay -and (Test-Path $overlay)) { Remove-Item -Force $overlay }
}

Write-Host "==> Installers" -ForegroundColor Cyan
$bundle = Join-Path $Root "gui\src-tauri\target\release\bundle"
$artifacts = @(Get-ChildItem -Path (Join-Path $bundle "nsis\*.exe"), (Join-Path $bundle "msi\*.msi") `
                             -ErrorAction SilentlyContinue)
if (-not $artifacts) { throw "Tauri produced no installers under $bundle." }

foreach ($a in $artifacts) {
    if ($Sign) {
        # Independent check that the bundler really did call signCommand — a
        # silently-unsigned installer is exactly the failure worth catching here.
        & $signTool verify /pa /q $a.FullName
        $state = if ($LASTEXITCODE -eq 0) { "signed" } else { "UNSIGNED" }
    } else {
        $state = "unsigned"
    }
    Write-Host ("    {0,-8} {1}" -f $state, $a.FullName)
}

Write-Host ""
Write-Host "Done." -ForegroundColor Green
Write-Host "  Installers require the Microsoft Edge WebView2 runtime (bundled with Windows 10/11)."
