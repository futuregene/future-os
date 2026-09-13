# Embedded in the installer, not installed with the app. Windows PowerShell 5.1+.
# Exit codes: 0 = writable, 32 = running/locked, 5 = access/check failure.
param(
    [Parameter(Mandatory = $true)][string]$InstallDir,
    [ValidateSet('Check', 'Close')][string]$Mode = 'Check'
)
$ErrorActionPreference = 'Stop'

function Invoke-FutureOSPreflight {
    param([string]$Directory, [string]$Action)

    try {
        $directoryPath = [IO.Path]::GetFullPath($Directory)
        # Current release desktop name, legacy desktop name, then the CLI/agent.
        $targets = @('futureos.exe', 'future-desktop.exe', 'future.exe') | ForEach-Object {
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
                    # Close is called ONLY after an interactive interruption warning.
                    # Do not kill the process tree or processes from other installs.
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

exit (Invoke-FutureOSPreflight -Directory $InstallDir -Action $Mode)
