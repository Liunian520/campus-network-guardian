# Shared Inno Setup compiler detection. Dot-source this file; loading it does
# not invoke a compiler or modify any files, processes, or registry settings.

function ConvertFrom-InnoCompilerOutput {
    [CmdletBinding()]
    param([Parameter(Mandatory = $true)][string] $Output)

    $engineVersion = [regex]::Match(
        $Output,
        '(?im)^\s*Compiler engine version:\s+Inno Setup\s+(\d+\.\d+\.\d+(?:\.\d+)?)(?:\s|$)'
    )
    if (-not $engineVersion.Success) {
        throw 'ISCC did not report a recognizable compiler engine version.'
    }
    return [version] $engineVersion.Groups[1].Value
}

function Test-InnoCompilerVersion {
    [CmdletBinding()]
    param([Parameter(Mandatory = $true)][version] $Version)

    return $Version.Major -eq 6 -and $Version.Minor -ge 5
}

function Get-InnoCompilerVersion {
    [CmdletBinding()]
    param([Parameter(Mandatory = $true)][string] $CompilerPath)

    # ISCC.exe does not guarantee PE version metadata. Its /? output also omits
    # the engine version and intentionally exits 1. Compile an in-memory, empty
    # script with output disabled instead: this loads the actual compiler DLL
    # and prints its engine version without producing or running an installer.
    $versionProbe = @'
[Setup]
AppId=CampusNetworkGuardian.CompilerVersionProbe
AppName=Compiler version probe
AppVersion=1.0.0
DefaultDirName={tmp}\CampusNetworkGuardianCompilerProbe
CreateAppDir=no
Uninstallable=no
PrivilegesRequired=lowest
Output=no
'@
    $compilerOutput = $versionProbe | & $CompilerPath '/O-' '-' 2>&1 | Out-String
    $compilerExitCode = $LASTEXITCODE
    if ($compilerExitCode -ne 0) {
        throw "ISCC compiler probe failed with exit code ${compilerExitCode}:`n$compilerOutput"
    }
    return ConvertFrom-InnoCompilerOutput -Output $compilerOutput
}
