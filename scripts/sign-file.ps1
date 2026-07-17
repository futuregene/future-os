#requires -Version 5.1
<#
.SYNOPSIS
    Sign one binary. Invoked by the Tauri bundler via bundle.windows.signCommand.

.DESCRIPTION
    Tauri calls this once per artifact it produces (the app .exe, then the NSIS
    setup and the MSI), substituting the artifact path for %1. A non-zero exit
    aborts the bundle — which is what we want: a half-signed installer is worse
    than a failed build.

    Prefer passing -Thumbprint: the caller resolves the certificate once up
    front, so every artifact in a build is signed by the same certificate and a
    dead SimplySign session is reported before the build starts rather than
    part-way through. Without it, each call resolves the certificate itself.

.PARAMETER Path
    The binary to sign. Tauri substitutes this for the %1 placeholder.

.PARAMETER Thumbprint
    Certificate to sign with. Resolved from the store when omitted.

.PARAMETER CertSubject
    Substring of the certificate Subject, to disambiguate when the store holds
    several code signing certificates. Ignored when -Thumbprint is given.

.PARAMETER TimestampUrl
    RFC 3161 timestamp server. Timestamping is what keeps signatures valid after
    the certificate expires, so it is on by default.

.EXAMPLE
    pwsh scripts/sign-file.ps1 -Path .\FutureOS.exe
    pwsh scripts/sign-file.ps1 -Path .\FutureOS.exe -Thumbprint AABBCC...
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Path,
    [string]$Thumbprint,
    [string]$CertSubject,
    [string]$TimestampUrl = "http://time.certum.pl/"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

. "$PSScriptRoot\lib\windows-signing.ps1"

$signTool = Find-SignTool
if (-not $Thumbprint) {
    $Thumbprint = (Resolve-SigningCert $CertSubject).Thumbprint
}

Invoke-SignFile -SignTool $signTool -Thumbprint $Thumbprint -Path $Path -TimestampUrl $TimestampUrl
Write-Host "    signed: $(Split-Path -Leaf $Path)"
