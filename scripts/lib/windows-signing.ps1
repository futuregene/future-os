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
        [string]$TimestampUrl = "http://time.certum.pl/",
        [int]$Attempts = 3
    )
    if (-not (Test-Path -LiteralPath $Path)) { throw "Nothing to sign at '$Path'." }

    # Both the Certum cloud card and the timestamp server are network-backed, and
    # a build signs a dozen artifacts back to back, so a single request failing
    # (SignerSign() 0x80090020, timestamp timeouts) is routine and not a reason to
    # lose the build. Retry with a backoff; a genuinely broken setup — dead
    # SimplySign session, wrong thumbprint — fails all the attempts anyway.
    for ($i = 1; $i -le $Attempts; $i++) {
        # /fd + /td sha256: SHA-1 is no longer accepted for code signing.
        # /tr (RFC 3161) keeps the signature valid past certificate expiry.
        #
        # Output is captured rather than left to fall through — see
        # Get-SignatureState — and folded into the error, which is otherwise just
        # an exit code.
        $out = & $SignTool sign /sha1 $Thumbprint /fd sha256 /tr $TimestampUrl /td sha256 /q $Path
        if ($LASTEXITCODE -eq 0) { break }

        $detail = "signtool sign failed (exit $LASTEXITCODE) for '$Path'.`n$(($out | Out-String).Trim())"
        if ($i -eq $Attempts) { throw $detail }

        $delay = [Math]::Pow(2, $i) * 5   # 10s, 20s, ...
        Write-Host "    sign attempt $i/$Attempts failed, retrying in ${delay}s"
        Write-Host $detail
        Start-Sleep -Seconds $delay
    }

    $out = & $SignTool verify /pa /q $Path
    if ($LASTEXITCODE -ne 0) {
        throw "signtool verify failed (exit $LASTEXITCODE) for '$Path'.`n$(($out | Out-String).Trim())"
    }
}

# "signed" | "NO-TIMESTAMP" | "UNSIGNED". `signtool verify` says nothing about
# timestamping, and an untimestamped signature dies with the certificate —
# taking every already-shipped artifact with it — so check that separately.
function Get-SignatureState {
    param(
        [Parameter(Mandatory)][string]$SignTool,
        [Parameter(Mandatory)][string]$Path
    )
    # Capture signtool's stdout instead of letting it fall through: anything a
    # function writes to the output stream becomes part of its return value, so
    # an uncaptured line here turns the state into an object[] and every caller's
    # `-eq 'signed'` silently fails.
    $out = & $SignTool verify /pa /q $Path
    if ($LASTEXITCODE -ne 0) {
        if ($out) { Write-Host ($out | Out-String).Trim() }
        return "UNSIGNED"
    }
    if (-not (Get-AuthenticodeSignature -LiteralPath $Path).TimeStamperCertificate) { return "NO-TIMESTAMP" }
    "signed"
}

# Write a `tauri build --config` overlay pointing bundle.windows.signCommand back
# at sign-file.ps1, and return its path. Generated per build rather than
# committed to tauri.conf.json so unsigned builds — local dev, and CI on machines
# without the certificate — are unaffected.
#
# FailLog is how the caller finds out about a signing failure it would otherwise
# never see: see Assert-NoSignFailures.
function New-SignOverlayConfig {
    param(
        [Parameter(Mandatory)][string]$Thumbprint,
        [Parameter(Mandatory)][string]$SignScript,
        [string]$TimestampUrl = "http://time.certum.pl/",
        [string]$FailLog
    )
    $signArgs = @(
        "-NoProfile", "-ExecutionPolicy", "Bypass",
        "-File", $SignScript,
        "-Thumbprint", $Thumbprint,
        "-TimestampUrl", $TimestampUrl
    )
    if ($FailLog) { $signArgs += @("-FailLog", $FailLog) }
    $signArgs += @("-Path", "%1")   # last: Tauri substitutes the artifact path for %1

    # Object notation, not the string form: Tauri splits the string form on
    # spaces, and these paths contain them.
    #
    # cmd is the PowerShell host running us, rather than assuming `pwsh` is on
    # PATH inside the bundler's environment.
    $overlay = @{
        bundle = @{
            windows = @{
                signCommand = @{
                    cmd  = (Get-Process -Id $PID).Path
                    args = $signArgs
                }
            }
        }
    }
    $path = Join-Path ([System.IO.Path]::GetTempPath()) "futureos-sign-overlay-$PID.json"
    $overlay | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $path -Encoding utf8
    $path
}

# A non-zero exit from signCommand aborts the bundle only where Tauri itself is
# the caller. The NSIS uninstaller is signed by makensis instead, through the
# !uninstfinalize hook, and makensis prints the failure and carries on — so a
# build can "succeed" with an unsigned uninstaller inside a correctly signed
# setup.exe. Reading it back out of the installer would mean unpacking an NSIS
# archive; instead sign-file.ps1 records every failure it hits, and this turns a
# non-empty log into the build failure it should have been.
function Assert-NoSignFailures([string]$FailLog) {
    if (-not (Test-Path -LiteralPath $FailLog)) { return }
    $failures = (Get-Content -LiteralPath $FailLog -Raw).Trim()
    if (-not $failures) { return }
    throw @"
Signing failed for one or more artifacts during bundling. The bundler may have
ignored this and produced a partially-signed installer — discard it.

$failures
"@
}
