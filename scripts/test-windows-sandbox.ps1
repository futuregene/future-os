[CmdletBinding()]
param(
    [string]$OutputDirectory = "target/windows-sandbox-results",
    [switch]$IncludeClippy,
    [switch]$AllowElevated
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ($env:OS -ne "Windows_NT") {
    throw "This script must run on Windows."
}

$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $repoRoot

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($identity)
$isElevated = $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if ($isElevated -and -not $AllowElevated) {
    throw "Run this script from a normal, non-administrator PowerShell. Use -AllowElevated only for diagnosis; an elevated pass does not validate the unelevated product mode."
}

$outputRoot = if ([IO.Path]::IsPathRooted($OutputDirectory)) {
    $OutputDirectory
} else {
    Join-Path $repoRoot $OutputDirectory
}
New-Item -ItemType Directory -Force -Path $outputRoot | Out-Null
$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$logPath = Join-Path $outputRoot "windows-sandbox-$stamp.log"

function Write-LogLine {
    param([string]$Text)
    $Text | Tee-Object -FilePath $logPath -Append
}

function Invoke-LoggedNative {
    param(
        [string]$Label,
        [string]$FilePath,
        [string[]]$Arguments,
        [string]$RequiredOutput = ""
    )

    Write-LogLine ""
    Write-LogLine "=== $Label ==="
    Write-LogLine ("COMMAND: {0} {1}" -f $FilePath, ($Arguments -join " "))
    # Windows PowerShell 5.1 wraps every native stderr line in an ErrorRecord.
    # With the script-wide Stop preference, harmless Cargo progress such as
    # "Updating crates.io index" would otherwise terminate this function before
    # Cargo can run its tests. Native success/failure is authoritative through
    # LASTEXITCODE; stderr remains captured in the report.
    $previousErrorActionPreference = $ErrorActionPreference
    $global:LASTEXITCODE = $null
    try {
        $ErrorActionPreference = "Continue"
        $lines = @(& $FilePath @Arguments 2>&1)
        $exitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }
    foreach ($line in $lines) {
        Write-LogLine ([string]$line)
    }
    if ($null -eq $exitCode) {
        throw "$Label did not start or did not report an exit code"
    }
    Write-LogLine "EXIT CODE: $exitCode"
    if ($exitCode -ne 0) {
        throw "$Label failed with exit code $exitCode"
    }
    if ($RequiredOutput -and -not ($lines | Where-Object { ([string]$_).Contains($RequiredOutput) })) {
        throw "$Label did not report the required result: $RequiredOutput"
    }
}

function Get-WindowsCapabilityRecordCount {
    param([string]$StatePath)

    if (-not (Test-Path -LiteralPath $StatePath)) {
        return 0
    }
    try {
        $state = Get-Content -LiteralPath $StatePath -Raw | ConvertFrom-Json
        return @($state.records).Count
    } catch {
        throw "Windows capability state is unreadable: $($_.Exception.Message)"
    }
}

try {
    Write-LogLine "FutureOS Windows unelevated sandbox integration report"
    Write-LogLine "Started: $((Get-Date).ToString('o'))"
    Write-LogLine "Repository: $repoRoot"
    Write-LogLine "PowerShell: $($PSVersionTable.PSVersion) ($($PSVersionTable.PSEdition))"
    Write-LogLine "OS: $([System.Environment]::OSVersion.VersionString)"
    Write-LogLine "Architecture: $env:PROCESSOR_ARCHITECTURE"
    Write-LogLine "User: $($identity.Name)"
    Write-LogLine "Elevated: $isElevated"
    $capabilityStatePath = Join-Path $env:USERPROFILE ".future/windows-capabilities.json"
    $initialCapabilityRecords = Get-WindowsCapabilityRecordCount $capabilityStatePath
    Write-LogLine "Initial persisted Windows capability records: $initialCapabilityRecords"

    $tempRoot = [IO.Path]::GetPathRoot([IO.Path]::GetFullPath($env:TEMP))
    $driveLetter = $tempRoot.TrimEnd('\').TrimEnd(':')
    try {
        $tempVolume = Get-Volume -DriveLetter $driveLetter -ErrorAction Stop
        Write-LogLine "TEMP volume: $tempRoot ($($tempVolume.FileSystem))"
        if ($tempVolume.FileSystem -ne "NTFS") {
            throw "The integration fixture requires local NTFS, but TEMP is on $($tempVolume.FileSystem)."
        }
    } catch {
        Write-LogLine "TEMP volume probe failed: $($_.Exception.Message)"
        throw
    }

    Invoke-LoggedNative "Git revision" "git" @("rev-parse", "HEAD")
    Invoke-LoggedNative "Git worktree status" "git" @("status", "--short")
    Invoke-LoggedNative "Rust compiler" "rustc" @("-Vv")
    Invoke-LoggedNative "Cargo" "cargo" @("-V")

    # Intentionally manual-only. This command is not added to GitHub Actions.
    # A single test thread avoids overlapping ACL mutations between fixtures.
    Invoke-LoggedNative `
        "Windows sandbox native and end-to-end integration matrix" `
        "cargo" `
        @(
            "test",
            "--manifest-path", "agent/Cargo.toml",
            "sandbox::windows",
            "--",
            "--nocapture",
            "--test-threads=1"
        )

    $restrictedTokenUnsupported = Select-String `
        -Path $logPath `
        -SimpleMatch `
        -Quiet `
        -Pattern "SKIP: Windows host rejected CreateRestrictedToken"

    Invoke-LoggedNative `
        "Windows capability approval and receipt binding" `
        "cargo" `
        @(
            "test",
            "--manifest-path", "agent/Cargo.toml",
            "windows_capability",
            "--",
            "--nocapture",
            "--test-threads=1"
        )

    Invoke-LoggedNative `
        "Agent user-scoped singleton lifecycle" `
        "cargo" `
        @(
            "test",
            "--manifest-path", "agent/Cargo.toml",
            "--test", "cli_smoke",
            "agent_is_singleton_per_user_even_on_different_ports",
            "--",
            "--nocapture",
            "--test-threads=1"
        )

    if ($IncludeClippy) {
        Invoke-LoggedNative `
            "Agent Clippy" `
            "cargo" `
            @(
                "clippy",
                "--manifest-path", "agent/Cargo.toml",
                "--all-targets",
                "--",
                "-D", "warnings"
            )
    }

    # tauri-build requires the configured externalBin to exist even for a Rust
    # unit test. A clean checkout does not contain build artifacts, so create an
    # empty host-triple placeholder and remove it afterward only if this script
    # created it. Never overwrite or delete a real locally-built sidecar.
    $hostTripleLine = (& rustc -Vv | Select-String '^host:' | Select-Object -First 1)
    if ($null -eq $hostTripleLine) {
        throw "rustc -Vv did not report a host triple"
    }
    $hostTriple = ([string]$hostTripleLine).Substring(5).Trim()
    $sidecarDirectory = Join-Path $repoRoot "desktop/src-tauri/binaries"
    $sidecarPath = Join-Path $sidecarDirectory "future-$hostTriple.exe"
    $createdSidecarPlaceholder = $false
    if (-not (Test-Path -LiteralPath $sidecarPath)) {
        New-Item -ItemType Directory -Force -Path $sidecarDirectory | Out-Null
        New-Item -ItemType File -Force -Path $sidecarPath | Out-Null
        $createdSidecarPlaceholder = $true
    }
    try {
        Invoke-LoggedNative `
            "Desktop graceful Agent shutdown lifecycle" `
            "cargo" `
            @(
                "test",
                "--manifest-path", "desktop/src-tauri/Cargo.toml",
                "agent_supervisor::tests::graceful_shutdown_",
                "--",
                "--nocapture",
                "--test-threads=1"
            )

        if ($IncludeClippy) {
            Invoke-LoggedNative `
                "Desktop backend Clippy" `
                "cargo" `
                @(
                    "clippy",
                    "--manifest-path", "desktop/src-tauri/Cargo.toml",
                    "--all-targets",
                    "--",
                    "-D", "warnings"
                )
        }
    } finally {
        if ($createdSidecarPlaceholder -and (Test-Path -LiteralPath $sidecarPath)) {
            Remove-Item -LiteralPath $sidecarPath -Force
        }
    }

    if (-not $restrictedTokenUnsupported) {
        Invoke-LoggedNative `
            "Packaged-sidecar Windows sandbox release probe" `
            "cargo" `
            @(
                "run", "--quiet",
                "--manifest-path", "cli/Cargo.toml",
                "--", "agent", "--probe-windows-sandbox"
            ) `
            '"available":true'
    }

    $remainingCapabilityRecords = Get-WindowsCapabilityRecordCount $capabilityStatePath
    Write-LogLine "Remaining persisted Windows capability records: $remainingCapabilityRecords"
    if ($remainingCapabilityRecords -ne 0) {
        throw "Windows sandbox lifecycle left $remainingCapabilityRecords persisted capability record(s)"
    }

    Write-LogLine ""
    if ($restrictedTokenUnsupported) {
        Write-LogLine "RESULT: UNSUPPORTED"
        Write-LogLine "This Windows host rejected CreateRestrictedToken. No command was run without the sandbox."
        Write-LogLine "Finished: $((Get-Date).ToString('o'))"
        Write-Host ""
        Write-Host "UNSUPPORTED. Send this log back for review: $logPath" -ForegroundColor Yellow
        exit 2
    }
    Write-LogLine "RESULT: PASS"
    Write-LogLine "Finished: $((Get-Date).ToString('o'))"
    Write-Host ""
    Write-Host "PASS. Send this log back for review: $logPath" -ForegroundColor Green
    exit 0
} catch {
    Write-LogLine ""
    Write-LogLine "RESULT: FAIL"
    Write-LogLine "ERROR: $($_.Exception.Message)"
    Write-LogLine "Finished: $((Get-Date).ToString('o'))"
    Write-Host ""
    Write-Host "FAIL. Send this log back for review: $logPath" -ForegroundColor Red
    exit 1
}
