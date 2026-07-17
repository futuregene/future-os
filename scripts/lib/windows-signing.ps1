#requires -Version 5.1
<#
.SYNOPSIS
    Shared Authenticode signing helpers for the Windows build scripts.

.DESCRIPTION
    Dot-source this file; it defines functions only and runs nothing:

        . "$PSScriptRoot\lib\windows-signing.ps1"

    Used by scripts/sign-file.ps1 (the Tauri signCommand callback) and
    scripts/build-windows-installer.ps1.
#>

Set-StrictMode -Version Latest

# signtool ships with the Windows SDK and is normally not on PATH. Its directory
# carries the SDK version, so resolve it instead of pinning one.
function Find-SignTool {
    $onPath = Get-Command signtool.exe -ErrorAction SilentlyContinue
    if ($onPath) { return $onPath.Source }

    $found = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" `
                           -ErrorAction SilentlyContinue | Sort-Object FullName -Descending
    if (-not $found) {
        throw "signtool.exe not found. Install the Windows SDK with the 'Windows SDK Signing Tools' component."
    }
    $found[0].FullName
}

# Pick the signing certificate from the user's store by capability, not by a
# hard-coded thumbprint (which changes on renewal) or company name (which the CA
# may render differently from the registered name).
function Resolve-SigningCert([string]$Subject) {
    $now = Get-Date
    $certs = @(Get-ChildItem Cert:\CurrentUser\My -CodeSigningCert -ErrorAction SilentlyContinue |
               Where-Object { $_.NotBefore -le $now -and $_.NotAfter -gt $now })

    if ($Subject) {
        $certs = @($certs | Where-Object { $_.Subject -like "*$Subject*" })
    }

    if ($certs.Count -eq 0) {
        $hint = if ($Subject) {
            "No valid code signing certificate matches Subject '*$Subject*'."
        } else {
            "No valid code signing certificate in CurrentUser\My."
        }
        throw @"
$hint

For Certum Code Signing in the Cloud the certificate only appears while the
SimplySign session is live. Check that SimplySign Desktop is logged in (the
session expires after a couple of hours) and that the virtual card is mounted:

    certutil -scinfo
    Get-ChildItem Cert:\CurrentUser\My -CodeSigningCert | Format-List Subject, Thumbprint, NotAfter
"@
    }

    if ($certs.Count -gt 1) {
        $list = ($certs | ForEach-Object { "  $($_.Thumbprint)  $($_.Subject)" }) -join "`n"
        throw @"
Found $($certs.Count) code signing certificates — pass -CertSubject to disambiguate:

$list
"@
    }

    $certs[0]
}

function Invoke-SignFile {
    param(
        [Parameter(Mandatory)][string]$SignTool,
        [Parameter(Mandatory)][string]$Thumbprint,
        [Parameter(Mandatory)][string]$Path,
        [string]$TimestampUrl = "http://time.certum.pl/"
    )
    if (-not (Test-Path -LiteralPath $Path)) { throw "Nothing to sign at '$Path'." }

    # /fd + /td sha256: SHA-1 is no longer accepted for code signing.
    # /tr (RFC 3161) keeps the signature valid past certificate expiry.
    & $SignTool sign /sha1 $Thumbprint /fd sha256 /tr $TimestampUrl /td sha256 /q $Path
    if ($LASTEXITCODE -ne 0) { throw "signtool sign failed (exit $LASTEXITCODE) for '$Path'." }

    & $SignTool verify /pa /q $Path
    if ($LASTEXITCODE -ne 0) { throw "signtool verify failed (exit $LASTEXITCODE) for '$Path'." }
}
