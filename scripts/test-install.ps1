# Offline regression: stop at the download boundary; never download or install.
$ErrorActionPreference = 'Stop'
$oldVersion = $env:FUTUREOS_VERSION
$oldBase = $env:FUTUREOS_BASE
function Invoke-WebRequest {
    param($Uri, $OutFile)
    $script:capturedUrl = $Uri
    throw 'test_download_boundary'
}
function Write-Warning {
    param($Message)
    $script:capturedWarning = $Message
}
try {
    $env:FUTUREOS_BASE = 'https://example.invalid/releases'
    foreach ($version in @('v1.2.3', 'V1.2.3', '1.2.3')) {
        $env:FUTUREOS_VERSION = $version
        $script:capturedUrl = $null
        $script:capturedWarning = $null
        try {
            . "$PSScriptRoot/install.ps1"
            throw 'installer unexpectedly crossed the download boundary'
        } catch {
            if ($_.Exception.Message -ne 'test_download_boundary') { throw }
        }
        if ($script:capturedUrl -ne 'https://example.invalid/releases/1.2.3/FutureOS_1.2.3_x64-setup.exe') {
            throw "Incorrect pinned URL: $script:capturedUrl"
        }
        if ($script:capturedWarning -notmatch 'skipping SHA-256') {
            throw 'Missing pinned checksum warning'
        }
    }
    Write-Host 'Pinned installer regression passed.'
} finally {
    $env:FUTUREOS_VERSION = $oldVersion
    $env:FUTUREOS_BASE = $oldBase
}
