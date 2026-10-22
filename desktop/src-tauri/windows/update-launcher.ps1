# Runs independently after full Desktop cleanup. Never starts installation
# until the exact parent process handle has exited and released its EXE.
param([int]$ParentPid, [string]$Desktop, [string]$Directory, [string]$Log, [Parameter(Mandatory = $true)][string]$Record, [ValidateRange(1, 60000)][int]$ParentExitTimeoutMs = 60000)
$ErrorActionPreference = 'Stop'
$parent = $null
$installer = $null
$parentExited = $false
$snapshotComplete = $false
$original = @{}
$resumeArguments = ''
$result = $null
$installationStarted = $false
$observerLease = $null
function Write-Diagnostic {
    param([string]$Message)
    try { [IO.File]::AppendAllText($Log, "$(Get-Date -Format o) $Message`r`n") }
    catch { Write-Host $Message }
}
function Write-Result {
    param([string]$Stage, $Outcome, $Diagnostic, [bool]$RestartSafe = $false)
    if ($null -eq $result) { throw 'Update record is unavailable; installation is forbidden.' }
    $result.stage = $Stage
    $result.outcome = $Outcome
    $result.diagnostic = $Diagnostic
    $result.restartSafe = $RestartSafe
    $temporary = $Record + '.' + [Guid]::NewGuid().ToString('N') + '.tmp'
    $backup = $temporary + '.previous'
    $replaced = $false
    try {
        $bytes = [Text.UTF8Encoding]::new($false).GetBytes(($result | ConvertTo-Json -Depth 8))
        $stream = [IO.FileStream]::new($temporary, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
        try { $stream.Write($bytes, 0, $bytes.Length); $stream.Flush($true) } finally { $stream.Dispose() }
        # Windows PowerShell 5.1 binds a null string argument as an empty
        # pathname. Use an actual same-directory backup, preserving atomic
        # replacement; keep that backup for diagnostics if replacement fails.
        [IO.File]::Replace($temporary, $Record, $backup)
        $replaced = $true
    } finally {
        Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue
        if ($replaced) { Remove-Item -LiteralPath $backup -Force -ErrorAction SilentlyContinue }
    }
}
function Get-ProgramSnapshot {
    $snapshot = @{}
    $programRoot = [IO.Path]::GetDirectoryName($Desktop)
    # Reject reparse points rather than following links outside the install.
    $entries = @(Get-ChildItem -LiteralPath $programRoot -Force -Recurse)
    foreach ($entry in $entries) {
        if (($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'Installation contains a reparse point; automatic recovery cannot be verified.' }
        $snapshot[$entry.FullName] = $(if ($entry.PSIsContainer) { 'directory' } else { (Get-FileHash -LiteralPath $entry.FullName -Algorithm SHA256).Hash })
    }
    return ,$snapshot
}
try {
    $result = [IO.File]::ReadAllText($Record) | ConvertFrom-Json
    if ($result.formatVersion -ne 1 -or $null -ne $result.outcome -or $result.stage -ne 'preparing') { throw 'Invalid update handoff record.' }
    $observerLease = [IO.FileStream]::new((Join-Path ([IO.Path]::GetDirectoryName($Record)) 'observer.lock'), [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    $parent = [Diagnostics.Process]::GetProcessById($ParentPid)
    $null = $parent.Handle
    # Complete all fallible discovery and backup work while Desktop is alive.
    $original = Get-ProgramSnapshot
    $snapshotComplete = $true
    $backup = Join-Path ([IO.Path]::GetDirectoryName($Record)) 'program-backup'
    $null = New-Item -ItemType Directory -Path $backup -Force
    $programRoot = [IO.Path]::GetDirectoryName($Desktop)
    Get-ChildItem -LiteralPath $programRoot -Force | Copy-Item -Destination $backup -Recurse -Force
    [IO.File]::WriteAllText((Join-Path $Directory 'ready'), 'ready')
    $deadline = [DateTime]::UtcNow.AddMinutes(15)
    while (-not [IO.File]::Exists((Join-Path $Directory 'commit'))) {
        if ($parent.HasExited -or [DateTime]::UtcNow -gt $deadline) { throw 'Update preparation cancelled; installation was not started.' }
        Start-Sleep -Milliseconds 50
    }
    Write-Result 'awaiting-exit' $null $null
    if (-not $parent.WaitForExit($ParentExitTimeoutMs)) { throw 'Desktop did not exit; installation was not started.' }
    $parentExited = $true
    $argsFile = Join-Path $Directory 'resume-args.txt'
    if ([IO.File]::Exists($argsFile)) { $resumeArguments = [IO.File]::ReadAllText($argsFile) }
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = Join-Path $Directory 'installer.exe'
    $info.Arguments = '/P /R /UPDATE /FUTUREOS_CLEAN /ARGS ' + $resumeArguments
    $info.UseShellExecute = $false
    Write-Result 'installing' $null $null
    $installationStarted = $true
    $installer = [Diagnostics.Process]::Start($info)
    if ($null -eq $installer) { throw 'Windows did not start the installer.' }
    # No timeout kill: interrupting extraction can prevent NSIS rollback.
    $installer.WaitForExit()
    Write-Diagnostic "Installer exit=$($installer.ExitCode)"
    if ($installer.ExitCode -ne 0) { throw "Installer failed with exit $($installer.ExitCode); NSIS rollback was requested." }
    Write-Result 'awaiting-validation' 'pending-validation' $null $true
} catch {
    $failure = $_.Exception.Message
    Write-Diagnostic $failure
    $restored = $snapshotComplete
    if ($installationStarted -and $parentExited -and $snapshotComplete) {
        try {
            $programRoot = [IO.Path]::GetDirectoryName($Desktop)
            # Preserve any new/unrecognized files; a mismatch requires repair
            # rather than deleting data that was not present in our backup.
            Get-ChildItem -LiteralPath $backup -Force | Copy-Item -Destination $programRoot -Recurse -Force
        } catch { Write-Diagnostic "Backup restoration failed: $($_.Exception.Message)"; $restored = $false }
    }
    if ($snapshotComplete) {
        try {
            $actual = Get-ProgramSnapshot
            if ($actual.Count -ne $original.Count) { $restored = $false }
            foreach ($path in $original.Keys) {
                if (-not $actual.ContainsKey($path) -or $actual[$path] -ne $original[$path]) { $restored = $false }
            }
        } catch { $restored = $false }
    }
    $outcome = $(if (-not $installationStarted) { 'not-installed' } elseif ($restored) { 'failed-restored' } else { 'repair-required' })
    try { Write-Result 'finished' $outcome $failure $restored } catch { Write-Diagnostic "Cannot persist update failure: $($_.Exception.Message)"; $restored = $false }
    if (-not $restored) { Write-Diagnostic 'Original executable hashes were not restored; automatic recovery is disabled.' }
    if ($restored -and $parentExited -and [IO.File]::Exists($Desktop) -and
        -not [IO.File]::Exists((Join-Path ([IO.Path]::GetDirectoryName($Desktop)) 'futureos-recovery-failed'))) {
        try {
            $info = [Diagnostics.ProcessStartInfo]::new()
            $info.FileName = $Desktop
            $info.Arguments = $resumeArguments
            $info.UseShellExecute = $false
            $restarted = [Diagnostics.Process]::Start($info)
            if ($null -eq $restarted) { throw 'Windows did not start Desktop recovery.' }
            $restarted.Dispose()
            Write-Diagnostic 'Original program verified; Desktop recovery launch requested.'
        } catch {
            $recoveryFailure = "Recovery launch failed: $($_.Exception.Message)"
            Write-Diagnostic $recoveryFailure
            try { Write-Result 'finished' 'failed-restored' ($failure + '; ' + $recoveryFailure) $true } catch { Write-Diagnostic $_.Exception.Message }
        }
    }
    exit 5
} finally {
    if ($null -ne $installer) { $installer.Dispose() }
    if ($null -ne $parent) { $parent.Dispose() }
    if ($null -ne $observerLease) { $observerLease.Dispose() }
    if (-not $installationStarted -and $backup) { Remove-Item -LiteralPath $backup -Recurse -Force -ErrorAction SilentlyContinue }
    Remove-Item -LiteralPath $Directory -Recurse -Force -ErrorAction SilentlyContinue
}
