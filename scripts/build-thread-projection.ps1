#requires -Version 5.1
<#
.SYNOPSIS
    Build the shared @future-os/thread-projection package when its compiled dist/
    is missing or older than the TypeScript sources.
.DESCRIPTION
    desktop/ and mobile/ both consume thread-projection through a `file:`
    dependency, so it must be up to date before either app builds, typechecks or
    starts. Every build-*/start-* script calls this so no one has to remember to
    rebuild it by hand.
#>
Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot
$tp = Join-Path $Root "thread-projection"
# thread-projection is a root-workspace member, so npm hoists its node_modules
# (and the install stamp) to the repo root — not next to the package.
$stamp = Join-Path $Root "node_modules\.package-lock.json"
$pkg = Join-Path $tp "package.json"
$dist = Join-Path $tp "dist\index.js"

if (-not (Test-Path $stamp) -or
    (Get-Item $pkg).LastWriteTimeUtc -gt (Get-Item $stamp).LastWriteTimeUtc) {
    Write-Host "  npm install thread-projection/"
    Push-Location $tp
    try {
        npm install
        if ($LASTEXITCODE -ne 0) { throw "npm install failed in thread-projection" }
    }
    finally { Pop-Location }
}

$needsBuild = -not (Test-Path $dist)
if (-not $needsBuild) {
    $distTime = (Get-Item $dist).LastWriteTimeUtc
    foreach ($f in Get-ChildItem (Join-Path $tp "src") -Recurse -File) {
        if ($f.LastWriteTimeUtc -gt $distTime) { $needsBuild = $true; break }
    }
}
if ($needsBuild) {
    Write-Host "  build thread-projection/"
    Push-Location $tp
    try {
        npm run build
        if ($LASTEXITCODE -ne 0) { throw "npm run build failed in thread-projection" }
    }
    finally { Pop-Location }
}
