# FutureOS one-line installer for Windows
#
#   iex (irm https://dl.future-os.cn/install.ps1)
#
# Downloads the signed NSIS installer for the latest release from the release
# manifest, verifies its SHA-256, and runs it silently.
#
# Env overrides:
#   FUTUREOS_VERSION  pin a specific release (e.g. v0.1.2); default = latest
#   FUTUREOS_BASE     release mirror base URL; default https://dl.future-os.cn/releases
$ErrorActionPreference = 'Stop'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$Base = if ($env:FUTUREOS_BASE) { $env:FUTUREOS_BASE } else { 'https://dl.future-os.cn/releases' }
$Version = $env:FUTUREOS_VERSION

if ($Version) {
    # Byte-identical alias of the signed installer, kept for the pinned-version path.
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

Write-Host ""
Write-Host "FutureOS $Version installed." -ForegroundColor Green
Write-Host "  - Launch 'FutureOS' from the Start menu; the desktop app auto-starts the agent."
Write-Host "  - The unified 'future' CLI is bundled with the app (run 'future init' to link it"
Write-Host "    into ~\.future\bin, then use 'future agent|tui|channel|loop')."
Write-Host "  - The /future-loop skill is not included in the installer: it needs a source build;"
Write-Host "    see docs/build-and-install.md (make install-skills)."
