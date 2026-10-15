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
    This proves that packaged shutdown/startup/uninstall reaches the reset path
    without adding an ACE. Seeding refuses to overwrite non-empty state.
    ExpectEmbedded verifies the host itself owns the Agent lock and that the
    packaged CLI can complete a read-only request. ExpectBundled is an alias.
    Pass InstallDirectory for assertions; FUTURE_HOME selects the data root.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File .\scripts\tests\test-windows-sandbox-lifecycle.ps1 -Action Snapshot
    powershell -ExecutionPolicy Bypass -File .\scripts\tests\test-windows-sandbox-lifecycle.ps1 -Action ExpectEmbedded -InstallDirectory C:\FutureOS
    powershell -ExecutionPolicy Bypass -File .\scripts\tests\test-windows-sandbox-lifecycle.ps1 -Action SeedCleanupFixture
    powershell -ExecutionPolicy Bypass -File .\scripts\tests\test-windows-sandbox-lifecycle.ps1 -Action ExpectStopped -InstallDirectory C:\FutureOS
#>
[CmdletBinding()]
param(
    [ValidateSet(
        "Snapshot",
        "SeedCleanupFixture",
        "ExpectClean",
        "ExpectEmbedded",
        "ExpectBundled",
        "ExpectStopped",
        "ExpectExternalAttached",
        "ExpectExternalSurvives",
        "ExpectRecovered"
    )]
    [string]$Action = "Snapshot",
    [string]$InstallDirectory,
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

$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
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
$stamp = (Get-Date -Format "yyyyMMdd-HHmmss-fff") + "-$PID"
$logPath = Join-Path $outputRoot "windows-sandbox-lifecycle-$stamp.log"

function Write-LogLine {
    param([string]$Text)
    $Text | Tee-Object -FilePath $logPath -Append
}

function Test-AbsolutePath {
    param([string]$Path)
    return $Path -and $Path -match '^(?:[A-Za-z]:[\\/]|[\\/]{2}[^\\/]+[\\/][^\\/]+(?:[\\/]|$))'
}

function Get-AgentHomeDirectory {
    foreach ($candidate in @($env:HOME, $env:USERPROFILE)) {
        if (Test-AbsolutePath $candidate) {
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
$futureRoot = if (Test-AbsolutePath $env:FUTURE_HOME) {
    [IO.Path]::GetFullPath($env:FUTURE_HOME)
} else { Join-Path $agentHome '.future' }
$capabilityStatePath = Join-Path $futureRoot 'windows-capabilities.json'
$instancePath = Join-Path $futureRoot 'agent/agent-instance.json'
$instanceLockPath = Join-Path $futureRoot 'agent/agent-instance.lock'
if ($InstallDirectory) {
    if (-not (Test-AbsolutePath $InstallDirectory)) { throw 'InstallDirectory must be absolute.' }
    $InstallDirectory = [IO.Path]::GetFullPath($InstallDirectory).TrimEnd('\', '/')
} elseif ($Action -notin @('Snapshot', 'SeedCleanupFixture')) {
    throw 'Pass -InstallDirectory to scope lifecycle assertions to the installation under test.'
}

function Test-AgentLockHeld {
    if (-not [IO.File]::Exists($instanceLockPath)) { return $false }
    $stream = $null
    try {
        # Never create, truncate, delete or rewrite the production lock.
        $stream = [IO.FileStream]::new($instanceLockPath, [IO.FileMode]::Open,
            [IO.FileAccess]::ReadWrite, [IO.FileShare]::ReadWrite)
        $stream.Lock(0, 1)
        $stream.Unlock(0, 1)
        return $false
    } catch [IO.IOException] {
        $code = $_.Exception.HResult -band 0xffff
        if ($code -eq 2) { return $false } # File disappeared while the owner exited.
        if ($code -in @(32, 33)) { return $true }
        throw
    } finally {
        if ($null -ne $stream) { $stream.Dispose() }
    }
}

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
    $installed = @($all | Where-Object {
        -not $InstallDirectory -or ($_.ExecutablePath -and
            [IO.Path]::GetDirectoryName($_.ExecutablePath).TrimEnd('\', '/') -ieq $InstallDirectory)
    })
    $apps = @($installed | Where-Object { $_.Name -in @('futureos.exe', 'futureos-headless.exe') })
    $agents = @($installed | Where-Object {
        $_.Name -ieq 'future-agent.exe' -or ($_.Name -ieq 'future.exe' -and
            $_.CommandLine -match '(?i)(?:^|[\s"])(?:agent)(?:[\s"]|$)')
    })
    $appIds = @($apps | ForEach-Object { [uint32]$_.ProcessId })
    $bundled = @($agents | Where-Object { $appIds -contains [uint32]$_.ParentProcessId })
    $held = Test-AgentLockHeld
    $owner = $null
    $ownerError = $null
    if (Test-Path -LiteralPath $instancePath) {
        $process = $null
        try {
            $info = Get-Content -LiteralPath $instancePath -Raw | ConvertFrom-Json
            $process = [Diagnostics.Process]::GetProcessById([int]$info.pid)
            $actualExe = [IO.Path]::GetFullPath($process.MainModule.FileName)
            if ([IO.Path]::GetFileName($actualExe) -notin @('future.exe', 'future-agent.exe', 'futureos.exe', 'futureos-headless.exe', 'future-desktop.exe')) {
                throw 'Agent metadata names an unrecognized host executable.'
            }
            if ($actualExe -ine [IO.Path]::GetFullPath([string]$info.executable) -or
                [IO.Path]::GetFullPath([string]$info.futureHome).TrimEnd('\', '/') -ine $futureRoot.TrimEnd('\', '/') -or
                $process.StartTime.ToFileTimeUtc() -ne [long]$info.startTimeFiletime) {
                throw 'Agent metadata does not match the live executable, creation time and data root.'
            }
            $owner = $all | Where-Object { $_.ProcessId -eq $info.pid } | Select-Object -First 1
            if ($null -eq $owner) { throw 'Agent owner exited during inspection.' }
        } catch { $ownerError = $_.Exception.Message }
        finally { if ($null -ne $process) { $process.Dispose() } }
    }
    $external = @($agents | Where-Object { $appIds -notcontains [uint32]$_.ParentProcessId })
    # An external owner can be a CLI or a headless/Desktop in another install.
    if ($null -ne $owner -and $appIds -notcontains [uint32]$owner.ProcessId -and
        @($external | Where-Object { $_.ProcessId -eq $owner.ProcessId }).Count -eq 0) {
        $external += $owner
    }
    $state = Read-CapabilityState
    $recordCount = if ($null -eq $state) { 0 } else { @($state.records).Count }
    $treeIds = [Collections.Generic.HashSet[uint32]]::new()
    foreach ($entry in @($apps) + @($agents) + @($external)) { $null = $treeIds.Add([uint32]$entry.ProcessId) }
    do {
        $changed = $false
        foreach ($entry in $all) {
            if ($treeIds.Contains([uint32]$entry.ParentProcessId) -and $treeIds.Add([uint32]$entry.ProcessId)) { $changed = $true }
        }
    } while ($changed)
    return [pscustomobject]@{
        Apps = $apps
        Agents = $agents
        BundledAgents = $bundled
        ExternalAgents = $external
        Owner = $owner
        OwnerError = $ownerError
        LockHeld = $held
        MetadataExists = (Test-Path -LiteralPath $instancePath)
        ProcessTree = @($all | Where-Object { $treeIds.Contains([uint32]$_.ProcessId) })
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
    Write-LogLine "Agent lock held: $($State.LockHeld)"
    if ($null -ne $State.Owner) { Write-LogLine "Verified owner: $($State.Owner.Name) PID=$($State.Owner.ProcessId)" }
    if ($State.OwnerError) { Write-LogLine "Owner diagnosis: $($State.OwnerError)" }
    foreach ($app in @($State.ProcessTree)) {
        $process = $null
        try {
            $process = [Diagnostics.Process]::GetProcessById([int]$app.ProcessId)
            Write-LogLine "Process=$($app.Name) PID=$($app.ProcessId) ParentPID=$($app.ParentProcessId) threads=$($process.Threads.Count) workingSetBytes=$($process.WorkingSet64) privateBytes=$($process.PrivateMemorySize64)"
        } catch { Write-LogLine "Metrics unavailable for PID=$($app.ProcessId): $($_.Exception.Message)" }
        finally { if ($null -ne $process) { $process.Dispose() } }
    }
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
        { $_ -in @('ExpectEmbedded', 'ExpectBundled', 'ExpectRecovered') } {
            if ($appCount -ne 1) { $errors.Add("expected one FutureOS process, found $appCount") }
            if ($agentCount -ne 0) { $errors.Add("embedded mode must have no standalone Agent process, found $agentCount") }
            if ($externalCount -ne 0) { $errors.Add("expected no external Agent, found $externalCount") }
            if (-not $State.LockHeld) { $errors.Add('expected the Agent instance lock to be held') }
            if ($null -eq $State.Owner -or @($State.Apps | Where-Object { $_.ProcessId -eq $State.Owner.ProcessId }).Count -ne 1) {
                $errors.Add('expected verified Agent metadata to name the Desktop/headless host itself')
            }
            if ($Expected -eq 'ExpectRecovered' -and $State.CapabilityRecords -ne 0) { $errors.Add('expected startup recovery to remove all capability records') }
        }
        "ExpectStopped" {
            if ($appCount -ne 0) { $errors.Add("expected FutureOS to be stopped, found $appCount process(es)") }
            if ($agentCount -ne 0) { $errors.Add("expected bundled Agent to be stopped, found $agentCount Agent process(es)") }
            if ($State.CapabilityRecords -ne 0) { $errors.Add("expected shutdown cleanup to remove all records, found $($State.CapabilityRecords)") }
        }
        "ExpectExternalAttached" {
            if ($appCount -ne 1) { $errors.Add("expected one FutureOS process, found $appCount") }
            if ($bundledCount -ne 0) { $errors.Add("FutureOS unexpectedly owns an Agent process") }
            if ($externalCount -ne 1) { $errors.Add("expected one external Agent, found $externalCount") }
        }
        "ExpectExternalSurvives" {
            if ($appCount -ne 0) { $errors.Add("expected FutureOS to be stopped, found $appCount process(es)") }
            if ($bundledCount -ne 0) { $errors.Add("a stopped FutureOS cannot own the surviving Agent") }
            if ($externalCount -ne 1) { $errors.Add("expected one external Agent, found $externalCount") }
            if ($State.CapabilityRecords -ne 1) { $errors.Add("expected Desktop to leave the external Agent's one fixture record untouched, found $($State.CapabilityRecords)") }
        }
    }
    if ($Expected -in @('ExpectClean', 'ExpectStopped')) {
        if ($State.LockHeld) { $errors.Add('Agent instance lock is still held') }
        if ($State.MetadataExists) { $errors.Add('Agent instance metadata was not removed') }
    }
    if ($Expected -in @('ExpectExternalAttached', 'ExpectExternalSurvives')) {
        if (-not $State.LockHeld -or $null -eq $State.Owner -or
            @($State.ExternalAgents | Where-Object { $_.ProcessId -eq $State.Owner.ProcessId }).Count -ne 1) {
            $errors.Add('expected a verified external owner holding the Agent instance lock')
        }
    }
    return $errors
}

function Assert-AgentReady {
    # A successful read-only CLI request includes the shared compatibility and
    # readiness handshake. Never let this probe start an Agent or open its DB.
    if ($env:FUTURE_AGENT_GRPC_ADDR -or $env:FUTURE_AGENT_SOCKET) {
        throw 'Unset endpoint overrides for lifecycle validation; use the data-root-scoped default IPC.'
    }
    $cli = Join-Path $InstallDirectory 'future.exe'
    if (-not (Test-Path -LiteralPath $cli)) { throw "Missing packaged CLI: $cli" }
    $stdout = Join-Path $outputRoot "probe-$PID.stdout"
    $stderr = Join-Path $outputRoot "probe-$PID.stderr"
    $probe = $null
    try {
        $probe = Start-Process -FilePath $cli -ArgumentList 'session list --json' -PassThru -WindowStyle Hidden -RedirectStandardOutput $stdout -RedirectStandardError $stderr
        if (-not $probe.WaitForExit(15000)) {
            $probe.Kill()
            $null = $probe.WaitForExit(5000)
            throw 'Packaged CLI readiness probe timed out.'
        }
        if ($probe.ExitCode -ne 0) { throw "Packaged CLI readiness probe failed: $(Get-Content -LiteralPath $stderr -Raw)" }
        $reply = Get-Content -LiteralPath $stdout -Raw
        if ([string]::IsNullOrWhiteSpace($reply) -or -not $reply.TrimStart().StartsWith('[')) {
            throw 'Packaged CLI did not return a sessions JSON array.'
        }
        $null = $reply | ConvertFrom-Json
        Write-LogLine 'Packaged CLI compatibility/readiness and list_sessions RPC: PASS'
    } finally {
        if ($null -ne $probe) { $probe.Dispose() }
        Remove-Item -LiteralPath $stdout, $stderr -Force -ErrorAction SilentlyContinue
    }
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
    Write-LogLine "FutureOS root: $futureRoot"
    Write-LogLine "Installation: $InstallDirectory"
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
        if ($errors.Count -eq 0 -and $Action -in @('ExpectEmbedded', 'ExpectBundled', 'ExpectRecovered', 'ExpectExternalAttached', 'ExpectExternalSurvives')) {
            try {
                # Metadata is published before ready. Retry a still-starting
                # service instead of turning that normal interval into failure.
                Assert-AgentReady
                $afterProbe = Get-LifecycleState
                $errors = @(Get-ExpectationErrors $Action $afterProbe)
                if ($errors.Count -eq 0 -and ($afterProbe.Owner.ProcessId -ne $state.Owner.ProcessId -or
                    $afterProbe.Owner.CreationDate -ne $state.Owner.CreationDate)) {
                    $errors = @('Agent ownership changed during the readiness probe.')
                }
                $state = $afterProbe
            } catch { $errors = @($_.Exception.Message) }
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
