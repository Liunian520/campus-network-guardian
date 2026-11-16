[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'inno-compiler.ps1')

# Output shape comes from the official Inno Setup 6.7.4 ISCC.dpr:
# c2913c993858a781fcb455bac8b8949c1c366cb8, ProcessCommandLine / Go.
# /? only prints the major-version banner; compilation prints the engine line.
$officialStyleOutput = @'
Inno Setup 6 Command-Line Compiler
Copyright (C) 1997-2026 Jordan Russell. All rights reserved.
https://www.innosetup.com

Compiler engine version: Inno Setup 6.7.4
Successful compile (0.012 sec). Output was disabled.
'@
$detected = ConvertFrom-InnoCompilerOutput -Output $officialStyleOutput
if ($detected -ne [version] '6.7.4') {
    throw "Expected the compiler engine version 6.7.4, got $detected."
}

foreach ($sample in @(
    @{ Text = 'Compiler engine version: Inno Setup 6.5.0'; Supported = $true },
    @{ Text = 'Compiler engine version: Inno Setup 6.7.4.0'; Supported = $true },
    @{ Text = 'Compiler engine version: Inno Setup 6.4.3'; Supported = $false },
    @{ Text = 'Compiler engine version: Inno Setup 7.0.0'; Supported = $false }
)) {
    $version = ConvertFrom-InnoCompilerOutput -Output $sample.Text
    if ((Test-InnoCompilerVersion -Version $version) -ne $sample.Supported) {
        throw "Incorrect compiler support decision for $version."
    }
}
foreach ($invalidOutput in @(
    'Inno Setup 6 Command-Line Compiler',
    'FileVersion: 0.0.0.0',
    'Compiler engine version: Inno Setup 6.7.4-beta',
    'Unrelated application 6.7.4'
)) {
    $rejected = $false
    try { $null = ConvertFrom-InnoCompilerOutput -Output $invalidOutput }
    catch { $rejected = $true }
    if (-not $rejected) { throw "Unrecognized output was incorrectly accepted: $invalidOutput" }
}

# Exercise stdin/output-disable and exit-code handling with a PowerShell fake.
# This invokes no native executable and cannot produce an installer.
function Invoke-FakeInnoCompiler {
    param([string] $OutputOption, [string] $SourceOption)
    $probe = $input | Out-String
    if ($OutputOption -cne '/O-' -or $SourceOption -cne '-') {
        throw 'Compiler detection must disable output and read only the in-memory probe.'
    }
    if ($probe -notmatch '(?m)^Output=no\r?$' -or
        $probe -notmatch '(?m)^Uninstallable=no\r?$' -or
        $probe -match '(?im)^\[(Files|Registry|Run|Icons)\]') {
        throw 'Compiler detection did not use the output-disabled, empty probe.'
    }
    $global:LASTEXITCODE = 0
    Write-Output $officialStyleOutput
}
$probedVersion = Get-InnoCompilerVersion -CompilerPath 'Invoke-FakeInnoCompiler'
if ($probedVersion -ne [version] '6.7.4') { throw 'The command probe did not return the actual engine version.' }

function Invoke-FailedFakeInnoCompiler {
    $null = $input | Out-String
    $global:LASTEXITCODE = 2
    Write-Output $officialStyleOutput
}
$failedProbeRejected = $false
try { $null = Get-InnoCompilerVersion -CompilerPath 'Invoke-FailedFakeInnoCompiler' }
catch { $failedProbeRejected = $true }
if (-not $failedProbeRejected) { throw 'A failed compiler probe was accepted just because it printed a version.' }

$global:LASTEXITCODE = 0
Write-Host 'Inno compiler detection tests passed; no compiler or installer was executed.'
