#requires -Version 5.1
<#
.SYNOPSIS
    Collect and assert FutureOS Windows packaged-app lifecycle checkpoints.

.DESCRIPTION
    Manual-only companion to test-windows-sandbox.ps1. It does not click UI,
    stop processes, uninstall software, or change arbitrary ACLs. The tester
    performs each documented portable/installer action and invokes this script
    at the checkpoint that follows it.

    SeedCleanupFixture writes one valid FutureOS capability metadata record
    whose target is a dedicated directory under TEMP. It does not add an ACE.
    This safely proves that packaged shutdown/startup/uninstall really reaches
    the already-native-tested reset path while the Windows product gate remains
    closed. Seeding refuses to overwrite any non-empty capability state.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File .\scripts\test-windows-sandbox-lifecycle.ps1 -Action Snapshot
    powershell -ExecutionPolicy Bypass -File .\scripts\test-windows-sandbox-lifecycle.ps1 -Action ExpectBundled
    powershell -ExecutionPolicy Bypass -File .\scripts\test-windows-sandbox-lifecycle.ps1 -Action SeedCleanupFixture
    powershell -ExecutionPolicy Bypass -File .\scripts\test-windows-sandbox-lifecycle.ps1 -Action ExpectStopped
#>
[CmdletBinding()]
param(
    [ValidateSet(
        "Snapshot",
        "SeedCleanupFixture",
        "ExpectClean",
        "ExpectBundled",
        "ExpectStopped",
        "ExpectExternalAttached",
        "ExpectExternalSurvives",
        "ExpectRecovered"
    )]
    [string]$Action = "Snapshot",
    [string]$OutputDirectory = "target/windows-sandbox-results",
    [int]$TimeoutSeconds = 20,
    [switch]$AllowElevated
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ($env:OS -ne "Windows_NT") {
    throw "This script must run on Windows."
}
if ($TimeoutSeconds -lt 0 -or $TimeoutSeconds -gt 300) {
    throw "TimeoutSeconds must be between 0 and 300."
}

$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $repoRoot

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($identity)
$isElevated = $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if ($isElevated -and -not $AllowElevated) {
    throw "Run lifecycle validation from a normal, non-administrator PowerShell."
}

$outputRoot = if ([IO.Path]::IsPathRooted($OutputDirectory)) {
    $OutputDirectory
} else {
    Join-Path $repoRoot $OutputDirectory
}
New-Item -ItemType Directory -Force -Path $outputRoot | Out-Null
$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$logPath = Join-Path $outputRoot "windows-sandbox-lifecycle-$stamp.log"

function Write-LogLine {
    param([string]$Text)
    $Text | Tee-Object -FilePath $logPath -Append
}

function Get-AgentHomeDirectory {
    foreach ($candidate in @($env:HOME, $env:USERPROFILE)) {
        if (-not [string]::IsNullOrWhiteSpace($candidate) -and [IO.Path]::IsPathRooted($candidate)) {
            return [IO.Path]::GetFullPath($candidate)
        }
    }
    $profile = [Environment]::GetFolderPath([Environment+SpecialFolder]::UserProfile)
    if ([string]::IsNullOrWhiteSpace($profile)) {
        throw "Windows user profile directory is unavailable"
    }
    return [IO.Path]::GetFullPath($profile)
}

$agentHome = Get-AgentHomeDirectory
$capabilityStatePath = Join-Path $agentHome ".future/windows-capabilities.json"

function Read-CapabilityState {
    if (-not (Test-Path -LiteralPath $capabilityStatePath)) {
        return $null
    }
    try {
        return Get-Content -LiteralPath $capabilityStatePath -Raw | ConvertFrom-Json
    } catch {
        throw "Windows capability state is unreadable: $($_.Exception.Message)"
    }
}

function Get-LifecycleState {
    $all = @(Get-CimInstance Win32_Process)
    $apps = @($all | Where-Object { $_.Name -ieq "futureos.exe" })
    $agents = @($all | Where-Object {
        $_.Name -ieq "future.exe" -and
        -not [string]::IsNullOrWhiteSpace($_.CommandLine) -and
        $_.CommandLine -match '(?i)(?:^|[\s"])(?:agent)(?:[\s"]|$)'
    })
    $appIds = @($apps | ForEach-Object { [uint32]$_.ProcessId })
    $bundled = @($agents | Where-Object { $appIds -contains [uint32]$_.ParentProcessId })
    $external = @($agents | Where-Object { $appIds -notcontains [uint32]$_.ParentProcessId })
    $state = Read-CapabilityState
    $recordCount = if ($null -eq $state) { 0 } else { @($state.records).Count }
    return [pscustomobject]@{
        Apps = $apps
        Agents = $agents
        BundledAgents = $bundled
        ExternalAgents = $external
        CapabilityRecords = $recordCount
    }
}

function Write-StateSnapshot {
    param($State)
    Write-LogLine "Desktop processes: $(@($State.Apps).Count)"
    foreach ($process in @($State.Apps)) {
        Write-LogLine "  FutureOS PID=$($process.ProcessId) ParentPID=$($process.ParentProcessId)"
    }
    Write-LogLine "Agent processes: $(@($State.Agents).Count)"
    foreach ($process in @($State.Agents)) {
        $owner = if (@($State.BundledAgents | Where-Object { $_.ProcessId -eq $process.ProcessId }).Count -eq 1) {
            "bundled"
        } else {
            "external"
        }
        Write-LogLine "  future agent PID=$($process.ProcessId) ParentPID=$($process.ParentProcessId) Owner=$owner"
    }
    Write-LogLine "Persisted capability records: $($State.CapabilityRecords)"
}

function Get-ExpectationErrors {
    param([string]$Expected, $State)
    $errors = New-Object System.Collections.Generic.List[string]
    $appCount = @($State.Apps).Count
    $agentCount = @($State.Agents).Count
    $bundledCount = @($State.BundledAgents).Count
    $externalCount = @($State.ExternalAgents).Count

    switch ($Expected) {
        "ExpectClean" {
            if ($appCount -ne 0) { $errors.Add("expected no FutureOS process, found $appCount") }
            if ($agentCount -ne 0) { $errors.Add("expected no Agent process, found $agentCount") }
            if ($State.CapabilityRecords -ne 0) { $errors.Add("expected zero capability records, found $($State.CapabilityRecords)") }
        }
        "ExpectBundled" {
            if ($appCount -ne 1) { $errors.Add("expected one FutureOS process, found $appCount") }
            if ($agentCount -ne 1) { $errors.Add("expected one Agent process, found $agentCount") }
            if ($bundledCount -ne 1) { $errors.Add("expected the Agent parent to be FutureOS, bundled count is $bundledCount") }
            if ($externalCount -ne 0) { $errors.Add("expected no external Agent, found $externalCount") }
        }
        "ExpectStopped" {
            if ($appCount -ne 0) { $errors.Add("expected FutureOS to be stopped, found $appCount process(es)") }
            if ($agentCount -ne 0) { $errors.Add("expected bundled Agent to be stopped, found $agentCount Agent process(es)") }
            if ($State.CapabilityRecords -ne 0) { $errors.Add("expected shutdown cleanup to remove all records, found $($State.CapabilityRecords)") }
        }
        "ExpectExternalAttached" {
            if ($appCount -ne 1) { $errors.Add("expected one FutureOS process, found $appCount") }
            if ($agentCount -ne 1) { $errors.Add("expected one Agent process, found $agentCount") }
            if ($bundledCount -ne 0) { $errors.Add("FutureOS unexpectedly owns an Agent process") }
            if ($externalCount -ne 1) { $errors.Add("expected one external Agent, found $externalCount") }
        }
        "ExpectExternalSurvives" {
            if ($appCount -ne 0) { $errors.Add("expected FutureOS to be stopped, found $appCount process(es)") }
            if ($agentCount -ne 1) { $errors.Add("expected the external Agent to survive, found $agentCount") }
            if ($bundledCount -ne 0) { $errors.Add("a stopped FutureOS cannot own the surviving Agent") }
            if ($externalCount -ne 1) { $errors.Add("expected one external Agent, found $externalCount") }
            if ($State.CapabilityRecords -ne 1) { $errors.Add("expected Desktop to leave the external Agent's one fixture record untouched, found $($State.CapabilityRecords)") }
        }
        "ExpectRecovered" {
            if ($appCount -ne 1) { $errors.Add("expected one relaunched FutureOS process, found $appCount") }
            if ($agentCount -ne 1) { $errors.Add("expected one recovered Agent process, found $agentCount") }
            if ($bundledCount -ne 1) { $errors.Add("expected the recovered Agent to be owned by FutureOS") }
            if ($externalCount -ne 0) { $errors.Add("expected no external Agent after recovery, found $externalCount") }
            if ($State.CapabilityRecords -ne 0) { $errors.Add("expected startup recovery to remove all records, found $($State.CapabilityRecords)") }
        }
    }
    return $errors
}

function Seed-CleanupFixture {
    $existing = Read-CapabilityState
    $existingCount = if ($null -eq $existing) { 0 } else { @($existing.records).Count }
    if ($existingCount -ne 0) {
        throw "Refusing to seed over $existingCount existing capability record(s). Reset or finish active tests first."
    }

    $fixture = Join-Path ([IO.Path]::GetFullPath($env:TEMP)) "FutureOS-Sandbox-Lifecycle-Fixture"
    New-Item -ItemType Directory -Force -Path $fixture | Out-Null
    $record = [ordered]@{
        name = "futureos.windows.lifecycle-fixture-v1"
        kind = "policy"
        policy_fingerprint = ("0" * 64)
        writable_root = $fixture
    }
    $document = [ordered]@{
        schema_version = 1
        records = @($record)
    }
    $parent = Split-Path -Parent $capabilityStatePath
    New-Item -ItemType Directory -Force -Path $parent | Out-Null
    $temporary = "$capabilityStatePath.lifecycle-$PID.tmp"
    $json = $document | ConvertTo-Json -Depth 8
    $utf8 = New-Object System.Text.UTF8Encoding($false)
    try {
        [IO.File]::WriteAllText($temporary, $json, $utf8)
        Move-Item -LiteralPath $temporary -Destination $capabilityStatePath -Force
    } finally {
        if (Test-Path -LiteralPath $temporary) {
            Remove-Item -LiteralPath $temporary -Force
        }
    }
    $seeded = Read-CapabilityState
    if (@($seeded.records).Count -ne 1) {
        throw "Lifecycle cleanup fixture was not persisted correctly"
    }
    Write-LogLine "Seeded one cleanup fixture record at: $capabilityStatePath"
    Write-LogLine "Fixture target: $fixture"
}

try {
    Write-LogLine "FutureOS Windows sandbox packaged lifecycle report"
    Write-LogLine "Started: $((Get-Date).ToString('o'))"
    Write-LogLine "Action: $Action"
    Write-LogLine "Repository: $repoRoot"
    Write-LogLine "Git revision: $(& git rev-parse HEAD)"
    Write-LogLine "User: $($identity.Name)"
    Write-LogLine "Elevated: $isElevated"
    Write-LogLine "Agent home: $agentHome"
    Write-LogLine "Capability state: $capabilityStatePath"

    if ($Action -eq "SeedCleanupFixture") {
        Seed-CleanupFixture
        $state = Get-LifecycleState
        Write-StateSnapshot $state
        Write-LogLine "RESULT: SEEDED"
        Write-LogLine "Finished: $((Get-Date).ToString('o'))"
        Write-Host "SEEDED. Perform the documented exit/restart/uninstall action, then run its Expect* checkpoint." -ForegroundColor Yellow
        Write-Host "Report: $logPath"
        exit 0
    }

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $state = Get-LifecycleState
        # In Windows PowerShell 5.1, assigning an empty array emitted by an
        # `if` statement produces $null. Keep an actual array so StrictMode can
        # safely read Count for the observation-only Snapshot action.
        $errors = @()
        if ($Action -ne "Snapshot") {
            $errors = @(Get-ExpectationErrors $Action $state)
        }
        if ($errors.Count -eq 0) { break }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)

    Write-StateSnapshot $state
    if ($errors.Count -ne 0) {
        foreach ($message in $errors) { Write-LogLine "ASSERTION: $message" }
        throw "$Action did not reach the expected lifecycle state within $TimeoutSeconds second(s)"
    }
    Write-LogLine "RESULT: PASS"
    Write-LogLine "Finished: $((Get-Date).ToString('o'))"
    Write-Host "PASS: $Action" -ForegroundColor Green
    Write-Host "Report: $logPath"
    exit 0
} catch {
    Write-LogLine "RESULT: FAIL"
    Write-LogLine "ERROR: $($_.Exception.Message)"
    Write-LogLine "Finished: $((Get-Date).ToString('o'))"
    Write-Host "FAIL: $Action. Report: $logPath" -ForegroundColor Red
    exit 1
}
