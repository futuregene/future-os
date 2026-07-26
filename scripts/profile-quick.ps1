<#
.SYNOPSIS
    Run the agent with optional CPU profiling via blondie.
    Called by 'make profile-quick' on Windows.
#>
param(
    [int]$Duration = 30,
    [int]$Port = 50052,
    [string]$Binary = ".\target\release\future-agent.exe"
)

$ErrorActionPreference = "Continue"
$addr = "127.0.0.1:$Port"
$svg = "profile-results/quick-profile.svg"

if (-not (Test-Path "profile-results")) {
    New-Item -ItemType Directory -Force -Path "profile-results" | Out-Null
}

if (-not (Test-Path $Binary)) {
    Write-Host "error: $Binary not found" -ForegroundColor Red
    exit 1
}

# Try blondie first for CPU flamegraph
$blondie = Get-Command blondie -ErrorAction SilentlyContinue
if ($blondie) {
    Write-Host "[INFO] Attempting CPU flamegraph via blondie (requires admin)..."
    $args = @("flamegraph", $Binary, "--", "--grpc-addr", $addr, "--profile-seconds", $Duration, "--verbose")
    $result = & $blondie.Source @args 2>&1
    $exitCode = $LASTEXITCODE
    
    if ($exitCode -eq 0 -and (Test-Path "flamegraph.svg")) {
        Move-Item -Force "flamegraph.svg" $svg
        $sz = (Get-Item $svg).Length
        Write-Host "Flamegraph: $svg ($([Math]::Round($sz/1024, 1)) KB)" -ForegroundColor Green
        exit 0
    }
    
    # blondie failed — show output and fall back
    if ($result -match "NotAnAdmin") {
        Write-Host "[WARN] blondie requires administrator privileges. Run from an elevated prompt for flamegraphs." -ForegroundColor Yellow
    } elseif ($result -match "Error") {
        Write-Host "[WARN] blondie failed: $result" -ForegroundColor Yellow
    }
    Write-Host "[INFO] Falling back to agent-only run (no CPU flamegraph)..."
} else {
    Write-Host "[INFO] blondie not installed. Install for flamegraph support:"
    Write-Host "  cargo install blondie --features inferno,clap"
    Write-Host "[INFO] Running agent without CPU flamegraph..."
}

# Fallback: run agent without profiling
$proc = Start-Process -FilePath $Binary `
    -ArgumentList @("--grpc-addr", $addr, "--profile-seconds", $Duration, "--verbose") `
    -NoNewWindow -Wait -PassThru

if ($proc.ExitCode -ne 0) {
    Write-Host "[WARN] Agent exited with code $($proc.ExitCode)"
} else {
    Write-Host "[INFO] Profile run complete."
}
