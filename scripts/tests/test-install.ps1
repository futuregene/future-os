# Offline regressions: mock download/process boundaries; never download or install.
$ErrorActionPreference = 'Stop'
$oldVersion = $env:FUTUREOS_VERSION
$oldBase = $env:FUTUREOS_BASE
$oldSkip = $env:FUTUREOS_SKIP_VERIFY
$oldLocalAppData = $env:LOCALAPPDATA
$oldUserProfile = $env:USERPROFILE
$oldTemp = $env:TEMP
$sandbox = Join-Path ([System.IO.Path]::GetTempPath()) ("futureos-install-test-" + [guid]::NewGuid().ToString('n'))
$null = New-Item -ItemType Directory -Path $sandbox -Force
function Invoke-WebRequest {
    param($Uri, $OutFile)
    $script:capturedUrl = $Uri
    throw 'test_download_boundary'
}
function Invoke-RestMethod {
    param($Uri)
    $script:manifestUrl = $Uri
    if ($script:manifestMissing) { throw "test_manifest_missing: $Uri" }
    return $script:releaseManifest
}
function Write-Warning {
    param($Message)
    $script:capturedWarning = $Message
}
# The real thing would find an installed future.exe and run `future init`.
function Get-Command {
    param($Name, $ErrorAction)
    if ($Name -eq 'future.exe') { return $null }
    return Microsoft.PowerShell.Core\Get-Command -Name $Name -ErrorAction SilentlyContinue
}
# What the mocked download writes; its hash is what the manifest publishes.
$payload = [Text.Encoding]::UTF8.GetBytes('futureos-test-installer')
$payloadSha = [BitConverter]::ToString(
    [Security.Cryptography.SHA256]::Create().ComputeHash($payload)).Replace('-', '').ToLowerInvariant()
$manifestDownload = 'https://example.invalid/releases/1.2.3/FutureOS_1.2.3_x64-setup.exe'
$script:releaseManifest = @{
    version = '1.2.3'
    assets  = @{ 'windows-x86_64' = @{ url = $manifestDownload; sha256 = $payloadSha } }
}
try {
    $env:FUTUREOS_BASE = 'https://example.invalid/releases'
    $env:FUTUREOS_VERSION = $null
    $env:FUTUREOS_SKIP_VERIFY = $null
    # Keep every installer path inside the sandbox: nothing may touch a real
    # installation or a real user profile.
    $env:TEMP = $sandbox
    $env:LOCALAPPDATA = $sandbox
    $env:USERPROFILE = $sandbox

    # A pinned version must read that release's own manifest instead of
    # trusting a URL the script constructs itself.
    foreach ($version in @('v1.2.3', 'V1.2.3', '1.2.3')) {
        $env:FUTUREOS_VERSION = $version
        $script:capturedUrl = $null
        $script:capturedWarning = $null
        $script:manifestUrl = $null
        $script:manifestMissing = $false
        try {
            . "$PSScriptRoot/../release/install.ps1"
            throw 'installer unexpectedly crossed the download boundary'
        } catch {
            if ($_.Exception.Message -ne 'test_download_boundary') { throw }
        }
        if ($script:manifestUrl -ne 'https://example.invalid/releases/1.2.3/latest.json') {
            throw "Pinned installer did not read the version manifest: $($script:manifestUrl)"
        }
        if ($script:capturedUrl -ne $manifestDownload) {
            throw "Incorrect pinned URL: $($script:capturedUrl)"
        }
        if ($script:capturedWarning) {
            throw "Unexpected warning on the verified path: $($script:capturedWarning)"
        }
    }
    Write-Host 'Pinned installer manifest regression passed.'

    # No manifest for that version: refuse rather than install unverified.
    $script:manifestMissing = $true
    $env:FUTUREOS_VERSION = 'v1.2.3'
    $script:capturedUrl = $null
    try {
        . "$PSScriptRoot/../release/install.ps1"
        throw 'installer unexpectedly continued without a manifest'
    } catch {
        if (-not $_.Exception.Message.Contains('cannot be verified')) { throw }
    }
    if ($script:capturedUrl) { throw 'Unverified download started without an explicit opt-in' }
    Write-Host 'Pinned installer fail-closed regression passed.'

    # ... and the explicit opt-in still installs, with a warning that names it.
    $script:capturedUrl = $null
    $script:capturedWarning = $null
    $env:FUTUREOS_SKIP_VERIFY = '1'
    try {
        . "$PSScriptRoot/../release/install.ps1"
        throw 'installer unexpectedly crossed the download boundary'
    } catch {
        if ($_.Exception.Message -ne 'test_download_boundary') { throw }
    }
    if ($script:capturedUrl -ne 'https://example.invalid/releases/1.2.3/FutureOS_1.2.3_x64-setup.exe') {
        throw "Incorrect unverified URL: $($script:capturedUrl)"
    }
    if ($script:capturedWarning -notmatch 'FUTUREOS_SKIP_VERIFY') {
        throw 'Missing explicit opt-in warning'
    }
    Write-Host 'Pinned installer opt-in regression passed.'

    # A failed silent install must explain how to recover, and must not proceed
    # to `future init`. Both external side effects remain mocked; the download
    # writes the bytes whose hash the manifest publishes so the real
    # Get-FileHash confirms them.
    $script:manifestMissing = $false
    $env:FUTUREOS_VERSION = 'v1.2.3'
    $env:FUTUREOS_SKIP_VERIFY = $null
    function Invoke-WebRequest {
        param($Uri, $OutFile)
        $script:capturedUrl = $Uri
        [System.IO.File]::WriteAllBytes($OutFile, $payload)
    }
    function Start-Process {
        param($FilePath, $ArgumentList, [switch]$Wait, [switch]$PassThru)
        return [pscustomobject]@{ ExitCode = $script:installerExitCode }
    }
    foreach ($case in @(
        @{ Code = 32; Message = 'still running or its files are locked' },
        @{ Code = 5; Message = 'write access' },
        @{ Code = 1234; Message = 'exited with code 1234' }
    )) {
        $script:installerExitCode = $case.Code
        try {
            . "$PSScriptRoot/../release/install.ps1"
            throw 'installer unexpectedly continued after failure'
        } catch {
            if (-not $_.Exception.Message.Contains($case.Message) -or
                -not $_.Exception.Message.Contains('interactively')) { throw }
        }
    }
    Write-Host 'Installer failure guidance regressions passed.'
} finally {
    $env:FUTUREOS_VERSION = $oldVersion
    $env:FUTUREOS_BASE = $oldBase
    $env:FUTUREOS_SKIP_VERIFY = $oldSkip
    $env:LOCALAPPDATA = $oldLocalAppData
    $env:USERPROFILE = $oldUserProfile
    $env:TEMP = $oldTemp
    Remove-Item -Recurse -Force $sandbox -ErrorAction SilentlyContinue
}
