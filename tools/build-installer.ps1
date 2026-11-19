[CmdletBinding()]
param(
    [Parameter(Mandatory = $false)]
    [string] $IsccPath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'inno-compiler.ps1')
. (Join-Path $PSScriptRoot 'windows-binary-version.ps1')

$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$manifestPath = Join-Path $repositoryRoot 'Cargo.toml'
$installerScript = Join-Path $repositoryRoot 'installer\campus-network-guardian.iss'
$buildDirectory = Join-Path $repositoryRoot 'target\installer-build'
$outputDirectory = Join-Path $repositoryRoot 'dist'
$rustTarget = 'x86_64-pc-windows-msvc'

function Resolve-InnoCompiler {
    param([string] $RequestedPath)

    if (-not [string]::IsNullOrWhiteSpace($RequestedPath)) {
        $resolvedCompiler = (Resolve-Path -LiteralPath $RequestedPath).Path
        if (-not (Test-Path -LiteralPath $resolvedCompiler -PathType Leaf)) {
            throw "ISCC compiler not found: $RequestedPath"
        }
        return $resolvedCompiler
    }

    $compilerCommand = Get-Command 'ISCC.exe' -ErrorAction SilentlyContinue
    if ($null -ne $compilerCommand) {
        return $compilerCommand.Source
    }

    $candidateRoots = @(
        [System.Environment]::GetFolderPath('ProgramFilesX86'),
        [System.Environment]::GetFolderPath('ProgramFiles'),
        [System.Environment]::GetFolderPath('LocalApplicationData')
    )
    foreach ($candidateRoot in $candidateRoots) {
        if ([string]::IsNullOrWhiteSpace($candidateRoot)) { continue }
        $relativeCompiler = if ($candidateRoot -eq [System.Environment]::GetFolderPath('LocalApplicationData')) {
            'Programs\Inno Setup 6\ISCC.exe'
        } else {
            'Inno Setup 6\ISCC.exe'
        }
        $candidateCompiler = Join-Path $candidateRoot $relativeCompiler
        if (Test-Path -LiteralPath $candidateCompiler -PathType Leaf) {
            return $candidateCompiler
        }
    }
    throw 'Inno Setup 6.5 or newer is required. Install it from https://jrsoftware.org/isinfo.php or pass -IsccPath C:\path\to\ISCC.exe. This script does not download or install build tools.'
}

$compilerPath = Resolve-InnoCompiler -RequestedPath $IsccPath
$compilerVersion = Get-InnoCompilerVersion -CompilerPath $compilerPath
if (-not (Test-InnoCompilerVersion -Version $compilerVersion)) {
    throw "Inno Setup 6.5 or newer in the 6.x series is required; found compiler engine $compilerVersion."
}
$manifestText = Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8
$packageSection = [regex]::Match($manifestText, '(?ms)^\[package\]\s*(.*?)(?=^\[|\z)')
$versionMatch = [regex]::Match($packageSection.Groups[1].Value, '(?m)^\s*version\s*=\s*"(\d+\.\d+\.\d+)"\s*$')
if (-not $versionMatch.Success) {
    throw 'Cargo.toml must contain a three-part release version in [package].'
}
$applicationVersion = $versionMatch.Groups[1].Value
$setupBaseName = "campus-network-guardian-$applicationVersion-windows-x64-setup"
$setupPath = Join-Path $outputDirectory "$setupBaseName.exe"

if (-not (Get-Command 'cargo.exe' -ErrorAction SilentlyContinue)) {
    throw 'Rust cargo.exe is required to build the Windows application.'
}
if (-not (Test-Path -LiteralPath (Join-Path $repositoryRoot 'LICENSE') -PathType Leaf)) {
    throw 'The open-source LICENSE file is missing.'
}
New-Item -ItemType Directory -Path $outputDirectory -Force | Out-Null

Push-Location -LiteralPath $repositoryRoot
try {
    & cargo.exe build --release --locked --bin campus-network-guardian --target $rustTarget --target-dir $buildDirectory
    if ($LASTEXITCODE -ne 0) {
        throw "Rust build failed with exit code $LASTEXITCODE."
    }
    $applicationPath = Join-Path $buildDirectory "$rustTarget\release\campus-network-guardian.exe"
    if (-not (Test-Path -LiteralPath $applicationPath -PathType Leaf)) {
        throw "Rust build did not produce $applicationPath."
    }
    $binaryVersion = [System.Diagnostics.FileVersionInfo]::GetVersionInfo($applicationPath)
    if (-not (Test-WindowsBinaryVersion -VersionInfo $binaryVersion -ExpectedVersion $applicationVersion)) {
        throw "Windows executable version '$($binaryVersion.ProductVersion)' does not match Cargo $applicationVersion."
    }

    $compilerArguments = @(
        '/Qp',
        "/DAppVersion=$applicationVersion",
        "/DRepoRoot=$repositoryRoot",
        "/DSourceExe=$applicationPath",
        "/DSetupOutputDir=$outputDirectory",
        "/F$setupBaseName",
        $installerScript
    )
    & $compilerPath @compilerArguments
    if ($LASTEXITCODE -ne 0) {
        throw "Inno Setup compilation failed with exit code $LASTEXITCODE."
    }
    if (-not (Test-Path -LiteralPath $setupPath -PathType Leaf)) {
        throw "Inno Setup did not produce $setupPath."
    }
    Write-Host "Installer built: $setupPath"
    Get-Item -LiteralPath $setupPath
} finally {
    Pop-Location
}
