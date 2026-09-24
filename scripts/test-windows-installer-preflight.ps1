# Windows-only offline regression. Uses throwaway EXEs, never the real agent.
# Run with Windows PowerShell 5.1 or pwsh; NSIS is required (Tauri cache accepted).
param([string]$MakeNsis)
$ErrorActionPreference = 'Stop'
if (-not $MakeNsis) {
    $candidates = @(
        (Join-Path $env:LOCALAPPDATA 'tauri\NSIS\makensis.exe'),
        (Join-Path ${env:ProgramFiles(x86)} 'NSIS\makensis.exe')
    )
    $MakeNsis = $candidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
    if (-not $MakeNsis) { $MakeNsis = (Get-Command makensis.exe).Source }
}
$root = Join-Path ([IO.Path]::GetTempPath()) ('futureos-preflight-test-' + [Guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $root
# Isolate the user-scoped Agent lock from a developer's real FutureOS data.
$previousHome = $env:HOME
$previousFutureHome = $env:FUTURE_HOME
$env:HOME = $root
$env:FUTURE_HOME = $null
$processes = [Collections.Generic.List[Diagnostics.Process]]::new()
$preflight = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\desktop\src-tauri\windows\installer-preflight.ps1'))
$powershell = Join-Path $env:WINDIR 'System32\WindowsPowerShell\v1.0\powershell.exe'

function Assert-Equal($Actual, $Expected, [string]$Label) {
    if ($Actual -ne $Expected) { throw "${Label}: expected $Expected, got $Actual" }
}
function New-Install([string]$Name) {
    $directory = Join-Path $root $Name
    $null = New-Item -ItemType Directory -Path $directory
    Copy-Item -LiteralPath $fixture -Destination (Join-Path $directory 'future.exe')
    Copy-Item -LiteralPath $fixture -Destination (Join-Path $directory 'future-agent.exe')
    Copy-Item -LiteralPath $fixture -Destination (Join-Path $directory 'futureos.exe')
    Copy-Item -LiteralPath $fixture -Destination (Join-Path $directory 'future-desktop.exe')
    return $directory
}
function Start-Fixture([string]$Directory, [string]$Name = 'future.exe') {
    $ready = Join-Path $Directory ($Name + '.ready')
    $p = Start-Process -FilePath (Join-Path $Directory $Name) -ArgumentList "--hold `"$ready`"" -PassThru
    $processes.Add($p)
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    while (-not (Test-Path -LiteralPath $ready)) {
        if ($p.HasExited -or [DateTime]::UtcNow -gt $deadline) { throw 'Fixture did not start' }
        Start-Sleep -Milliseconds 50
    }
    return $p
}
function Wait-Exit([Diagnostics.Process]$Process) {
    if (-not $Process.WaitForExit(60000)) { throw "Test process timed out: $($Process.Id) $($Process.ProcessName) ($($Process.MainWindowTitle))" }
    $Process.Refresh()
    return $Process.ExitCode
}
function Invoke-Preflight([string]$Directory, [string]$Mode = 'Check') {
    $p = Start-Process -FilePath $powershell -ArgumentList "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File `"$preflight`" -InstallDir `"$Directory`" -Mode $Mode" -WindowStyle Hidden -PassThru
    $processes.Add($p)
    return (Wait-Exit $p)
}
function Start-Installer([string]$Directory, [string]$Flags = '/S') {
    # NSIS /D must be last and unquoted, even when it contains spaces.
    $p = Start-Process -FilePath $installer -ArgumentList "$Flags /D=$Directory" -PassThru
    $processes.Add($p)
    return $p
}
function Assert-Unchanged([string]$Directory) {
    Assert-Equal (Get-FileHash -LiteralPath (Join-Path $Directory 'future.exe')).Hash $fixtureHash 'Old binary preserved'
    Assert-Equal (Get-FileHash -LiteralPath (Join-Path $Directory 'future-agent.exe')).Hash $fixtureHash 'Legacy Agent preserved'
    Assert-Equal (Test-Path -LiteralPath (Join-Path $Directory 'installed.marker')) $false 'No partial install'
}

# Drive only dialogs owned by the throwaway installer PID. No desktop-wide clicks.
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class PreflightDialogs {
    public static IntPtr LastWindow;
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr hwnd);
    private delegate bool EnumProc(IntPtr hwnd, IntPtr param);
    [DllImport("user32.dll")] private static extern bool EnumWindows(EnumProc callback, IntPtr param);
    [DllImport("user32.dll")] private static extern bool EnumChildWindows(IntPtr hwnd, EnumProc callback, IntPtr param);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern int GetClassName(IntPtr hwnd, StringBuilder text, int size);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern int GetWindowText(IntPtr hwnd, StringBuilder text, int size);
    [DllImport("user32.dll")] private static extern IntPtr GetDlgItem(IntPtr hwnd, int id);
    [DllImport("user32.dll")] private static extern IntPtr SendMessage(IntPtr hwnd, uint msg, IntPtr wp, IntPtr lp);
    public static string Click(int pid, int button) {
        string result = null;
        EnumWindows((hwnd, unused) => {
            uint owner; GetWindowThreadProcessId(hwnd, out owner);
            if (owner != pid) return true;
            var cls = new StringBuilder(128); GetClassName(hwnd, cls, cls.Capacity);
            // Standard MessageBox, not the main NSIS dialog (same class).
            if (cls.ToString() != "#32770" || GetDlgItem(hwnd, button) == IntPtr.Zero || GetDlgItem(hwnd, 65535) == IntPtr.Zero) return true;
            var text = new StringBuilder();
            EnumChildWindows(hwnd, (child, ignored) => {
                var part = new StringBuilder(4096); GetWindowText(child, part, part.Capacity);
                text.AppendLine(part.ToString()); return true;
            }, IntPtr.Zero);
            result = text.ToString();
            LastWindow = hwnd;
            SendMessage(hwnd, 0x0111, (IntPtr)button, GetDlgItem(hwnd, button));
            return false;
        }, IntPtr.Zero);
        return result;
    }
}
'@
function Click-Dialog([Diagnostics.Process]$Process, [int]$Button, [string]$ExpectedText) {
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    while ([DateTime]::UtcNow -lt $deadline) {
        $text = [PreflightDialogs]::Click($Process.Id, $Button)
        if ($null -ne $text) {
            if (-not $text.Contains($ExpectedText)) { throw "Unexpected dialog: $text" }
            while ([PreflightDialogs]::IsWindow([PreflightDialogs]::LastWindow)) {
                if ([DateTime]::UtcNow -gt $deadline) { throw 'Clicked dialog did not close' }
                Start-Sleep -Milliseconds 50
            }
            return
        }
        if ($Process.HasExited) { throw 'Installer exited before showing guidance' }
        Start-Sleep -Milliseconds 100
    }
    throw 'Guidance dialog did not appear'
}

try {
    $fixture = Join-Path $root 'fixture.exe'
    $csc = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
    & $csc /nologo /target:winexe "/out:$fixture" (Join-Path $PSScriptRoot 'tests\windows-installer-fixture.cs')
    if ($LASTEXITCODE -ne 0) { throw 'Fixture compilation failed' }
    $fixtureHash = (Get-FileHash -LiteralPath $fixture).Hash
    $newFixture = Join-Path $root 'fixture-new.exe'
    & $csc /nologo /target:winexe /define:INSTALLER_NEW "/out:$newFixture" (Join-Path $PSScriptRoot 'tests\windows-installer-fixture.cs')
    if ($LASTEXITCODE -ne 0) { throw 'New fixture compilation failed' }
    $newFixtureHash = (Get-FileHash -LiteralPath $newFixture).Hash
    if ($newFixtureHash -eq $fixtureHash) { throw 'Old and new fixtures must differ' }
    $installer = Join-Path $root 'preflight-setup.exe'
    & $MakeNsis /V2 /WX /INPUTCHARSET UTF8 "/DTEST_OUTFILE=$installer" "/DTEST_FIXTURE=$newFixture" (Join-Path $PSScriptRoot 'tests\windows-installer-preflight.nsi')
    if ($LASTEXITCODE -ne 0) { throw 'NSIS compilation failed' }

    Write-Host 'Testing fresh install...'
    $fresh = Join-Path $root 'fresh install'
    Assert-Equal (Wait-Exit (Start-Installer $fresh)) 0 'Fresh install'
    Assert-Equal (Test-Path (Join-Path $fresh 'installed.marker')) $true 'Install completed'
    Assert-Equal (Get-FileHash -LiteralPath (Join-Path $fresh 'future.exe')).Hash $newFixtureHash 'Fresh install writes new Agent'
    Assert-Equal (Get-FileHash -LiteralPath (Join-Path $fresh 'futureos.exe')).Hash $newFixtureHash 'Fresh install writes new desktop'

    Write-Host 'Testing the user-scoped Agent lock independently of EXE paths...'
    $state = Join-Path $root '.future\agent'
    $null = New-Item -ItemType Directory -Force -Path $state
    $lockFile = [IO.FileStream]::new((Join-Path $state 'agent-instance.lock'),
        [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::ReadWrite)
    try {
        $lockFile.Lock(0, 1)
        Assert-Equal (Invoke-Preflight $fresh) 32 'Occupied Agent lock blocks install without metadata'
    } finally {
        $lockFile.Unlock(0, 1)
        $lockFile.Dispose()
    }
    Assert-Equal (Invoke-Preflight $fresh) 0 'Released Agent lock permits install'

    Write-Host 'Testing process detection, silent and passive modes...'
    # Space, apostrophe and Unicode paths; no command-string interpolation.
    $target = New-Install ("Program Files user's " + [char]0x672A + [char]0x6765)
    $other = New-Install 'other installation'
    $otherAgent = Start-Fixture $other
    Assert-Equal (Invoke-Preflight $target) 0 'Other installation does not block'
    Assert-Unchanged $target
    $agent = Start-Fixture $target
    $legacyAgent = Start-Fixture $target 'future-agent.exe'
    $desktop = Start-Fixture $target 'futureos.exe'
    $legacyDesktop = Start-Fixture $target 'future-desktop.exe'
    Assert-Equal (Invoke-Preflight $target) 32 'Detect running installation'
    Assert-Equal (Wait-Exit (Start-Installer $target)) 32 'Silent install fails closed'
    Assert-Equal (Wait-Exit (Start-Installer $target '/P')) 32 'Passive install fails closed'
    Assert-Equal $agent.HasExited $false 'Silent mode preserves tasks'
    Assert-Equal $legacyAgent.HasExited $false 'Silent mode preserves legacy Agent tasks'
    Assert-Equal $desktop.HasExited $false 'Silent mode preserves desktop'
    Assert-Unchanged $target

    Write-Host 'Testing automatic update recovery from an old running Agent...'
    $automatic = New-Install 'automatic mixed update'
    $automaticAgent = Start-Fixture $automatic
    $automaticLegacyAgent = Start-Fixture $automatic 'future-agent.exe'
    $automaticDesktop = Start-Fixture $automatic 'futureos.exe'
    Assert-Equal (Wait-Exit (Start-Installer $automatic '/S /UPDATE')) 0 'Automatic update repairs running mixed install'
    Assert-Equal $automaticAgent.HasExited $true 'Automatic update closes current Agent'
    Assert-Equal $automaticLegacyAgent.HasExited $true 'Automatic update closes legacy Agent'
    Assert-Equal $automaticDesktop.HasExited $true 'Automatic update closes old desktop'
    Assert-Equal (Test-Path (Join-Path $automatic 'installed.marker')) $true 'Automatic update completed'
    Assert-Equal (Get-FileHash -LiteralPath (Join-Path $automatic 'future.exe')).Hash $newFixtureHash 'Automatic update writes new Agent'
    Assert-Equal (Get-FileHash -LiteralPath (Join-Path $automatic 'futureos.exe')).Hash $newFixtureHash 'Automatic update writes new desktop'
    Assert-Equal (Test-Path (Join-Path $automatic 'future-agent.exe')) $false 'Automatic update removes legacy Agent'

    Write-Host 'Testing interactive cancellation...'
    # Interactive Cancel never stops processes or replaces files.
    $ui = Start-Installer $target ''
    Click-Dialog $ui 2 'Save your work'
    Assert-Equal (Wait-Exit $ui) 32 'Cancel exits setup'
    Assert-Equal $agent.HasExited $false 'Cancel preserves tasks'
    Assert-Unchanged $target

    Write-Host 'Testing Chinese guidance and explicit close consent...'
    # Chinese dialog + No recheck + Yes consent closes only this installation.
    $ui = Start-Installer $target '/ZH'
    Click-Dialog $ui 7 ([string][char]0x672A)
    Click-Dialog $ui 6 'future.exe'
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    while (-not (Test-Path (Join-Path $target 'installed.marker'))) {
        if ($ui.HasExited -or [DateTime]::UtcNow -gt $deadline) {
            $dialog = [PreflightDialogs]::Click($ui.Id, 2)
            throw "Consented install did not finish (agent exited=$($agent.HasExited), desktop exited=$($desktop.HasExited)): $dialog"
        }
        Start-Sleep -Milliseconds 100
    }
    Assert-Equal (Wait-Exit $ui) 0 'Consent continues installation'
    Assert-Equal $agent.HasExited $true 'Target agent closed'
    Assert-Equal $legacyAgent.HasExited $true 'Target legacy agent closed'
    Assert-Equal $desktop.HasExited $true 'Target desktop closed'
    Assert-Equal $legacyDesktop.HasExited $true 'Legacy desktop closed'
    Assert-Equal $otherAgent.HasExited $false 'Other installation untouched'
    Assert-Equal (Test-Path (Join-Path $target 'future-agent.exe')) $false 'Legacy Agent removed before install'

    Write-Host 'Testing an Agent in another installation holding the shared lock...'
    $ownerReady = Join-Path $other 'lock-owner.ready'
    $state = Join-Path $root '.future\agent'
    $lockedAgent = Start-Process -FilePath (Join-Path $other 'future.exe') -ArgumentList "--hold-agent-lock `"$ownerReady`" `"$state`"" -PassThru
    $processes.Add($lockedAgent)
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    while (-not (Test-Path -LiteralPath $ownerReady)) {
        if ($lockedAgent.HasExited -or [DateTime]::UtcNow -gt $deadline) { throw 'Lock owner did not start' }
        Start-Sleep -Milliseconds 50
    }
    $repair = New-Install 'cross-directory Agent repair'
    Assert-Equal (Invoke-Preflight $repair) 32 'Shared Agent lock blocks a different directory'
    Assert-Equal (Wait-Exit (Start-Installer $repair '/S /UPDATE')) 0 'Verified Agent owner closed by update'
    Assert-Equal $lockedAgent.HasExited $true 'Verified external Agent exited'
    Assert-Equal $otherAgent.HasExited $false 'Unrelated process in other installation survives'

    Write-Host 'Testing external lock and retry...'
    $locked = New-Install 'external lock'
    $handle = [IO.File]::Open((Join-Path $locked 'future.exe'), 'Open', 'Read', 'Read')
    try {
        Assert-Equal (Invoke-Preflight $locked) 32 'External lock detected'
        Assert-Equal (Invoke-Preflight $locked 'Close') 32 'Cannot close unrelated locker'
        Assert-Equal (Wait-Exit (Start-Installer $locked)) 32 'Locked install blocked'
        Assert-Unchanged $locked
        Assert-Equal (Wait-Exit (Start-Installer $locked '/S /UPDATE')) 32 'Automatic update fails closed on unrelated locker'
        Assert-Unchanged $locked
    } finally { $handle.Dispose() }
    Assert-Equal (Wait-Exit (Start-Installer $locked)) 0 'Retry succeeds after unlock'

    Write-Host 'Testing failed upgrade rollback...'
    $rollback = New-Install 'rollback after copy'
    Assert-Equal (Wait-Exit (Start-Installer $rollback '/S /FAILAFTERCOPY')) 5 'Failed upgrade reports failure'
    Assert-Equal (Get-FileHash -LiteralPath (Join-Path $rollback 'future.exe')).Hash $fixtureHash 'Failed upgrade restores old Agent'
    Assert-Equal (Get-FileHash -LiteralPath (Join-Path $rollback 'futureos.exe')).Hash $fixtureHash 'Failed upgrade restores old desktop'
    Assert-Equal (Get-FileHash -LiteralPath (Join-Path $rollback 'future-agent.exe')).Hash $fixtureHash 'Failed upgrade restores legacy Agent'
    Assert-Equal (Get-FileHash -LiteralPath (Join-Path $rollback 'future-desktop.exe')).Hash $fixtureHash 'Failed upgrade restores legacy desktop'
    Assert-Equal (Test-Path (Join-Path $rollback 'installed.marker')) $false 'Failed upgrade does not report completion'
    Assert-Equal (Invoke-Preflight $rollback) 0 'Failed upgrade releases Agent lock lease'

    Write-Host 'Testing failed post-install health check rollback...'
    $unloadable = New-Install 'rollback after health check'
    Assert-Equal (Wait-Exit (Start-Installer $unloadable '/S /FAILHEALTH')) 5 'Unloadable desktop reports failure'
    Assert-Equal (Get-FileHash -LiteralPath (Join-Path $unloadable 'future.exe')).Hash $fixtureHash 'Health failure restores old Agent'
    Assert-Equal (Get-FileHash -LiteralPath (Join-Path $unloadable 'futureos.exe')).Hash $fixtureHash 'Health failure restores old desktop'
    Assert-Equal (Test-Path (Join-Path $unloadable 'installed.marker')) $false 'Health failure does not report completion'

    Write-Host 'Testing read-only executable...'
    $readOnly = New-Install 'read only'
    $readOnlyExe = Join-Path $readOnly 'future.exe'
    [IO.File]::SetAttributes($readOnlyExe, [IO.FileAttributes]::ReadOnly)
    try {
        Assert-Equal (Invoke-Preflight $readOnly) 5 'Read-only is not a running-process error'
        Assert-Equal (Wait-Exit (Start-Installer $readOnly)) 5 'Write failure blocks install'
        Assert-Unchanged $readOnly
    } finally { [IO.File]::SetAttributes($readOnlyExe, [IO.FileAttributes]::Normal) }

    Write-Host 'Testing uninstall preflight...'
    # Uninstall owns this installation: it closes exact-path processes and
    # continues even when sandbox cleanup is unsupported or fails.
    $uninstallDir = New-Install 'uninstall'
    Copy-Item (Join-Path $fresh 'uninstall.exe') $uninstallDir
    $uninstallAgent = Start-Fixture $uninstallDir
    $uninstaller = Start-Process -FilePath (Join-Path $uninstallDir 'uninstall.exe') -ArgumentList "/S _?=$uninstallDir" -PassThru
    $processes.Add($uninstaller)
    Assert-Equal (Wait-Exit $uninstaller) 0 'Silent uninstall closes the owned Agent and completes'
    Assert-Equal $uninstallAgent.HasExited $true 'Uninstall closes the owned Agent'
    Assert-Equal (Test-Path (Join-Path $uninstallDir 'uninstalled.marker')) $true 'Uninstall proceeded'
    Assert-Equal (Test-Path (Join-Path $uninstallDir 'futureos.exe')) $false 'Uninstall removes desktop before completion'
    Assert-Equal (Test-Path (Join-Path $uninstallDir 'future.exe')) $false 'Uninstall removes Agent before completion'
    Assert-Equal (Test-Path (Join-Path $uninstallDir 'future-desktop.exe')) $false 'Uninstall removes legacy desktop before completion'
    Assert-Equal (Test-Path (Join-Path $uninstallDir 'future-agent.exe')) $false 'Uninstall removes legacy Agent before completion'

    # Both unattended modes mirror Tauri's production auto-close behavior.
    foreach ($flags in @('/S', '/P')) {
        $failedCleanup = New-Install ('cleanup failure ' + $flags.TrimStart('/'))
        Copy-Item (Join-Path $fresh 'uninstall.exe') $failedCleanup
        $null = New-Item -ItemType File -Path (Join-Path $failedCleanup 'fail-cleanup')
        $uninstaller = Start-Process -FilePath (Join-Path $failedCleanup 'uninstall.exe') -ArgumentList "$flags _?=$failedCleanup" -PassThru
        $processes.Add($uninstaller)
        Assert-Equal (Wait-Exit $uninstaller) 0 'Cleanup failure does not block unattended uninstall'
        Assert-Equal (Test-Path (Join-Path $failedCleanup 'uninstalled.marker')) $true 'Cleanup failure still completes uninstall'
    }

    $mixed = New-Install 'mixed old CLI'
    Copy-Item (Join-Path $fresh 'uninstall.exe') $mixed
    $null = New-Item -ItemType File -Path (Join-Path $mixed 'unsupported-cleanup')
    $uninstaller = Start-Process -FilePath (Join-Path $mixed 'uninstall.exe') -ArgumentList "/S _?=$mixed" -PassThru
    $processes.Add($uninstaller)
    Assert-Equal (Wait-Exit $uninstaller) 0 'Pre-sandbox mixed CLI does not block uninstall'
    Assert-Equal (Test-Path (Join-Path $mixed 'uninstalled.marker')) $true 'Mixed install removal completes'

    $hungCleanup = New-Install 'hung cleanup'
    Copy-Item (Join-Path $fresh 'uninstall.exe') $hungCleanup
    $null = New-Item -ItemType File -Path (Join-Path $hungCleanup 'hang-cleanup')
    $startedAt = [DateTime]::UtcNow
    $uninstaller = Start-Process -FilePath (Join-Path $hungCleanup 'uninstall.exe') -ArgumentList "/S _?=$hungCleanup" -PassThru
    $processes.Add($uninstaller)
    Assert-Equal (Wait-Exit $uninstaller) 0 'Hung sandbox cleanup times out without blocking uninstall'
    if (([DateTime]::UtcNow - $startedAt).TotalSeconds -gt 30) { throw 'Hung sandbox cleanup exceeded its bounded timeout' }
    Assert-Equal (Test-Path (Join-Path $hungCleanup 'uninstalled.marker')) $true 'Uninstall continues after cleanup timeout'

    $missingCli = New-Install 'missing CLI'
    Copy-Item (Join-Path $fresh 'uninstall.exe') $missingCli
    Remove-Item (Join-Path $missingCli 'future.exe')
    $uninstaller = Start-Process -FilePath (Join-Path $missingCli 'uninstall.exe') -ArgumentList "/S _?=$missingCli" -PassThru
    $processes.Add($uninstaller)
    Assert-Equal (Wait-Exit $uninstaller) 0 'Missing CLI does not block uninstall'
    Assert-Equal (Test-Path (Join-Path $missingCli 'uninstalled.marker')) $true 'Missing CLI removal completes'

    $lockedUninstall = New-Install 'locked uninstall'
    Copy-Item (Join-Path $fresh 'uninstall.exe') $lockedUninstall
    $lockedUninstallHandle = [IO.File]::Open((Join-Path $lockedUninstall 'future.exe'), 'Open', 'Read', 'Read')
    try {
        $uninstaller = Start-Process -FilePath (Join-Path $lockedUninstall 'uninstall.exe') -ArgumentList "/S _?=$lockedUninstall" -PassThru
        $processes.Add($uninstaller)
        Assert-Equal (Wait-Exit $uninstaller) 32 'External lock blocks uninstall cleanly'
        Assert-Equal (Test-Path (Join-Path $lockedUninstall 'uninstalled.marker')) $false 'Locked uninstall keeps uninstall state intact'
        Assert-Equal (Test-Path (Join-Path $lockedUninstall 'futureos.exe')) $true 'Locked uninstall preserves desktop'
    } finally { $lockedUninstallHandle.Dispose() }

    Write-Host 'Windows installer preflight regressions passed.'
} finally {
    foreach ($p in $processes) {
        if (-not $p.HasExited) { $p.Kill(); $p.WaitForExit() }
        $p.Dispose()
    }
    $env:HOME = $previousHome
    $env:FUTURE_HOME = $previousFutureHome
    Remove-Item -LiteralPath $root -Recurse -Force
}
