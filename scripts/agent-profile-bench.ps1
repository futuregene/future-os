<#
.SYNOPSIS
    Windows equivalent of agent-profile-bench.sh.
    Uses blondie (ETW-based CPU sampler) to trace the agent, drives load with
    'future run' prompts, and writes a flamegraph SVG.
.DESCRIPTION
    Expects the profiled binary to already exist at .\target\release\future-agent.exe
    (built by the 'make profile-agent' target).
    Requires administrator privileges for ETW kernel tracing.
    Outputs:
      profile-results/agent-profile-<ts>.svg   flamegraph
      profile-results/agent-profile-<ts>.log   agent stdout/stderr
.PARAMETER Duration
    Profile duration in seconds (overrides PROFILE_DURATION env var).
    Default: 90 (or $env:PROFILE_DURATION if set).
.PARAMETER Port
    gRPC port for the agent. Default: 50052 (or $env:PROFILE_PORT if set).
#>
param(
    [int]$Duration = 90,
    [int]$Port = 50052
)

# Environment variable overrides (Match bash script behaviour)
if ($env:PROFILE_DURATION) { $Duration = [int]$env:PROFILE_DURATION }
if ($env:PROFILE_PORT)     { $Port     = [int]$env:PROFILE_PORT }

$ErrorActionPreference = "Stop"
$addr = "127.0.0.1:$Port"
$ts = Get-Date -Format "yyyyMMdd-HHmmss"
$svg = "profile-results/agent-profile-$ts.svg"
$log = "profile-results/agent-profile-$ts.log"
$binary = "$PSScriptRoot\..\target\release\future-agent.exe"
# Resolve to absolute path (blondie needs the real path)
$binary = [System.IO.Path]::GetFullPath($binary)

# Ensure output directory exists
if (-not (Test-Path "profile-results")) {
    New-Item -ItemType Directory -Force -Path "profile-results" | Out-Null
}

if (-not (Test-Path $binary)) {
    Write-Host "error: $binary not found — run the build step first" -ForegroundColor Red
    exit 1
}

# Check that blondie is available
$blondieExe = Get-Command blondie -ErrorAction SilentlyContinue
if (-not $blondieExe) {
    Write-Host "blondie not found. Install it with: cargo install blondie --features inferno" -ForegroundColor Yellow
    Write-Host "Note: CPU profiling on Windows requires administrator privileges."
    exit 1
}

Write-Host "Starting profiled agent on $addr for ${Duration}s ..."
Write-Host "Using blondie for ETW-based CPU sampling (requires admin)."

# blondie flamegraph <binary> <args...> launches the binary, traces it,
# and writes flamegraph.svg when the process exits.
# We run blondie in the background while we drive load via gRPC.
$blondieArgs = @(
    "flamegraph",
    $binary,
    "--",
    "--grpc-addr", $addr,
    "--profile-seconds", $Duration,
    "--verbose"
)

$blondiePsi = New-Object System.Diagnostics.ProcessStartInfo
$blondiePsi.FileName = $blondieExe.Source
$blondiePsi.Arguments = $blondieArgs -join ' '
$blondiePsi.UseShellExecute = $false
$blondiePsi.RedirectStandardOutput = $true
$blondiePsi.RedirectStandardError = $true

$blondieProc = [System.Diagnostics.Process]::Start($blondiePsi)
if (-not $blondieProc) {
    Write-Host "error: failed to start blondie" -ForegroundColor Red
    exit 1
}

# Capture blondie output in background
$blondieOutput = New-Object System.Text.StringBuilder
$blondieOutputEvent = Register-ObjectEvent -InputObject $blondieProc `
    -EventName OutputDataReceived -Action {
        param($sender, $e)
        if ($e.Data) { $Event.MessageData.AppendLine($e.Data) | Out-Null }
    } -MessageData $blondieOutput
$blondieErrorEvent = Register-ObjectEvent -InputObject $blondieProc `
    -EventName ErrorDataReceived -Action {
        param($sender, $e)
        if ($e.Data) { $Event.MessageData.AppendLine($e.Data) | Out-Null }
    } -MessageData $blondieOutput
$blondieProc.BeginOutputReadLine()
$blondieProc.BeginErrorReadLine()

try {
    # Wait for gRPC port to accept connections (max ~30s — blondie needs time
    # to start the ETW session AND launch the agent)
    Write-Host -NoNewline "Waiting for agent to come up"
    $up = $false
    for ($i = 0; $i -lt 300; $i++) {
        try {
            $tcp = New-Object Net.Sockets.TcpClient
            $tcp.Connect("127.0.0.1", $Port)
            $tcp.Close()
            $tcp.Dispose()
            Write-Host " — up."
            $up = $true
            break
        } catch {
            # Port not ready yet
        }
        if ($blondieProc.HasExited) {
            Write-Host ""
            Write-Host "error: blondie/agent exited during startup (code $($blondieProc.ExitCode))" -ForegroundColor Red
            Write-Host $blondieOutput.ToString()
            exit 1
        }
        Write-Host -NoNewline "."
        Start-Sleep -Milliseconds 100
    }
    if (-not $up) {
        Write-Host ""
        Write-Host "error: agent did not start within 30s" -ForegroundColor Red
        Write-Host $blondieOutput.ToString()
        exit 1
    }

    # Drive load with 'future run' one-shot prompts
    $future = Get-Command future -ErrorAction SilentlyContinue
    if ($future) {
        Write-Host "Driving load with 'future run' one-shot prompts ..."
        $prompts = @(
            "Summarise what this repository does in three sentences.",
            "List the main entry points of the Rust agent crate.",
            "Explain how session persistence works, briefly."
        )
        $n = $prompts.Count
        $step = [Math]::Max(3, [Math]::Floor(($Duration - 10) / $n))
        foreach ($p in $prompts) {
            if ($blondieProc.HasExited) { break }
            Write-Host "  -> $p"
            try {
                $null = future run --grpc-addr $addr $p 2>&1
            } catch {
                if ($blondieProc.HasExited) {
                    Write-Host "    (cut short by profile timer - expected)"
                } else {
                    Write-Host "    (prompt failed - continuing)"
                }
            }
            Start-Sleep -Seconds $step
        }
    } else {
        Write-Host "'future' CLI not found - agent will profile idle/shutdown only."
        Write-Host "Install it with 'make install' or drive load manually against $addr."
    }

    Write-Host "Waiting for profile timer to expire and flamegraph to be written ..."
    $blondieProc.WaitForExit()
} finally {
    # Cleanup event handlers
    if ($blondieOutputEvent) { Unregister-Event -SourceIdentifier $blondieOutputEvent.Name -ErrorAction SilentlyContinue }
    if ($blondieErrorEvent) { Unregister-Event -SourceIdentifier $blondieErrorEvent.Name -ErrorAction SilentlyContinue }

    if ($blondieProc -and !$blondieProc.HasExited) {
        $blondieProc.Kill()
    }
}

# Save blondie output to log
$blondieOutput.ToString() | Out-File -FilePath $log -Encoding UTF8

# blondie writes flamegraph.svg in the working directory
if (Test-Path "flamegraph.svg") {
    Move-Item -Force "flamegraph.svg" $svg
}

if (Test-Path $svg) {
    $sz = (Get-Item $svg).Length
    Write-Host "Done: $svg ($([Math]::Round($sz/1024, 1)) KB)"
} else {
    Write-Host "warning: flamegraph not found at $svg - check $log" -ForegroundColor Yellow
    exit 1
}
