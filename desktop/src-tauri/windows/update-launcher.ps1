# Runs independently after full Desktop cleanup. Never starts installation
# until the exact parent process handle has exited and released its EXE.
param([int]$ParentPid, [string]$Desktop, [string]$Directory, [string]$Log)
$ErrorActionPreference = 'Stop'
$parent = $null
$installer = $null
$parentExited = $false
$snapshotComplete = $false
$original = @{}
$resumeArguments = ''
function Write-Diagnostic {
    param([string]$Message)
    try { [IO.File]::AppendAllText($Log, "$(Get-Date -Format o) $Message`r`n") }
    catch { Write-Host $Message }
}
try {
    $parent = [Diagnostics.Process]::GetProcessById($ParentPid)
    $null = $parent.Handle
    [IO.File]::WriteAllText((Join-Path $Directory 'ready'), 'ready')
    if (-not $parent.WaitForExit(60000)) { throw 'Desktop did not exit; installation was not started.' }
    $parentExited = $true
    $argsFile = Join-Path $Directory 'resume-args.txt'
    if ([IO.File]::Exists($argsFile)) { $resumeArguments = [IO.File]::ReadAllText($argsFile) }
    foreach ($name in @('futureos.exe', 'futureos-headless.exe', 'future.exe', 'future-desktop.exe', 'future-agent.exe')) {
        $path = Join-Path ([IO.Path]::GetDirectoryName($Desktop)) $name
        $original[$path] = $(if ([IO.File]::Exists($path)) { (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash } else { $null })
    }
    $snapshotComplete = $true
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = Join-Path $Directory 'installer.exe'
    $info.Arguments = '/P /R /UPDATE /FUTUREOS_CLEAN /ARGS ' + $resumeArguments
    $info.UseShellExecute = $false
    $installer = [Diagnostics.Process]::Start($info)
    if ($null -eq $installer) { throw 'Windows did not start the installer.' }
    # No timeout kill: interrupting extraction can prevent NSIS rollback.
    $installer.WaitForExit()
    Write-Diagnostic "Installer exit=$($installer.ExitCode)"
    if ($installer.ExitCode -ne 0) { throw "Installer failed with exit $($installer.ExitCode); NSIS rollback was requested." }
} catch {
    Write-Diagnostic $_.Exception.Message
    $restored = $snapshotComplete
    foreach ($path in $original.Keys) {
        try {
            $actual = $(if ([IO.File]::Exists($path)) { (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash } else { $null })
            if ($actual -ne $original[$path]) { $restored = $false }
        } catch { $restored = $false }
    }
    if (-not $restored) { Write-Diagnostic 'Original executable hashes were not restored; automatic recovery is disabled.' }
    if ($restored -and $parentExited -and [IO.File]::Exists($Desktop) -and
        -not [IO.File]::Exists((Join-Path ([IO.Path]::GetDirectoryName($Desktop)) 'futureos-recovery-failed'))) {
        try {
            $info = [Diagnostics.ProcessStartInfo]::new()
            $info.FileName = $Desktop
            $info.Arguments = $resumeArguments
            $info.UseShellExecute = $false
            $null = [Diagnostics.Process]::Start($info)
        } catch {
            Write-Diagnostic "Recovery launch failed: $($_.Exception.Message)"
        }
    }
    exit 5
} finally {
    if ($null -ne $installer) { $installer.Dispose() }
    if ($null -ne $parent) { $parent.Dispose() }
    Remove-Item -LiteralPath $Directory -Recurse -Force -ErrorAction SilentlyContinue
}
