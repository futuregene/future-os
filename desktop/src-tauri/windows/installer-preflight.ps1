# Embedded in the installer, not installed with the app. Windows PowerShell 5.1+.
# Preflight exits: 0 = writable, 32 = running/locked, 5 = access/check failure.
# ResetSandbox passes through the maintenance CLI code; 124 means timeout.
param(
    [Parameter(Mandatory = $true)][string]$InstallDir,
    [ValidateSet('Check', 'Close', 'ResetSandbox')][string]$Mode = 'Check'
)
$ErrorActionPreference = 'Stop'

function Invoke-FutureOSSandboxReset {
    param([string]$Directory)

    $process = $null
    try {
        $executable = [IO.Path]::Combine([IO.Path]::GetFullPath($Directory), 'future.exe')
        if (-not [IO.File]::Exists($executable)) { return 0 }
        $startInfo = [Diagnostics.ProcessStartInfo]::new()
        $startInfo.FileName = $executable
        $startInfo.Arguments = 'agent --reset-windows-sandbox'
        $startInfo.UseShellExecute = $false
        $startInfo.CreateNoWindow = $true
        $process = [Diagnostics.Process]::Start($startInfo)
        if ($null -eq $process) { return 1 }
        if (-not $process.WaitForExit(15000)) {
            Write-Host 'FutureOS sandbox cleanup timed out; terminating the maintenance process.'
            $process.Kill()
            $null = $process.WaitForExit(5000)
            return 124
        }
        return $process.ExitCode
    } catch {
        Write-Host "FutureOS sandbox cleanup: $($_.Exception.Message)"
        return 5
    } finally {
        if ($null -ne $process) { $process.Dispose() }
    }
}

function Invoke-FutureOSPreflight {
    param([string]$Directory, [string]$Action)

    try {
        $directoryPath = [IO.Path]::GetFullPath($Directory)
        # Current release desktop/CLI names plus both legacy executable names.
        # A pre-unified install can leave future-agent.exe running after the
        # desktop exits and block the new bundled Agent from taking ownership.
        $targets = @('futureos.exe', 'future-desktop.exe', 'future.exe', 'future-agent.exe') | ForEach-Object {
            [IO.Path]::Combine($directoryPath, $_)
        }
        # Desktop first: it must not restart the agent while we are stopping it.
        foreach ($target in $targets) {
            $name = [IO.Path]::GetFileNameWithoutExtension($target)
            foreach ($process in [Diagnostics.Process]::GetProcessesByName($name)) {
                try {
                    # Use the process object (with its cached handle), not a later
                    # PID lookup: a recycled PID must never kill another process.
                    $null = $process.Handle
                    if (-not [string]::Equals($process.MainModule.FileName, $target, [StringComparison]::OrdinalIgnoreCase)) {
                        continue
                    }
                    if ($Action -eq 'Check') { return 32 }
                    # Close is used after interactive consent, by the explicit
                    # in-app update path, or by uninstall. Never kill a process
                    # tree or a process belonging to another installation.
                    $process.Kill()
                    if (-not $process.WaitForExit(10000)) { return 32 }
                } catch {
                    # A process can exit during inspection. Otherwise do not guess
                    # its path or escalate: the write probe below fails closed.
                    Write-Host "Could not inspect/close a ${name} process: $($_.Exception.Message)"
                } finally {
                    $process.Dispose()
                }
            }
        }

        # OPEN_EXISTING, never truncate the installed executables. Checking only
        # the process list misses read-only files, ACLs and non-FutureOS lockers.
        foreach ($target in $targets) {
            if ([IO.File]::Exists($target)) {
                $file = [IO.File]::Open($target, [IO.FileMode]::Open, [IO.FileAccess]::Write, [IO.FileShare]::Read)
                $file.Dispose()
            }
        }
        # Also check creation in the directory (including a first install).
        $null = [IO.Directory]::CreateDirectory($directoryPath)
        $probe = [IO.Path]::Combine($directoryPath, '.futureos-install-' + [Guid]::NewGuid().ToString('N'))
        $file = [IO.FileStream]::new($probe, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None, 1, [IO.FileOptions]::DeleteOnClose)
        $file.Dispose()
        return 0
    } catch [IO.IOException] {
        Write-Host "FutureOS install preflight: $($_.Exception.Message)"
        $code = $_.Exception.HResult -band 0xffff
        if ($code -eq 32 -or $code -eq 33) { return 32 }
        return 5
    } catch {
        Write-Host "FutureOS install preflight: $($_.Exception.Message)"
        return 5
    }
}

if ($Mode -eq 'ResetSandbox') {
    exit (Invoke-FutureOSSandboxReset -Directory $InstallDir)
}
exit (Invoke-FutureOSPreflight -Directory $InstallDir -Action $Mode)
