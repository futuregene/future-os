#!/usr/bin/env pwsh
# ---------------------------------------------------------------------------
# Diagnostic: compare exit-code strategies for the Windows shell wrapper.
#
# The agent's PowerShell wrapper (agent/src/sandbox/mod.rs::windows_wrapper_script)
# decides a command's exit code. For NATIVE .exe commands it uses $LASTEXITCODE
# (reliable). The open question is the FALLBACK for pure-PowerShell commands that
# run no native process: today we use `$Error.Count -gt 0`; Claude Code uses `$?`.
#
#   $Error.Count : cumulative — ANY error during the command counts as failure,
#                  even one the command handled (try/catch) or recovered from.
#   $?           : reflects only the LAST statement — matches bash's
#                  "exit code = last command" convention, fewer false failures.
#
# This script runs a matrix through THREE wrapper variants, using real
# -EncodedCommand invocation (base64/UTF-16LE, same as production), and prints
# each variant's exit code side by side plus an output preview.
#
#   CURRENT  : ForEach-Object pipe + $Error.Count fallback   (what we ship now)
#   QSTREAM  : drop the pipe so the command is last, capture $? (streaming kept,
#              but PowerShell's default error rendering replaces ForEach `"$_"`)
#   QBUF     : buffer output, capture $?, then stringify with ForEach-Object
#              (clean output like today, but loses incremental streaming)
#
# Read the table: `N OK` = exit code N matches expectation; `N XX` = mismatch.
#
# OUTCOME (verified on Windows PowerShell 5.1): KEEP $Error.Count — do NOT switch
# to $?. Because the wrapper uses `2>&1` (needed to merge stderr into captured
# output), errors become output objects and $? reports SUCCESS for "command not
# found" and Write-Error — i.e. $? reintroduces the silent-failure bug we set out
# to fix. $Error.Count's only downside is a false FAILURE on handled try/catch and
# fail-then-succeed (rare, and safe because visible). So the wrapper fallback is
# left as-is; this harness stays as the record of why, plus the authoritative
# raw-byte UTF-8 encoding check at the bottom.
#
# Usage (Windows PowerShell 5.1 or pwsh 7):
#   pwsh -NoProfile -File scripts\test-ps-exitcode.ps1
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\test-ps-exitcode.ps1
# ---------------------------------------------------------------------------

$ErrorActionPreference = 'Continue'
Set-Location (Split-Path -Parent $PSScriptRoot)

# Resolve the shell the agent would use: pwsh 7 preferred, else Windows PowerShell.
$shell = if (Get-Command pwsh -ErrorAction SilentlyContinue) { 'pwsh' } else { 'powershell' }
$ver = & $shell -NoProfile -NoLogo -Command '$PSVersionTable.PSVersion.ToString()'
# Decode child output as UTF-8 in THIS (parent) console too, so the stdout
# previews below aren't garbled by a GBK console mis-reading the child's UTF-8.
# (This only affects the previews; the authoritative encoding test reads raw
# bytes and is unaffected by console encoding.)
try { [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false) } catch {}
Write-Host ""
Write-Host "Windows shell exit-code strategy comparison"
Write-Host "Shell: $shell  (v$ver)"

