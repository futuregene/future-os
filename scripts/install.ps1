# FutureOS one-line installer for Windows
#
#   iex (irm https://dl.future-os.cn/install.ps1)
#
# Downloads the signed NSIS installer for the latest release from the release
# manifest, verifies its SHA-256, runs it silently, then executes `future init`
# and the interactive `future config` provider setup.
#
# Env overrides:
#   FUTUREOS_VERSION  pin a specific release (e.g. v0.1.2); default = latest
#   FUTUREOS_BASE     release mirror base URL; default https://dl.future-os.cn/releases
$ErrorActionPreference = 'Stop'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$Base = if ($env:FUTUREOS_BASE) { $env:FUTUREOS_BASE } else { 'https://dl.future-os.cn/releases' }
$Version = $env:FUTUREOS_VERSION

if ($Version) {
    # Signed release installers use the canonical name without a signing suffix.
    $Url = "$Base/$Version/FutureOS_${Version}_x64-setup.exe"
    $Sha = $null
} else {
    $latest = Invoke-RestMethod "$Base/latest.json"
    $Version = $latest.version
    $asset = $latest.assets.'windows-x86_64'
    $Url = $asset.url
    $Sha = $asset.sha256
}

Write-Host "==> Installing FutureOS $Version (windows-x86_64)" -ForegroundColor Green
$exe = Join-Path $env:TEMP "FutureOS_${Version}_setup.exe"
Invoke-WebRequest -Uri $Url -OutFile $exe

if ($Sha) {
    $got = (Get-FileHash -Path $exe -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($got -ne $Sha) {
        throw "Checksum mismatch for $Url (got $got, expected $Sha)"
    }
    Write-Host "==> Checksum verified" -ForegroundColor Green
}

Write-Host "==> Running installer (silent)..." -ForegroundColor Green
$p = Start-Process -FilePath $exe -ArgumentList '/S' -Wait -PassThru
if ($p.ExitCode -ne 0) {
    throw "Installer exited with code $($p.ExitCode)"
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

    Write-Host "==> Configuring a model provider..." -ForegroundColor Green
    & $futureExe config
    if ($LASTEXITCODE -ne 0) {
        Write-Warning "Model provider setup did not complete. Retry with: `"$futureExe`" config"
    }
} else {
    Write-Warning "Bundled future.exe was not found. Run 'future init' and 'future config' manually."
}

Write-Host ""
Write-Host "FutureOS $Version installed." -ForegroundColor Green
Write-Host "  - Launch 'FutureOS' from the Start menu; the desktop app auto-starts the agent."
Write-Host "  - Use the bundled CLI for 'future agent|tui|channel|loop'."
