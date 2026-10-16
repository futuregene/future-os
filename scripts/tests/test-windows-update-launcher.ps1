# Offline observer fixtures. No real installer, Agent, HOME or sandbox state.
# Invoke with the C# fixture built by test-windows-installer-preflight.ps1.
param([Parameter(Mandatory = $true)][string]$Fixture)
$ErrorActionPreference = 'Stop'
$root = Join-Path ([IO.Path]::GetTempPath()) ('futureos-observer-' + [Guid]::NewGuid().ToString('N'))
$script = Join-Path $PSScriptRoot '../../desktop/src-tauri/windows/update-launcher.ps1'
$children = @()
try {
    foreach ($case in @('success', 'launch-failure', 'installer-failure', 'rollback-incomplete', 'parent-timeout')) {
        $appDir = Join-Path $root "$case/app"
        $staging = Join-Path $root "$case/staged"
        $null = New-Item -ItemType Directory -Path $appDir, $staging -Force
        $desktop = Join-Path $appDir 'futureos.exe'
        Copy-Item -LiteralPath $Fixture -Destination $desktop
        Set-Content -LiteralPath (Join-Path $appDir 'record-launch') -Value 'record'
        $installer = Join-Path $staging 'installer.exe'
        # Missing EXE gives a deterministic launch failure without Windows'
        # modal "unsupported 16-bit application" dialog for malformed EXEs.
        if ($case -ne 'launch-failure') { Copy-Item -LiteralPath $Fixture -Destination $installer }
        if ($case -eq 'installer-failure') {
            Set-Content -LiteralPath (Join-Path $staging 'fail-cleanup') -Value 'fail'
        }
        if ($case -eq 'rollback-incomplete') {
            Set-Content -LiteralPath (Join-Path $staging 'corrupt-desktop') -Value $desktop
        }
        $parentReady = Join-Path $appDir 'parent.ready'
        $parent = Start-Process -FilePath $desktop -ArgumentList "--hold `"$parentReady`"" -PassThru
        $children += $parent
        $deadline = [DateTime]::UtcNow.AddSeconds(10)
        while (-not (Test-Path -LiteralPath $parentReady)) {
            if ($parent.HasExited -or [DateTime]::UtcNow -gt $deadline) { throw 'Parent fixture did not start' }
            Start-Sleep -Milliseconds 50
        }
        $record = Join-Path $root "$case/result.json"
        @{ formatVersion = 1; operationId = $case; fromVersion = 'old'; targetVersion = 'new'; stage = 'awaiting-exit'; outcome = $null; diagnostic = $null; restartSafe = $false } | ConvertTo-Json | Set-Content -LiteralPath $record
        $log = Join-Path $root "$case/observer.log"
        $powershell = [Diagnostics.Process]::GetCurrentProcess().MainModule.FileName
        $parentTimeout = $(if ($case -eq 'parent-timeout') { 5000 } else { 60000 })
        $observer = Start-Process -FilePath $powershell -ArgumentList "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File `"$script`" -ParentPid $($parent.Id) -Desktop `"$desktop`" -Directory `"$staging`" -Log `"$log`" -Record `"$record`" -ParentExitTimeoutMs $parentTimeout" -PassThru
        $children += $observer
        $deadline = [DateTime]::UtcNow.AddSeconds(10)
        while (-not (Test-Path -LiteralPath (Join-Path $staging 'ready'))) {
            if ($observer.HasExited -or [DateTime]::UtcNow -gt $deadline) { throw 'Observer did not become ready' }
            Start-Sleep -Milliseconds 50
        }
        $pending = [IO.File]::ReadAllText($record) | ConvertFrom-Json
        if ($pending.stage -ne 'awaiting-exit' -or $null -ne $pending.outcome) { throw 'Observer readiness was mistaken for installation success' }
        $leaseDenied = $false
        try {
            $unexpected = [IO.File]::Open((Join-Path ([IO.Path]::GetDirectoryName($record)) 'observer.lock'), [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::ReadWrite)
            $unexpected.Dispose()
        } catch [IO.IOException] { $leaseDenied = $true }
        if (-not $leaseDenied) { throw 'Live observer did not exclude repair acknowledgement' }
        # It must still be waiting for the exact Desktop, with no installer log.
        if (Test-Path -LiteralPath $log) { throw 'Installation started before Desktop exited' }
        if ($case -ne 'parent-timeout') {
            $parent.Kill()
            $null = $parent.WaitForExit(5000)
        }
        if (-not $observer.WaitForExit(15000)) { throw 'Observer did not finish' }
        $expected = $(if ($case -eq 'success') { 0 } else { 5 })
        if ($observer.ExitCode -ne $expected) { throw "Wrong observer exit for ${case}: $($observer.ExitCode)" }
        $recoveryExpected = $case -in @('launch-failure', 'installer-failure')
        $deadline = [DateTime]::UtcNow.AddSeconds(5)
        while ($recoveryExpected -and -not (Test-Path (Join-Path $appDir 'recovered.ready')) -and [DateTime]::UtcNow -lt $deadline) {
            Start-Sleep -Milliseconds 50
        }
        if ((Test-Path (Join-Path $appDir 'recovered.ready')) -ne $recoveryExpected) { throw "Unsafe/missing recovery for $case" }
        if (-not (Test-Path -LiteralPath $log)) { throw 'Missing installer diagnostics' }
        $result = [IO.File]::ReadAllText($record) | ConvertFrom-Json
        $outcome = $(if ($case -eq 'success') { 'installed' } elseif ($case -eq 'rollback-incomplete') { 'repair-required' } elseif ($case -eq 'parent-timeout') { 'not-installed' } else { 'failed-restored' })
        if ($result.outcome -ne $outcome) { throw "Wrong durable outcome for ${case}: $($result.outcome)" }
        $restartExpected = $case -notin @('rollback-incomplete', 'parent-timeout')
        if ($result.restartSafe -ne $restartExpected) { throw "Wrong recovery safety for $case" }
        if ($case -eq 'parent-timeout') {
            if ($parent.HasExited) { throw 'Observer terminated a parent that did not exit' }
            if ((Get-Content -LiteralPath $log -Raw).Contains('Installer exit=')) { throw 'Timed-out handoff started installation' }
        }
        if (Test-Path -LiteralPath $staging) { throw 'Staging was not cleaned up' }
        Write-Host "PASS: update observer $case"
    }
} catch {
    Get-ChildItem -LiteralPath $root -Filter observer.log -Recurse -ErrorAction SilentlyContinue |
        ForEach-Object { Write-Host $_.FullName; Get-Content -LiteralPath $_.FullName }
    throw
} finally {
    foreach ($child in $children) {
        if (-not $child.HasExited) { $child.Kill(); $null = $child.WaitForExit(5000) }
        $child.Dispose()
    }
    Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
}
