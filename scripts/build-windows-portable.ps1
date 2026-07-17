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

.PARAMETER Sign
    Authenticode-sign the three .exe files before zipping. Opt-in: a plain local
    build stays unsigned so it needs no certificate.

    Requires a code signing certificate in the CurrentUser\My store. For Certum
    Code Signing in the Cloud that means SimplySign Desktop must be logged in
    (the session is short-lived — log in shortly before building) and the
    virtual card mounted. The certificate is resolved at run time rather than
    pinned by thumbprint, so a certificate renewal doesn't break the build.

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
    pwsh scripts/build-windows-portable.ps1
    pwsh scripts/build-windows-portable.ps1 -SkipDeps -OutDir C:\builds
    pwsh scripts/build-windows-portable.ps1 -Sign
    pwsh scripts/build-windows-portable.ps1 -Sign -CertSubject "<part of the cert Subject>"
#>
[CmdletBinding()]
param(
    [switch]$SkipDeps,
    [string]$OutDir,
    [switch]$Sign,
    [string]$CertSubject,
    [string]$TimestampUrl = "http://time.certum.pl/"
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

# signtool ships with the Windows SDK and is normally not on PATH. Its directory
# carries the SDK version, so resolve it instead of pinning one.
function Find-SignTool {
    $onPath = Get-Command signtool.exe -ErrorAction SilentlyContinue
    if ($onPath) { return $onPath.Source }

    $found = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" `
                           -ErrorAction SilentlyContinue | Sort-Object FullName -Descending
    if (-not $found) {
        throw "signtool.exe not found. Install the Windows SDK with the 'Windows SDK Signing Tools' component."
    }
    $found[0].FullName
}

# Pick the signing certificate from the user's store by capability, not by a
# hard-coded thumbprint (which changes on renewal) or company name (which the CA
# may render differently from the registered name).
function Resolve-SigningCert([string]$Subject) {
    $now = Get-Date
    $certs = @(Get-ChildItem Cert:\CurrentUser\My -CodeSigningCert -ErrorAction SilentlyContinue |
               Where-Object { $_.NotBefore -le $now -and $_.NotAfter -gt $now })

    if ($Subject) {
        $certs = @($certs | Where-Object { $_.Subject -like "*$Subject*" })
    }

    if ($certs.Count -eq 0) {
        $hint = if ($Subject) {
            "No valid code signing certificate matches Subject '*$Subject*'."
        } else {
            "No valid code signing certificate in CurrentUser\My."
        }
        throw @"
$hint

For Certum Code Signing in the Cloud the certificate only appears while the
SimplySign session is live. Check that SimplySign Desktop is logged in (the
session expires after a couple of hours) and that the virtual card is mounted:

    certutil -scinfo
    Get-ChildItem Cert:\CurrentUser\My -CodeSigningCert | Format-List Subject, Thumbprint, NotAfter
"@
    }

    if ($certs.Count -gt 1) {
        $list = ($certs | ForEach-Object { "  $($_.Thumbprint)  $($_.Subject)" }) -join "`n"
        throw @"
Found $($certs.Count) code signing certificates — pass -CertSubject to disambiguate:

$list
"@
    }

    $certs[0]
}

function Invoke-SignFile {
    param(
        [Parameter(Mandatory)][string]$SignTool,
        [Parameter(Mandatory)][string]$Thumbprint,
        [Parameter(Mandatory)][string]$Path
    )
    # /fd + /td sha256: SHA-1 is no longer accepted for code signing.
    # /tr (RFC 3161) keeps the signature valid past certificate expiry.
    Invoke-Native { & $SignTool sign /sha1 $Thumbprint /fd sha256 /tr $TimestampUrl /td sha256 /q $Path }
    Invoke-Native { & $SignTool verify /pa /q $Path }
    Write-Host "    signed: $(Split-Path -Leaf $Path)"
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

# Sign the staged copies, not the build outputs, so target/ stays reusable and
# every .exe that ships in the zip carries a signature.
if ($Sign) {
    Write-Host "==> Signing binaries" -ForegroundColor Cyan
    foreach ($exe in @("FutureOS.exe", "future-agent.exe", "future.exe")) {
        Invoke-SignFile -SignTool $signTool -Thumbprint $signThumbprint -Path (Join-Path $stage $exe)
    }
}

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
