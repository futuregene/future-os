# Embedded in the installer, not installed with the app. Windows PowerShell 5.1+.
# Preflight exits: 0 = success, 32 = running/locked, 5 = access/check failure.
# ResetSandbox passes through the maintenance CLI code; 124 means timeout.
param(
    [Parameter(Mandatory = $true)][string]$InstallDir,
    [ValidateSet('Check', 'Close', 'CloseFiles', 'ResetSandbox', 'VerifyInstall', 'AcquireLease', 'HoldLease', 'ReleaseLease')][string]$Mode = 'Check',
    [string]$LeaseDir,
    [int]$InstallerPid
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

function Invoke-FutureOSInstallVerification {
    param([string]$Directory)

    try {
        $directoryPath = [IO.Path]::GetFullPath($Directory)
        # Both commands return before GUI/Agent initialization. Starting the
        # real files still makes Windows resolve their native dependencies, so
        # corrupt files, wrong architectures and missing DLLs fail here.
        foreach ($probe in @(
            @{ Name = 'futureos.exe'; Arguments = '--help' },
            @{ Name = 'future.exe'; Arguments = '--version' }
        )) {
            $executable = [IO.Path]::Combine($directoryPath, $probe.Name)
            if (-not [IO.File]::Exists($executable)) {
                Write-Host "FutureOS install verification: missing $executable"
                return 5
            }
            $process = $null
            try {
                $startInfo = [Diagnostics.ProcessStartInfo]::new()
                $startInfo.FileName = $executable
                $startInfo.Arguments = $probe.Arguments
                $startInfo.WorkingDirectory = $directoryPath
                $startInfo.UseShellExecute = $false
                $startInfo.CreateNoWindow = $true
                $process = [Diagnostics.Process]::Start($startInfo)
                if ($null -eq $process) { return 5 }
                if (-not $process.WaitForExit(15000)) {
                    Write-Host "FutureOS install verification timed out: $($probe.Name)"
                    $process.Kill()
                    $null = $process.WaitForExit(5000)
                    return 5
                }
                if ($process.ExitCode -ne 0) {
                    Write-Host "FutureOS install verification failed: $($probe.Name) exited $($process.ExitCode)"
                    return 5
                }
            } finally {
                if ($null -ne $process) { $process.Dispose() }
            }
        }
        return 0
    } catch {
        Write-Host "FutureOS install verification: $($_.Exception.Message)"
        return 5
    }
}

function Get-FutureOSAgentStateDirectory {
    $root = $env:FUTURE_HOME
    if (-not $root -or -not [IO.Path]::IsPathRooted($root)) {
        $profile = @($env:HOME, $env:USERPROFILE, [Environment]::GetFolderPath('UserProfile')) |
            Where-Object { $_ -and [IO.Path]::IsPathRooted($_) } | Select-Object -First 1
        if (-not $profile) { throw 'Cannot resolve the FutureOS user profile for Agent lock verification.' }
        $root = [IO.Path]::Combine($profile, '.future')
    }
    return [IO.Path]::Combine([IO.Path]::GetFullPath($root), 'agent')
}

function Test-FutureOSAgentLock {
    param([string]$Action)
    $stateDirectory = Get-FutureOSAgentStateDirectory
    $null = [IO.Directory]::CreateDirectory($stateDirectory)
    $lockPath = [IO.Path]::Combine($stateDirectory, 'agent-instance.lock')
    $metadataPath = [IO.Path]::Combine($stateDirectory, 'agent-instance.json')
    $deadline = [DateTime]::UtcNow.AddSeconds($(if ($Action -eq 'Close') { 10 } else { 0 }))
    do {
        $stream = $null
        try {
            $stream = [IO.FileStream]::new($lockPath, [IO.FileMode]::OpenOrCreate,
                [IO.FileAccess]::ReadWrite, [IO.FileShare]::ReadWrite)
            $stream.Lock(0, 1)
            $stream.Unlock(0, 1)
            return 0
        } catch [IO.IOException] {
            $code = $_.Exception.HResult -band 0xffff
            if ($code -ne 32 -and $code -ne 33) { throw }
            # A locked first byte cannot be read as a PID on Windows. Read the
            # separate metadata only for process identification.
            $owner = $null
            try {
                if ([IO.File]::Exists($metadataPath)) {
                    $info = Get-Content -LiteralPath $metadataPath -Raw | ConvertFrom-Json
                    $owner = [Diagnostics.Process]::GetProcessById([int]$info.pid)
                    $executable = [IO.Path]::GetFullPath($owner.MainModule.FileName)
                    $recorded = [IO.Path]::GetFullPath([string]$info.executable)
                    $recordedHome = [IO.Path]::GetFullPath([string]$info.futureHome)
                    $expectedHome = [IO.Path]::GetFullPath([IO.Path]::GetDirectoryName($stateDirectory))
                    $name = [IO.Path]::GetFileNameWithoutExtension($executable)
                    $verified = [string]::Equals($executable, $recorded, [StringComparison]::OrdinalIgnoreCase) -and
                        [string]::Equals($recordedHome, $expectedHome, [StringComparison]::OrdinalIgnoreCase) -and
                        @('future', 'future-agent').Contains($name.ToLowerInvariant()) -and
                        ([int64]$owner.StartTime.ToFileTimeUtc() -eq [int64]$info.startTimeFiletime)
                    if ($verified) {
                        Write-Host "FutureOS Agent lock held by PID $($owner.Id): $executable"
                        if ($Action -eq 'Close') {
                            $owner.Kill()
                            if (-not $owner.WaitForExit(10000)) { return 32 }
                        }
                    } else {
                        Write-Host 'FutureOS Agent lock metadata does not match its recorded process.'
                    }
                } else {
                    Write-Host 'FutureOS Agent lock is occupied (legacy Agent or missing metadata).'
                }
            } catch {
                Write-Host "FutureOS Agent lock is occupied; its owner could not be verified: $($_.Exception.Message)"
            } finally {
                if ($null -ne $owner) { $owner.Dispose() }
            }
            if ($Action -ne 'Close' -or [DateTime]::UtcNow -ge $deadline) { return 32 }
            Start-Sleep -Milliseconds 100
        } finally {
            if ($null -ne $stream) { $stream.Dispose() }
        }
    } while ($true)
}

function Invoke-FutureOSAgentLease {
    param([string]$Action, [string]$Directory, [int]$OwnerPid)
    if (-not $Directory) { return 5 }
    $ready = [IO.Path]::Combine($Directory, 'agent-lock-ready')
    $errorFile = [IO.Path]::Combine($Directory, 'agent-lock-error')
    $stop = [IO.Path]::Combine($Directory, 'agent-lock-stop')
    $done = [IO.Path]::Combine($Directory, 'agent-lock-done')
    if ($Action -eq 'AcquireLease') {
        foreach ($marker in @($ready, $errorFile, $stop, $done)) {
            Remove-Item -LiteralPath $marker -ErrorAction SilentlyContinue
        }
        $powershell = [Diagnostics.Process]::GetCurrentProcess().MainModule.FileName
        $arguments = '-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "' + $PSCommandPath +
            '" -InstallDir "' + $InstallDir + '" -Mode HoldLease -LeaseDir "' + $Directory +
            '" -InstallerPid ' + $OwnerPid
        $child = Start-Process -FilePath $powershell -ArgumentList $arguments -WindowStyle Hidden -PassThru
        try {
            $deadline = [DateTime]::UtcNow.AddSeconds(10)
            while ([DateTime]::UtcNow -lt $deadline) {
                if ([IO.File]::Exists($ready)) { return 0 }
                if ([IO.File]::Exists($errorFile)) { return [int][IO.File]::ReadAllText($errorFile) }
                if ($child.HasExited) { return 5 }
                Start-Sleep -Milliseconds 50
            }
            [IO.File]::WriteAllText($stop, 'stop')
            return 5
        } finally { $child.Dispose() }
    }
    if ($Action -eq 'ReleaseLease') {
        if (-not [IO.File]::Exists($ready)) { return 0 }
        [IO.File]::WriteAllText($stop, 'stop')
        $deadline = [DateTime]::UtcNow.AddSeconds(10)
        while (-not [IO.File]::Exists($done) -and [DateTime]::UtcNow -lt $deadline) {
            Start-Sleep -Milliseconds 50
        }
        if (-not [IO.File]::Exists($done)) { return 5 }
        return 0
    }
    # Keep the one-byte lock for the entire NSIS replacement. The owner
    # process handle makes an installer crash release the lease as well.
    $stream = $null
    $installer = $null
    try {
        $stateDirectory = Get-FutureOSAgentStateDirectory
        $null = [IO.Directory]::CreateDirectory($stateDirectory)
        $lockPath = [IO.Path]::Combine($stateDirectory, 'agent-instance.lock')
        $installer = [Diagnostics.Process]::GetProcessById($OwnerPid)
        $stream = [IO.FileStream]::new($lockPath, [IO.FileMode]::OpenOrCreate,
            [IO.FileAccess]::ReadWrite, [IO.FileShare]::ReadWrite)
        $stream.Lock(0, 1)
        [IO.File]::WriteAllText($ready, [string]$PID)
        while (-not [IO.File]::Exists($stop) -and -not $installer.HasExited) {
            Start-Sleep -Milliseconds 100
        }
        $stream.Unlock(0, 1)
        [IO.File]::WriteAllText($done, 'done')
        return 0
    } catch [IO.IOException] {
        [IO.File]::WriteAllText($errorFile, '32')
        return 32
    } catch {
        [IO.File]::WriteAllText($errorFile, '5')
        return 5
    } finally {
        if ($null -ne $stream) { $stream.Dispose() }
        if ($null -ne $installer) { $installer.Dispose() }
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

        # The Agent lock is user-home scoped, so an Agent from another install
        # can survive exact-path cleanup. Never replace binaries while that
        # Agent still owns this FutureOS home.
        if ($Action -ne 'CloseFiles') {
            $lockStatus = Test-FutureOSAgentLock -Action $Action
            if ($lockStatus -ne 0) { return $lockStatus }
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

if ($Mode -in @('AcquireLease', 'HoldLease', 'ReleaseLease')) {
    exit (Invoke-FutureOSAgentLease -Action $Mode -Directory $LeaseDir -OwnerPid $InstallerPid)
}
if ($Mode -eq 'ResetSandbox') {
    exit (Invoke-FutureOSSandboxReset -Directory $InstallDir)
}
if ($Mode -eq 'VerifyInstall') {
    exit (Invoke-FutureOSInstallVerification -Directory $InstallDir)
}
exit (Invoke-FutureOSPreflight -Directory $InstallDir -Action $Mode)
