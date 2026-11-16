$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'windows-binary-version.ps1')

function New-TestVersionInfo {
    param([string] $File = '0.4.0.0', [string] $Product = '0.4.0.0')
    [pscustomobject]@{
        FileVersion = $File
        ProductVersion = $Product
        FileMajorPart = 0
        FileMinorPart = 4
        FileBuildPart = 0
        FilePrivatePart = 0
        ProductMajorPart = 0
        ProductMinorPart = 4
        ProductBuildPart = 0
        ProductPrivatePart = 0
    }
}

function Assert-VersionResult {
    param([object] $Info, [bool] $Expected, [string] $Name)
    $actual = Test-WindowsBinaryVersion -VersionInfo $Info -ExpectedVersion '0.4.0'
    if ($actual -ne $Expected) { throw "Version validation regression: $Name." }
}

Assert-VersionResult -Info (New-TestVersionInfo) -Expected $true -Name 'ordinary four-part resource'
Assert-VersionResult -Info (New-TestVersionInfo -File '0.4.0' -Product '0.4.0') -Expected $true -Name 'three-part textual resource'
Assert-VersionResult -Info (New-TestVersionInfo -File ('0.4.0.0'.PadRight(20)) -Product ('0.4.0.0'.PadRight(50))) -Expected $true -Name 'official Inno Setup space-padded text'
Assert-VersionResult -Info (New-TestVersionInfo -File '0.4.1.0') -Expected $false -Name 'wrong textual file version'
Assert-VersionResult -Info (New-TestVersionInfo -Product '0.4.0.0 unexpected') -Expected $false -Name 'unexpected product text suffix'
Assert-VersionResult -Info (New-TestVersionInfo -File '') -Expected $false -Name 'missing file text'
$wrongFile = New-TestVersionInfo
$wrongFile.FileBuildPart = 1
Assert-VersionResult -Info $wrongFile -Expected $false -Name 'matching text with wrong fixed file version'
$wrongProduct = New-TestVersionInfo
$wrongProduct.ProductPrivatePart = 1
Assert-VersionResult -Info $wrongProduct -Expected $false -Name 'matching text with wrong fixed product revision'
$zeroFixedVersion = New-TestVersionInfo
$zeroFixedVersion.FileMinorPart = 0
$zeroFixedVersion.ProductMinorPart = 0
Assert-VersionResult -Info $zeroFixedVersion -Expected $false -Name 'unversioned fixed resource with matching text'
Write-Host 'Windows binary version sample regressions passed; no binary was inspected or executed.'
