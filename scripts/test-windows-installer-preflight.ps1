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
    $installer = Join-Path $root 'preflight-setup.exe'
    & $MakeNsis /V2 /WX /INPUTCHARSET UTF8 "/DTEST_OUTFILE=$installer" (Join-Path $PSScriptRoot 'tests\windows-installer-preflight.nsi')
    if ($LASTEXITCODE -ne 0) { throw 'NSIS compilation failed' }

    Write-Host 'Testing fresh install...'
    $fresh = Join-Path $root 'fresh install'
    Assert-Equal (Wait-Exit (Start-Installer $fresh)) 0 'Fresh install'
    Assert-Equal (Test-Path (Join-Path $fresh 'installed.marker')) $true 'Install completed'

    Write-Host 'Testing process detection, silent and passive modes...'
    # Space, apostrophe and Unicode paths; no command-string interpolation.
    $target = New-Install ("Program Files user's " + [char]0x672A + [char]0x6765)
    $other = New-Install 'other installation'
    $otherAgent = Start-Fixture $other
    Assert-Equal (Invoke-Preflight $target) 0 'Other installation does not block'
    Assert-Unchanged $target
    $agent = Start-Fixture $target
    $desktop = Start-Fixture $target 'futureos.exe'
    $legacyDesktop = Start-Fixture $target 'future-desktop.exe'
    Assert-Equal (Invoke-Preflight $target) 32 'Detect running installation'
    Assert-Equal (Wait-Exit (Start-Installer $target)) 32 'Silent install fails closed'
    Assert-Equal (Wait-Exit (Start-Installer $target '/P')) 32 'Passive install fails closed'
    Assert-Equal $agent.HasExited $false 'Silent mode preserves tasks'
    Assert-Equal $desktop.HasExited $false 'Silent mode preserves desktop'
    Assert-Unchanged $target

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
    Assert-Equal $desktop.HasExited $true 'Target desktop closed'
    Assert-Equal $legacyDesktop.HasExited $true 'Legacy desktop closed'
    Assert-Equal $otherAgent.HasExited $false 'Other installation untouched'

    Write-Host 'Testing external lock and retry...'
    $locked = New-Install 'external lock'
    $handle = [IO.File]::Open((Join-Path $locked 'future.exe'), 'Open', 'Read', 'Read')
    try {
        Assert-Equal (Invoke-Preflight $locked) 32 'External lock detected'
        Assert-Equal (Invoke-Preflight $locked 'Close') 32 'Cannot close unrelated locker'
        Assert-Equal (Wait-Exit (Start-Installer $locked)) 32 'Locked install blocked'
        Assert-Unchanged $locked
    } finally { $handle.Dispose() }
    Assert-Equal (Wait-Exit (Start-Installer $locked)) 0 'Retry succeeds after unlock'

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
    # Fresh uninstall fixture: verify preflight runs before sandbox cleanup.
    $uninstallDir = New-Install 'uninstall'
    Copy-Item (Join-Path $fresh 'uninstall.exe') $uninstallDir
    $uninstallAgent = Start-Fixture $uninstallDir
    $uninstaller = Start-Process -FilePath (Join-Path $uninstallDir 'uninstall.exe') -ArgumentList "/S _?=$uninstallDir" -PassThru
    $processes.Add($uninstaller)
    Assert-Equal (Wait-Exit $uninstaller) 32 'Uninstall checks agent before cleanup'
    Assert-Equal $uninstallAgent.HasExited $false 'Silent uninstall preserves tasks'
    Assert-Equal (Test-Path (Join-Path $uninstallDir 'uninstalled.marker')) $false 'Uninstall did not proceed'
    Assert-Equal (Invoke-Preflight $uninstallDir 'Close') 0 'Close uninstall fixture'
    $null = New-Item -ItemType File -Path (Join-Path $uninstallDir 'fail-cleanup')
    foreach ($flags in @('/S', '/P')) {
        $uninstaller = Start-Process -FilePath (Join-Path $uninstallDir 'uninstall.exe') -ArgumentList "$flags _?=$uninstallDir" -PassThru
        $processes.Add($uninstaller)
        Assert-Equal (Wait-Exit $uninstaller) 1 'Cleanup failure exits unattended uninstall'
        Assert-Equal (Test-Path (Join-Path $uninstallDir 'uninstalled.marker')) $false 'Cleanup failure blocks uninstall'
        Assert-Unchanged $uninstallDir
    }
    Remove-Item -LiteralPath (Join-Path $uninstallDir 'fail-cleanup')
    $uninstaller = Start-Process -FilePath (Join-Path $uninstallDir 'uninstall.exe') -ArgumentList "/S _?=$uninstallDir" -PassThru
    $processes.Add($uninstaller)
    Assert-Equal (Wait-Exit $uninstaller) 0 'Cleanup retry succeeds'
    Assert-Equal (Test-Path (Join-Path $uninstallDir 'uninstalled.marker')) $true 'Uninstall continues after cleanup'

    Write-Host 'Windows installer preflight regressions passed.'
} finally {
    foreach ($p in $processes) {
        if (-not $p.HasExited) { $p.Kill(); $p.WaitForExit() }
        $p.Dispose()
    }
    Remove-Item -LiteralPath $root -Recurse -Force
}
