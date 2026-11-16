# Pure version-resource validation shared by packaging and hosted smoke tests.
# Dot-sourcing this file does not inspect or execute any binary.
function Test-WindowsBinaryVersion {
    param(
        [Parameter(Mandatory = $true)] [object] $VersionInfo,
        [Parameter(Mandatory = $true)] [string] $ExpectedVersion
    )
    $expected = [regex]::Match($ExpectedVersion, '^(\d+)\.(\d+)\.(\d+)$')
    if (-not $expected.Success) { return $false }
    $pattern = '^' + [regex]::Escape($ExpectedVersion) + '(?:\.0)?$'
    # Inno 6 updates the leading text in fixed-width space placeholders.
    # Trim cosmetic padding, but still require exact text and both fixed quads.
    $productText = ([string] $VersionInfo.ProductVersion).Trim()
    $fileText = ([string] $VersionInfo.FileVersion).Trim()
    if ($productText -notmatch $pattern -or $fileText -notmatch $pattern) {
        return $false
    }
    $expectedParts = @([int] $expected.Groups[1].Value, [int] $expected.Groups[2].Value, [int] $expected.Groups[3].Value, 0)
    $fileParts = @($VersionInfo.FileMajorPart, $VersionInfo.FileMinorPart, $VersionInfo.FileBuildPart, $VersionInfo.FilePrivatePart)
    $productParts = @($VersionInfo.ProductMajorPart, $VersionInfo.ProductMinorPart, $VersionInfo.ProductBuildPart, $VersionInfo.ProductPrivatePart)
    for ($part = 0; $part -lt 4; $part++) {
        if ($fileParts[$part] -ne $expectedParts[$part] -or $productParts[$part] -ne $expectedParts[$part]) {
            return $false
        }
    }
    return $true
}
