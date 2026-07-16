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
# The row that decides it is "handled try/catch": CURRENT should show XX (a false
# failure), QSTREAM/QBUF should show OK. If they do, $? is the better fallback —
# then choose QSTREAM (keep streaming) or QBUF (keep clean output) from the
# stdout preview at the bottom.
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
Write-Host ""
Write-Host "Windows shell exit-code strategy comparison"
Write-Host "Shell: $shell  (v$ver)"

# Shared prologue (literal here-string — the $ stay literal in the wrapper).
$prologue = @'
chcp 65001 > $null
$OutputEncoding = [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
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
Write-Host ""
Write-Host "Done. Send me this whole output and I'll decide the fallback + which variant to ship."
