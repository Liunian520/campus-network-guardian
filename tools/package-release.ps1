[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^\d+\.\d+\.\d+$')]
    [string] $ReleaseVersion
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem
. (Join-Path $PSScriptRoot 'windows-binary-version.ps1')

$repositoryRoot = Split-Path -Parent $PSScriptRoot
$outputDirectory = Join-Path $repositoryRoot 'dist'
$assetPrefix = "campus-network-guardian-$ReleaseVersion-windows-x64"
$nativeExe = Get-Item -LiteralPath (Join-Path $repositoryRoot 'target/installer-build/x86_64-pc-windows-msvc/release/campus-network-guardian.exe')
$setupExe = Get-Item -LiteralPath (Join-Path $outputDirectory "$assetPrefix-setup.exe")
foreach ($binary in @($nativeExe, $setupExe)) {
    if (-not (Test-WindowsBinaryVersion -VersionInfo $binary.VersionInfo -ExpectedVersion $ReleaseVersion)) {
        throw "The embedded version in $($binary.Name) does not match release $ReleaseVersion."
    }
}

$utf8 = [System.Text.UTF8Encoding]::new($false, $true)
$setupZip = Join-Path $outputDirectory "$assetPrefix-setup.zip"
$instructionsName = '安装说明.txt'
$instructions = @"
校园网守护 v$ReleaseVersion

适用系统为 Windows 10 1607 及更新版本、Windows 11（x64）。
1. 完整解压本 ZIP，再运行 $($setupExe.Name)。
2. 安装使用当前 Windows 用户权限，可选择登录 Windows 后自动启动。
3. 首次运行时选择校园网络，并在应用中完成认证设置。关闭主窗口后，程序继续在托盘守护。
4. 已安装旧版时，安装程序会沿用已有配置。

项目主页 https://github.com/Liunian520/campus-network-guardian
"@

# Windows PowerShell's ZIP implementation recognizes the standard Encoding.UTF8
# instance when setting the UTF-8 filename flag for the Chinese entry name.
$setupArchive = [System.IO.Compression.ZipArchive]::new(
    [System.IO.File]::Create($setupZip), [System.IO.Compression.ZipArchiveMode]::Create, $false, [System.Text.Encoding]::UTF8
)
try {
    [void] [System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile($setupArchive, $setupExe.FullName, $setupExe.Name)
    $instructionsEntry = $setupArchive.CreateEntry($instructionsName)
    $writer = [System.IO.StreamWriter]::new($instructionsEntry.Open(), $utf8)
    try {
        $writer.WriteLine($instructions)
    } finally {
        $writer.Dispose()
    }
} finally {
    $setupArchive.Dispose()
}

$setupHash = (Get-FileHash -LiteralPath $setupExe.FullName -Algorithm SHA256).Hash
$setupArchive = [System.IO.Compression.ZipFile]::OpenRead($setupZip)
try {
    $zipInstaller = $setupArchive.GetEntry($setupExe.Name)
    $zipInstructions = $setupArchive.GetEntry($instructionsName)
    if ($setupArchive.Entries.Count -ne 2 -or $null -eq $zipInstaller -or $null -eq $zipInstructions) {
        throw 'The installer ZIP must contain the installer and installation instructions at its root.'
    }
    $installerStream = $zipInstaller.Open()
    try {
        $zipInstallerHash = (Get-FileHash -InputStream $installerStream -Algorithm SHA256).Hash
    } finally {
        $installerStream.Dispose()
    }
    if ($zipInstaller.Length -ne $setupExe.Length -or $zipInstallerHash -cne $setupHash) {
        throw 'The installer ZIP does not contain the same installer as the standalone EXE.'
    }
    $reader = [System.IO.StreamReader]::new($zipInstructions.Open(), $utf8, $false)
    try {
        if ($reader.ReadToEnd() -cne ($instructions + [Environment]::NewLine)) {
            throw 'The installer ZIP instructions did not round-trip as UTF-8.'
        }
    } finally {
        $reader.Dispose()
    }
} finally {
    $setupArchive.Dispose()
}

$portableZip = Join-Path $outputDirectory "$assetPrefix-portable.zip"
$portableFiles = @($nativeExe.FullName) + (@('README.md', 'LICENSE', 'THIRD-PARTY-NOTICES.md') | ForEach-Object {
    Join-Path $repositoryRoot $_
})
Compress-Archive -LiteralPath $portableFiles -DestinationPath $portableZip -Force

$checksums = foreach ($asset in @($setupExe.FullName, $setupZip, $portableZip)) {
    $hash = (Get-FileHash -LiteralPath $asset -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $([System.IO.Path]::GetFileName($asset))"
}
Set-Content -LiteralPath (Join-Path $outputDirectory 'SHA256SUMS.txt') -Value $checksums -Encoding ascii