# Shared prologue (literal here-string — the $ stay literal in the wrapper).
$prologue = @'
chcp 65001 > $null
$OutputEncoding = [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$ProgressPreference = 'SilentlyContinue'
$global:LASTEXITCODE = $null
'@

# --- wrapper builders. $cmd interpolates; every other $ is escaped `$ ----------
function Build-Cur([string]$cmd) {
@"
$prologue
& { $cmd } 2>&1 | ForEach-Object { "`$_" }
if (`$null -ne `$LASTEXITCODE) { exit `$LASTEXITCODE }
elseif (`$Error.Count -gt 0) { exit 1 }
else { exit 0 }
"@
}

function Build-QStream([string]$cmd) {
@"
$prologue
& { $cmd } 2>&1
`$ok = `$?; `$code = `$LASTEXITCODE
if (`$null -ne `$code) { exit `$code }
elseif (`$ok) { exit 0 }
else { exit 1 }
"@
}

function Build-QBuf([string]$cmd) {
@"
$prologue
`$out = & { $cmd } 2>&1
`$ok = `$?; `$code = `$LASTEXITCODE
`$out | ForEach-Object { "`$_" }
if (`$null -ne `$code) { exit `$code }
elseif (`$ok) { exit 0 }
else { exit 1 }
"@
}

# Run one wrapper via -EncodedCommand exactly as the agent does, return its code.
function Invoke-Variant([string]$shellExe, [string]$script) {
  $bytes = [System.Text.Encoding]::Unicode.GetBytes($script)   # UTF-16LE
  $enc = [Convert]::ToBase64String($bytes)
  $out = & $shellExe -NoProfile -NonInteractive -NoLogo -EncodedCommand $enc 2>&1 | Out-String
  return [PSCustomObject]@{ Code = $LASTEXITCODE; Out = $out.TrimEnd() }
}

function Test-Match($code, $expect) {
  if ($expect -is [int]) { return ($code -eq $expect) }
  if ($expect -eq 'non-0') { return ($code -ne 0) }
  return $true   # 'info' — no single correct answer
}

function Format-Cell($code, $expect) {
  if ($expect -eq 'info') { return "$code" }
  if (Test-Match $code $expect) { return "$code OK" }
  return "$code XX"
}

$MISSING = '.\__future_nope_xyz__.txt'
$tests = @(
  [ordered]@{ Name = 'cmdlet success';    Cmd = 'Get-ChildItem .';                                          Expect = 0 }
  [ordered]@{ Name = 'cmdlet hard fail';  Cmd = "Get-Content $MISSING -ErrorAction Stop";                   Expect = 'non-0' }
  [ordered]@{ Name = 'native success';    Cmd = 'cmd /c exit 0';                                            Expect = 0 }
  [ordered]@{ Name = 'native exit 3';     Cmd = 'cmd /c exit 3';                                            Expect = 3 }
  [ordered]@{ Name = 'command not found'; Cmd = 'notarealcommand_xyz123';                                   Expect = 'non-0' }
  [ordered]@{ Name = 'handled try/catch'; Cmd = "try { Get-Content $MISSING -ErrorAction Stop } catch { 'handled' }"; Expect = 0 }
  [ordered]@{ Name = 'fail then succeed'; Cmd = "Write-Output 'a'; Get-Content $MISSING; Write-Output 'b'"; Expect = 0 }
  [ordered]@{ Name = 'partial 2 paths';   Cmd = "Get-ChildItem .\, $MISSING";                               Expect = 'info' }
  [ordered]@{ Name = 'Write-Error';       Cmd = "Write-Error 'boom'";                                       Expect = 'non-0' }
  [ordered]@{ Name = 'Chinese output';    Cmd = 'Write-Output ([char]0x4E2D + [char]0x6587)';               Expect = 0 }
)

$fmt = "{0,-20} {1,-8} {2,-16} {3,-16} {4,-16}"
Write-Host ""
Write-Host ($fmt -f 'case', 'expect', 'CURRENT', 'QSTREAM', 'QBUF')
Write-Host ('-' * 78)

$disagree = @()
foreach ($t in $tests) {
  $cur = Invoke-Variant $shell (Build-Cur     $t.Cmd)
  $qs  = Invoke-Variant $shell (Build-QStream $t.Cmd)
  $qb  = Invoke-Variant $shell (Build-QBuf    $t.Cmd)
  Write-Host ($fmt -f $t.Name, "$($t.Expect)", `
    (Format-Cell $cur.Code $t.Expect), `
    (Format-Cell $qs.Code  $t.Expect), `
    (Format-Cell $qb.Code  $t.Expect))
  if (($cur.Code -ne $qs.Code) -or ($cur.Code -ne $qb.Code)) { $disagree += $t.Name }
}

Write-Host ""
if ($disagree.Count) {
  Write-Host "Variants DISAGREE on: $($disagree -join ', ')"
  Write-Host "(These are the cases the fallback choice actually changes.)"
} else {
  Write-Host "All variants agree on every case."
}

# Output rendering + encoding preview for the telling rows.
Write-Host ""
Write-Host "=== stdout preview (error rendering + Chinese encoding) ==="
foreach ($name in @('handled try/catch', 'cmdlet hard fail', 'Chinese output')) {
  $t = $tests | Where-Object { $_.Name -eq $name } | Select-Object -First 1
  if (-not $t) { continue }
  $curOut = (Invoke-Variant $shell (Build-Cur     $t.Cmd)).Out -replace "`r?`n", ' | '
  $qsOut  = (Invoke-Variant $shell (Build-QStream $t.Cmd)).Out -replace "`r?`n", ' | '
  Write-Host ""
  Write-Host "[$name]"
  Write-Host "  CURRENT: $curOut"
  Write-Host "  QSTREAM: $qsOut"
}
# Authoritative encoding check: capture the child's stdout as RAW BYTES (exactly
# what the Rust agent reads before from_utf8_lossy), independent of any console
# encoding. If the bytes are the UTF-8 of 中文, production renders it correctly.
Write-Host ""
Write-Host "=== raw-byte encoding check (matches Rust from_utf8_lossy) ==="
$tmp = [System.IO.Path]::GetTempFileName()
try {
  $script = Build-Cur 'Write-Output ([char]0x4E2D + [char]0x6587)'   # 中文
  $enc = [Convert]::ToBase64String([System.Text.Encoding]::Unicode.GetBytes($script))
  Start-Process $shell `
    -ArgumentList @('-NoProfile', '-NonInteractive', '-NoLogo', '-EncodedCommand', $enc) `
    -RedirectStandardOutput $tmp -NoNewWindow -Wait | Out-Null
  $raw = [System.IO.File]::ReadAllBytes($tmp)
  $hex = ($raw | ForEach-Object { $_.ToString('X2') }) -join ' '
  $want = [System.Text.Encoding]::UTF8.GetBytes([string]([char]0x4E2D + [char]0x6587))
  $wantHex = ($want | ForEach-Object { $_.ToString('X2') }) -join ' '
  $gbkHex = 'D6 D0 CE C4'   # 中文 in GBK/CP936, for reference if it's wrong
  Write-Host "  child stdout bytes : $hex"
  Write-Host "  want (UTF-8 of 中文): $wantHex"
  if ($hex -like "*$wantHex*") {
    Write-Host "  VERDICT: OK — wrapper emits UTF-8; production renders 中文 correctly."
  } elseif ($hex -like "*$gbkHex*") {
    Write-Host "  VERDICT: BROKEN — wrapper emitted GBK ($gbkHex), not UTF-8. Encoding fix is not taking effect."
  } else {
    Write-Host "  VERDICT: UNEXPECTED — neither UTF-8 nor GBK; inspect the bytes above."
  }
}
finally {
  Remove-Item $tmp -ErrorAction SilentlyContinue
}

Write-Host ""
Write-Host "Done. Send me this whole output and I'll decide the fallback + which variant to ship."
