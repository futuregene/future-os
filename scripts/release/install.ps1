# FutureOS one-line installer for Windows
#
#   iex (irm https://dl.future-os.cn/install.ps1)
#
# Downloads the signed NSIS installer for the latest release from the release
# manifest, verifies its SHA-256, runs it silently, then executes `future init`.
#
# Env overrides:
#   FUTUREOS_VERSION  pin a specific release (e.g. v0.1.2); default = latest
#   FUTUREOS_BASE     release mirror base URL; default https://dl.future-os.cn/releases
#   FUTUREOS_SKIP_VERIFY=1  install without SHA-256 verification (not recommended)
$ErrorActionPreference = 'Stop'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

# An unverified download leaves only TLS to the mirror, so the installer refuses
# one unless FUTUREOS_SKIP_VERIFY is set explicitly (mirrors install.sh).
$SkipVerify = [bool]($env:FUTUREOS_SKIP_VERIFY -and $env:FUTUREOS_SKIP_VERIFY -notin @('0', 'false', 'no'))

function Assert-VerifiedOrExplicit {
    param($Reason)
    if ($SkipVerify) {
        Write-Warning "$Reason - installing WITHOUT SHA-256 verification (FUTUREOS_SKIP_VERIFY is set)"
        return
    }
    throw "$Reason, so this download cannot be verified. Install the latest release instead, or set FUTUREOS_SKIP_VERIFY=1 to accept the risk."
}

$Base = if ($env:FUTUREOS_BASE) { $env:FUTUREOS_BASE } else { 'https://dl.future-os.cn/releases' }
$Version = $env:FUTUREOS_VERSION -replace '^[vV]', ''

if ($Version) {
    # Releases publish releases/<version>/latest.json beside the artifacts, so a
    # pinned install reads that manifest instead of trusting a URL we construct.
    # Signed release installers use the canonical name without a signing suffix.
    $manifest = $null
    try { $manifest = Invoke-RestMethod "$Base/$Version/latest.json" } catch { $manifest = $null }
    $asset = $manifest.assets.'windows-x86_64'
    if ($asset -and $asset.url -and $asset.sha256) {
        $Url = $asset.url
        $Sha = $asset.sha256
    } else {
        Assert-VerifiedOrExplicit "$Base/$Version/latest.json publishes no windows-x86_64 SHA-256"
        $Url = "$Base/$Version/FutureOS_${Version}_x64-setup.exe"
        $Sha = $null
    }
} else {
    $latest = Invoke-RestMethod "$Base/latest.json"
    $Version = $latest.version
    $asset = $latest.assets.'windows-x86_64'
    if ($asset -and $asset.url -and $asset.sha256) {
        $Url = $asset.url
        $Sha = $asset.sha256
    } else {
        Assert-VerifiedOrExplicit "$Base/latest.json publishes no windows-x86_64 SHA-256"
        $Url = "$Base/$Version/FutureOS_${Version}_x64-setup.exe"
        $Sha = $null
    }
}

Write-Host "==> Installing FutureOS $Version (windows-x86_64)" -ForegroundColor Green
$exe = Join-Path $env:TEMP "FutureOS_${Version}_setup.exe"
Invoke-WebRequest -Uri $Url -OutFile $exe

if ($Sha) {
    # Get-FileHash ships with PowerShell 4.0+; a missing hasher fails closed like
    # a missing shasum/sha256sum does in install.sh.
    if (-not (Get-Command Get-FileHash -ErrorAction SilentlyContinue)) {
        Assert-VerifiedOrExplicit 'Get-FileHash (PowerShell 4.0+) is not available to verify the download'
    } else {
        $got = (Get-FileHash -Path $exe -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($got -ne $Sha) {
            throw "Checksum mismatch for $Url (got $got, expected $Sha)"
        }
        Write-Host "==> Checksum verified" -ForegroundColor Green
    }
}

Write-Host "==> Running installer (silent)..." -ForegroundColor Green
$p = Start-Process -FilePath $exe -ArgumentList '/S' -Wait -PassThru
if ($p.ExitCode -eq 32) {
    throw "FutureOS is still running or its files are locked. Save your work, exit FutureOS (including the system tray and Future terminals), then retry. Or run `"$exe`" interactively for guided setup. No program files were replaced by the preflight check."
}
if ($p.ExitCode -eq 5) {
    throw "Setup could not verify write access to the install folder. Run `"$exe`" interactively to see recovery instructions and choose a writable folder."
}
if ($p.ExitCode -ne 0) {
    throw "Installer exited with code $($p.ExitCode). Run `"$exe`" interactively for details."
}

# Tauri's current-user NSIS mode normally installs under
# %LOCALAPPDATA%\FutureOS. Keep the other candidates for existing installs and
# future packaging-layout changes.
$futureCandidates = @(
    (Join-Path $env:LOCALAPPDATA 'FutureOS\future.exe'),
    (Join-Path $env:LOCALAPPDATA 'Programs\FutureOS\future.exe'),
    (Join-Path $env:USERPROFILE '.future\bin\future.exe')
)
$futureExe = $futureCandidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
if (-not $futureExe) {
    $futureCommand = Get-Command future.exe -ErrorAction SilentlyContinue
    if ($futureCommand) { $futureExe = $futureCommand.Source }
}

if ($futureExe) {
    Write-Host "==> Initializing FutureOS..." -ForegroundColor Green
    & $futureExe init
    if ($LASTEXITCODE -ne 0) {
        Write-Warning "future init did not complete. Retry with: `"$futureExe`" init"
    }
    # Interactive provider setup (`future config`) is intentionally skipped
    # until just before the release ships.
} else {
    Write-Warning "Bundled future.exe was not found. Run 'future init' and 'future config' manually."
}

Write-Host ""
Write-Host "FutureOS $Version installed." -ForegroundColor Green
Write-Host "  - Launch 'FutureOS' from the Start menu; the desktop app auto-starts the agent."
Write-Host "  - Use the bundled CLI for 'future agent|tui|channel|goal'."
