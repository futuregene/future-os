# Pure assertion regressions: load only the production functions via AST.
# Do not run the lifecycle script's process, HOME, lock or capability operations.
# Run in Windows CI with Windows PowerShell 5.1; no Pester dependency.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$source = Join-Path $PSScriptRoot 'test-windows-sandbox-lifecycle.ps1'
$tokens = $null
$parseErrors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile($source, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw ($parseErrors | Out-String) }
foreach ($name in @('Get-ExpectationErrors', 'Test-AbsolutePath')) {
    $functionAst = $ast.Find({ param($node)
        $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq $name
    }, $false)
    if ($null -eq $functionAst) { throw "Missing production function: $name" }
    . ([scriptblock]::Create($functionAst.Extent.Text))
}

function New-State {
    $app = [pscustomobject]@{ ProcessId = 101 }
    return [pscustomobject]@{
        Apps = @($app); Agents = @(); BundledAgents = @(); ExternalAgents = @()
        Owner = $app; LockHeld = $true; MetadataExists = $true; CapabilityRecords = 0
    }
}
function Assert-Result([string]$Action, $State, [bool]$Pass) {
    $errors = @(Get-ExpectationErrors $Action $State)
    if (($errors.Count -eq 0) -ne $Pass) {
        throw "Wrong ${Action} result: pass=$Pass; errors=$($errors -join '; ')"
    }
}

foreach ($action in @('ExpectEmbedded', 'ExpectBundled', 'ExpectRecovered')) {
    Assert-Result $action (New-State) $true
    $state = New-State
    $state.LockHeld = $false
    Assert-Result $action $state $false
    $state = New-State
    $state.Owner = $null
    Assert-Result $action $state $false
    $state = New-State
    $state.Owner = [pscustomobject]@{ ProcessId = 999 }
    Assert-Result $action $state $false
    $state = New-State
    $state.Agents = @([pscustomobject]@{ ProcessId = 102 })
    $state.BundledAgents = $state.Agents
    Assert-Result $action $state $false
}
$state = New-State
$state.CapabilityRecords = 1
Assert-Result 'ExpectRecovered' $state $false

foreach ($action in @('ExpectClean', 'ExpectStopped')) {
    $state = New-State
    $state.Apps = @(); $state.Owner = $null
    $state.LockHeld = $false; $state.MetadataExists = $false
    Assert-Result $action $state $true
    $state.LockHeld = $true
    Assert-Result $action $state $false
    $state.LockHeld = $false; $state.MetadataExists = $true
    Assert-Result $action $state $false
    $state.MetadataExists = $false; $state.CapabilityRecords = 1
    Assert-Result $action $state $false
}
foreach ($action in @('ExpectExternalAttached', 'ExpectExternalSurvives')) {
    $state = New-State
    $external = [pscustomobject]@{ ProcessId = 202 }
    $state.ExternalAgents = @($external); $state.Owner = $external
    if ($action -eq 'ExpectExternalSurvives') { $state.Apps = @(); $state.CapabilityRecords = 1 }
    Assert-Result $action $state $true
    $state.LockHeld = $false
    Assert-Result $action $state $false
    $state.LockHeld = $true; $state.Owner = $null
    Assert-Result $action $state $false
    $state.Owner = [pscustomobject]@{ ProcessId = 101 }
    Assert-Result $action $state $false
}
foreach ($path in @('C:relative', '\root-only', '', 'relative')) {
    if (Test-AbsolutePath $path) { throw "Unsafe absolute path accepted: $path" }
}
foreach ($path in @('C:\state', '\\server\share\state')) {
    if (-not (Test-AbsolutePath $path)) { throw "Absolute path rejected: $path" }
}
Write-Host 'Packaged lifecycle assertion regressions passed.'
