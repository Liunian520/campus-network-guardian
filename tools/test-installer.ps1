[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string] $SetupPath,
    [Parameter(Mandatory = $true)]
    [string] $ExpectedVersion
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

# This script installs and uninstalls the app in a disposable hosted runner.
# It must never be used to test against a person's Windows session.
if ($env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_OS -cne 'Windows' -or $env:RUNNER_ENVIRONMENT -cne 'github-hosted') {
    throw 'Installer smoke tests are restricted to GitHub-hosted Windows Actions runners.'
}
if ([string]::IsNullOrWhiteSpace($env:RUNNER_TEMP) -or $ExpectedVersion -notmatch '^\d+\.\d+\.\d+$') {
    throw 'A runner temporary directory and a stable three-part version are required.'
}
. (Join-Path $PSScriptRoot 'windows-binary-version.ps1')

function Assert-Condition {
    param([bool] $Condition, [string] $Message)
    if (-not $Condition) { throw $Message }
}

function Assert-ChildPath {
    param([string] $Path, [string] $Parent)
    $resolvedPath = [System.IO.Path]::GetFullPath($Path)
    $parentPrefix = [System.IO.Path]::GetFullPath($Parent).TrimEnd([char[]] '\/') + [System.IO.Path]::DirectorySeparatorChar
    Assert-Condition -Condition $resolvedPath.StartsWith($parentPrefix, [StringComparison]::OrdinalIgnoreCase) -Message 'The test path is outside the intended runner temporary directory.'
}

$setupExecutable = (Get-Item -LiteralPath $SetupPath).FullName
$setupVersion = (Get-Item -LiteralPath $setupExecutable).VersionInfo
Write-Host ("Installer version diagnostic: expected={0}; ProductVersion=<{1}>; FileVersion=<{2}>; ProductParts={3}.{4}.{5}.{6}; FileParts={7}.{8}.{9}.{10}; ProductTextLength={11}; FileTextLength={12}" -f
    $ExpectedVersion, $setupVersion.ProductVersion, $setupVersion.FileVersion,
    $setupVersion.ProductMajorPart, $setupVersion.ProductMinorPart, $setupVersion.ProductBuildPart, $setupVersion.ProductPrivatePart,
    $setupVersion.FileMajorPart, $setupVersion.FileMinorPart, $setupVersion.FileBuildPart, $setupVersion.FilePrivatePart,
    ([string] $setupVersion.ProductVersion).Length, ([string] $setupVersion.FileVersion).Length)
Assert-Condition -Condition (Test-WindowsBinaryVersion -VersionInfo $setupVersion -ExpectedVersion $ExpectedVersion) -Message 'The installer version does not match the requested version.'

$runKeyPath = 'Software\Microsoft\Windows\CurrentVersion\Run'
$runValueName = 'CampusNetworkGuardian'
$uninstallKeyPath = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\CampusNetworkGuardian.Windows_is1'
$runnerTemporaryDirectory = [System.IO.Path]::GetFullPath($env:RUNNER_TEMP)
$smokeDirectory = Join-Path $runnerTemporaryDirectory ('guardian-installer-smoke-' + [guid]::NewGuid().ToString('N'))
$applicationDirectory = Join-Path $smokeDirectory 'Installed App With Spaces'
$applicationExecutable = Join-Path $applicationDirectory 'campus-network-guardian.exe'
$ownRunCommand = '"' + $applicationExecutable + '"'
$foreignRunCommand = '"' + (Join-Path $smokeDirectory 'Another Install\campus-network-guardian.exe') + '"'
$configurationDirectory = Join-Path ([Environment]::GetFolderPath('ApplicationData')) 'CampusNetworkGuardian'
$configurationPath = Join-Path $configurationDirectory 'config.json'
$shortcutDirectory = Join-Path ([Environment]::GetFolderPath('Programs')) '校园网守护'
$shortcutPath = Join-Path $shortcutDirectory '校园网守护.lnk'
$configurationCreated = $false
$configurationHash = ''
$diagnosticDirectory = Join-Path ([System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))) 'dist\installer-smoke-logs'
$testApplicationProcess = $null
$smokeSucceeded = $false
$cleanupFailures = [System.Collections.Generic.List[string]]::new()

function Read-RunCommand {
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($runKeyPath, $false)
    if ($null -eq $key) { return $null }
    try {
        return $key.GetValue($runValueName, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
    } finally {
        $key.Dispose()
    }
}

function Write-RunCommand {
    param([string] $Command)
    $key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($runKeyPath, $true)
    try {
        $key.SetValue($runValueName, $Command, [Microsoft.Win32.RegistryValueKind]::String)
    } finally {
        $key.Dispose()
    }
}

function Remove-TestRunCommand {
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($runKeyPath, $true)
    if ($null -eq $key) { return }
    try {
        $current = $key.GetValue($runValueName, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
        if ($current -ceq $ownRunCommand -or $current -ceq $foreignRunCommand) {
            $key.DeleteValue($runValueName, $false)
        }
    } finally {
        $key.Dispose()
    }
}

function Assert-NoApplicationProcess {
    $processes = @(Get-Process -Name 'campus-network-guardian' -ErrorAction SilentlyContinue)
    Assert-Condition -Condition ($processes.Count -eq 0) -Message 'The application is running; silent installation must not launch it.'
}

function Save-TestProcessDiagnostics {
    param(
        [string] $Stage, [string] $Executable, [string[]] $Arguments,
        [string] $LogPath, [object] $ExitCode, [string] $Failure,
        [DateTime] $StartedAt
    )
    Assert-Condition -Condition ($Stage -match '^[a-z0-9-]+$') -Message 'The diagnostic stage name is invalid.'
    Assert-ChildPath -Path $LogPath -Parent $smokeDirectory
    New-Item -ItemType Directory -Path $diagnosticDirectory -Force | Out-Null
    $logExists = Test-Path -LiteralPath $LogPath -PathType Leaf
    $details = [ordered]@{
        Stage = $Stage
        Executable = $Executable
        Arguments = $Arguments
        ExitCode = $ExitCode
        Failure = $Failure
        StartedAtUtc = $StartedAt.ToUniversalTime().ToString('o')
        FinishedAtUtc = [DateTime]::UtcNow.ToString('o')
        WindowsVersion = [Environment]::OSVersion.VersionString
        Is64BitOperatingSystem = [Environment]::Is64BitOperatingSystem
        Is64BitProcess = [Environment]::Is64BitProcess
        UICulture = [Globalization.CultureInfo]::CurrentUICulture.Name
        Culture = [Globalization.CultureInfo]::CurrentCulture.Name
        PowerShellVersion = $PSVersionTable.PSVersion.ToString()
        LogPath = $LogPath
        LogExists = $logExists
    }
    $json = $details | ConvertTo-Json -Depth 4
    Set-Content -LiteralPath (Join-Path $diagnosticDirectory "$Stage-process.json") -Value $json -Encoding utf8
    if ($logExists) {
        Copy-Item -LiteralPath $LogPath -Destination (Join-Path $diagnosticDirectory "$Stage.log") -Force
    }
    if ($null -eq $ExitCode -or $ExitCode -ne 0 -or -not [string]::IsNullOrEmpty($Failure)) {
        Write-Host "Installer smoke process diagnostic for stage $Stage`:"
        Write-Host $json
        if ($logExists) {
            Write-Host "Last 100 lines of the stage's Inno log (only test data and runner paths):"
            Get-Content -LiteralPath $LogPath -Tail 100 | ForEach-Object { Write-Host ("  {0}" -f $_) }
        } else {
            Write-Host 'No requested Inno log was created. Failure may have occurred before logging initialized; the exact command and runner environment are recorded above.'
        }
    }
}

function Invoke-TestProcess {
    param([string] $Executable, [string[]] $Arguments, [string] $Stage, [string] $LogPath)
    # Inno's first phase can exit while a temporary second phase is still
    # uninstalling. -Wait tracks the process tree; the workflow step supplies
    # the overall five-minute timeout.
    $process = $null
    $failure = ''
    $startedAt = [DateTime]::UtcNow
    try {
        $process = Start-Process -FilePath $Executable -ArgumentList $Arguments -WindowStyle Hidden -PassThru -Wait
        if ($process.ExitCode -ne 0) {
            throw "Installer or uninstaller stage '$Stage' returned exit code $($process.ExitCode)."
        }
    } catch {
        $failure = $_.Exception.Message
        throw
    } finally {
        $exitCode = if ($null -ne $process) { $process.ExitCode } else { $null }
        try {
            Save-TestProcessDiagnostics -Stage $Stage -Executable $Executable -Arguments $Arguments -LogPath $LogPath -ExitCode $exitCode -Failure $failure -StartedAt $startedAt
        } catch {
            Write-Warning ("Could not save installer diagnostics: {0}" -f $_.Exception.Message)
        }
        if ($null -ne $process) { $process.Dispose() }
    }
    Start-Sleep -Milliseconds 250
    Assert-NoApplicationProcess
}

Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;

namespace CampusGuardianInstallerSmoke {
    public static class Native {
        [ComImport, Guid("000214F9-0000-0000-C000-000000000046"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
        private interface IShellLinkW {
            // Preserve the native vtable order through GetArguments. The setter
            // declarations are necessary slots, but this reader never calls them.
            [PreserveSig] int GetPath([Out, MarshalAs(UnmanagedType.LPWStr)] StringBuilder path, int capacity, IntPtr findData, uint flags);
            [PreserveSig] int GetIDList(out IntPtr itemList);
            [PreserveSig] int SetIDList(IntPtr itemList);
            [PreserveSig] int GetDescription([Out, MarshalAs(UnmanagedType.LPWStr)] StringBuilder description, int capacity);
            [PreserveSig] int SetDescription([MarshalAs(UnmanagedType.LPWStr)] string description);
            [PreserveSig] int GetWorkingDirectory([Out, MarshalAs(UnmanagedType.LPWStr)] StringBuilder directory, int capacity);
            [PreserveSig] int SetWorkingDirectory([MarshalAs(UnmanagedType.LPWStr)] string directory);
            [PreserveSig] int GetArguments([Out, MarshalAs(UnmanagedType.LPWStr)] StringBuilder arguments, int capacity);
        }
        [ComImport, Guid("0000010B-0000-0000-C000-000000000046"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
        private interface IPersistFileReader {
            [PreserveSig] int GetClassID(out Guid classId);
            [PreserveSig] int IsDirty();
            [PreserveSig] int Load([MarshalAs(UnmanagedType.LPWStr)] string fileName, uint mode);
        }
        private static void RequireSuccess(int result, string operation) {
            if (result != 0) {
                throw new COMException(operation + " returned HRESULT 0x" + result.ToString("X8") + ".", result);
            }
        }
        public static string[] ReadShortcut(string path) {
            object instance = Activator.CreateInstance(Type.GetTypeFromCLSID(
                new Guid("00021401-0000-0000-C000-000000000046"), true));
            try {
                // STGM_READ. Loading initializes the object from the existing
                // Unicode .lnk; do not Resolve, Save, or activate its target.
                RequireSuccess(((IPersistFileReader)instance).Load(path, 0), "IPersistFile.Load");
                var link = (IShellLinkW)instance;
                var target = new StringBuilder(32768);
                var arguments = new StringBuilder(32768);
                var directory = new StringBuilder(32768);
                RequireSuccess(link.GetPath(target, target.Capacity, IntPtr.Zero, 0), "IShellLinkW.GetPath");
                RequireSuccess(link.GetArguments(arguments, arguments.Capacity), "IShellLinkW.GetArguments");
                RequireSuccess(link.GetWorkingDirectory(directory, directory.Capacity), "IShellLinkW.GetWorkingDirectory");
                return new string[] { target.ToString(), arguments.ToString(), directory.ToString() };
            } finally {
                if (instance != null) { Marshal.FinalReleaseComObject(instance); }
            }
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct FileIdentity {
            public ulong VolumeSerialNumber;
            public ulong FileIdLow;
            public ulong FileIdHigh;
        }
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern SafeFileHandle CreateFileW(string path, uint access,
            uint share, IntPtr security, uint creation, uint flags, IntPtr template);
        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool GetFileInformationByHandleEx(SafeFileHandle file,
            int infoClass, out FileIdentity information, uint size);
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern uint GetFinalPathNameByHandleW(SafeFileHandle file,
            StringBuilder path, uint capacity, uint flags);

        public static string[] IdentifyPath(string path, bool directory) {
            if (String.IsNullOrWhiteSpace(path)) {
                throw new ArgumentException("A shortcut path was empty.");
            }
            // Read attributes only; OPEN_EXISTING cannot create or modify files.
            // FILE_FLAG_BACKUP_SEMANTICS permits opening a directory handle.
            using (var file = CreateFileW(path, 0x80, 7, IntPtr.Zero, 3,
                directory ? 0x02000000u : 0x80u, IntPtr.Zero)) {
                if (file.IsInvalid) {
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "Opening a shortcut target for identity verification failed.");
                }
                FileIdentity identity;
                if (!GetFileInformationByHandleEx(file, 18, out identity,
                    (uint)Marshal.SizeOf(typeof(FileIdentity)))) {
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "Reading a shortcut target's file identity failed.");
                }
                var finalPath = new StringBuilder(32768);
                uint length = GetFinalPathNameByHandleW(file, finalPath, (uint)finalPath.Capacity, 0);
                if (length == 0 || length >= finalPath.Capacity) {
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "Resolving a shortcut target's final path failed.");
                }
                string id = identity.VolumeSerialNumber.ToString("X16") + ":" +
                    identity.FileIdLow.ToString("X16") + identity.FileIdHigh.ToString("X16");
                return new string[] { id, finalPath.ToString() };
            }
        }

        private delegate bool EnumWindowsCallback(IntPtr window, IntPtr state);
        [DllImport("user32.dll")]
        private static extern bool EnumWindows(EnumWindowsCallback callback, IntPtr state);
        [DllImport("user32.dll")]
        private static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
        [DllImport("user32.dll", CharSet = CharSet.Unicode)]
        private static extern int GetClassNameW(IntPtr window, StringBuilder name, int capacity);
        [DllImport("user32.dll")]
        public static extern IntPtr GetDlgItem(IntPtr window, int controlId);
        [DllImport("user32.dll")]
        public static extern bool IsWindowVisible(IntPtr window);
        [DllImport("user32.dll")]
        public static extern bool IsIconic(IntPtr window);
        [DllImport("user32.dll")]
        public static extern bool ShowWindowAsync(IntPtr window, int command);
        [DllImport("user32.dll", SetLastError = true)]
        private static extern IntPtr SendMessageTimeoutW(IntPtr window, uint message,
            UIntPtr wParam, IntPtr lParam, uint flags, uint timeout, out UIntPtr result);
        [DllImport("user32.dll", SetLastError = true)]
        public static extern bool PostMessageW(IntPtr window, uint message, UIntPtr wParam, IntPtr lParam);

        public static IntPtr WindowForProcess(uint processId) {
            IntPtr found = IntPtr.Zero;
            EnumWindows((window, state) => {
                uint owner;
                GetWindowThreadProcessId(window, out owner);
                if (owner == processId) {
                    var className = new StringBuilder(256);
                    GetClassNameW(window, className, className.Capacity);
                    if (className.ToString() == "CampusNetworkGuardianWindow") {
                        found = window;
                        return false;
                    }
                }
                return true;
            }, IntPtr.Zero);
            return found;
        }

        public static ulong ControlState(IntPtr window, uint message) {
            UIntPtr result;
            if (SendMessageTimeoutW(window, message, UIntPtr.Zero, IntPtr.Zero,
                2, 2000, out result) == IntPtr.Zero) {
                throw new Win32Exception(Marshal.GetLastWin32Error(), "Reading a test UI control timed out.");
            }
            return result.ToUInt64();
        }
    }
}
'@

function Assert-FileIdentityVerification {
    # Exercise only dummy data inside the guarded hosted-runner test directory.
    # This must pass before using file identities to validate an installed .lnk.
    $fixtureDirectory = Join-Path $smokeDirectory 'file-identity-regression'
    Assert-ChildPath -Path $fixtureDirectory -Parent $smokeDirectory
    New-Item -ItemType Directory -Path $fixtureDirectory | Out-Null
    $first = Join-Path $fixtureDirectory 'first fake file.data'
    $second = Join-Path $fixtureDirectory 'second fake file.data'
    Set-Content -LiteralPath $first -Value 'Identical dummy content; never executable.' -Encoding ascii
    Set-Content -LiteralPath $second -Value 'Identical dummy content; never executable.' -Encoding ascii
    $original = [CampusGuardianInstallerSmoke.Native]::IdentifyPath($first, $false)
    $caseAlias = [CampusGuardianInstallerSmoke.Native]::IdentifyPath($first.ToUpperInvariant(), $false)
    $finalAlias = [CampusGuardianInstallerSmoke.Native]::IdentifyPath($original[1], $false)
    $different = [CampusGuardianInstallerSmoke.Native]::IdentifyPath($second, $false)
    $directory = [CampusGuardianInstallerSmoke.Native]::IdentifyPath($fixtureDirectory, $true)
    $directoryAlias = [CampusGuardianInstallerSmoke.Native]::IdentifyPath((Join-Path $fixtureDirectory '.'), $true)
    Assert-Condition -Condition ($original[0] -ceq $caseAlias[0] -and $original[0] -ceq $finalAlias[0]) -Message 'File identity failed to recognize equivalent dummy paths.'
    Assert-Condition -Condition ($original[0] -cne $different[0]) -Message 'File identity incorrectly accepted different dummy files with identical contents.'
    Assert-Condition -Condition ($directory[0] -ceq $directoryAlias[0]) -Message 'File identity failed to recognize equivalent dummy directory paths.'
    Write-Host 'Hosted dummy-file identity checks passed: aliases match, same-content different files are rejected.'
}

function Wait-TestCondition {
    param([scriptblock] $Condition, [string] $Failure)
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    do {
        if (& $Condition) { return }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw $Failure
}

function Assert-EmptyConfiguration {
    # This deliberately cannot deserialize as SavedConfig: it has neither
    # identity, username nor HTTP consent. Keep it unchanged before launching.
    Assert-ConfigurationPreserved
    $dummy = Get-Content -LiteralPath $configurationPath -Raw | ConvertFrom-Json
    $properties = @($dummy.PSObject.Properties.Name)
    Assert-Condition -Condition ($dummy.smoke_test_only -eq $true -and $dummy.autostart -eq $false) -Message 'The test configuration marker is invalid.'
    foreach ($forbidden in @('identity', 'username', 'http_acknowledged')) {
        Assert-Condition -Condition ($properties -notcontains $forbidden) -Message 'The test configuration unexpectedly contains authentication data.'
    }
}

function Invoke-ExistingInstanceShow {
    param([IntPtr] $Window)
    $secondProcess = Start-Process -FilePath $applicationExecutable -ArgumentList '--show' -WindowStyle Hidden -PassThru
    try {
        Assert-Condition -Condition $secondProcess.WaitForExit(10000) -Message 'The second --show instance did not exit.'
        Assert-Condition -Condition ($secondProcess.ExitCode -eq 0) -Message 'The second --show instance failed.'
    } finally {
        $secondProcess.Dispose()
    }
    Wait-TestCondition -Condition {
        [CampusGuardianInstallerSmoke.Native]::IsWindowVisible($Window) -and
            -not [CampusGuardianInstallerSmoke.Native]::IsIconic($Window)
    } -Failure 'A second --show launch did not restore the existing window.'
    $instances = @(Get-Process -Name 'campus-network-guardian' -ErrorAction SilentlyContinue)
    Assert-Condition -Condition ($instances.Count -eq 1 -and $instances[0].Id -eq $script:testApplicationProcess.Id) -Message 'The --show launch did not preserve exactly one original application instance.'
}

function Start-TestApplicationAndCheckUi {
    Assert-EmptyConfiguration
    Assert-NoApplicationProcess
    Write-Host 'Installer smoke test: launch only the unconfigured application; no account, password, consent, save, or check action is supplied.'
    $script:testApplicationProcess = Start-Process -FilePath $applicationExecutable -ArgumentList '--show' -WindowStyle Hidden -PassThru
    Wait-TestCondition -Condition {
        if ($script:testApplicationProcess.HasExited) {
            throw 'The unconfigured application exited before creating its window. A hosted Windows desktop/tray surface is required.'
        }
        [CampusGuardianInstallerSmoke.Native]::WindowForProcess([uint32] $script:testApplicationProcess.Id) -ne [IntPtr]::Zero
    } -Failure 'The hosted runner did not expose the application window.'
    $window = [CampusGuardianInstallerSmoke.Native]::WindowForProcess([uint32] $script:testApplicationProcess.Id)
    foreach ($controlId in @(102, 103, 104, 105)) {
        Wait-TestCondition -Condition {
            [CampusGuardianInstallerSmoke.Native]::GetDlgItem($window, $controlId) -ne [IntPtr]::Zero
        } -Failure "The hosted application did not create test control $controlId."
    }
    # Query lengths/states only. Do not retrieve any password or account text.
    $account = [CampusGuardianInstallerSmoke.Native]::GetDlgItem($window, 102)
    $password = [CampusGuardianInstallerSmoke.Native]::GetDlgItem($window, 103)
    $consent = [CampusGuardianInstallerSmoke.Native]::GetDlgItem($window, 104)
    $startup = [CampusGuardianInstallerSmoke.Native]::GetDlgItem($window, 105)
    Assert-Condition -Condition ([CampusGuardianInstallerSmoke.Native]::ControlState($account, 0x000e) -eq 0) -Message 'The unconfigured account field must be empty.'
    Assert-Condition -Condition ([CampusGuardianInstallerSmoke.Native]::ControlState($password, 0x000e) -eq 0) -Message 'The unconfigured password field must be empty.'
    Assert-Condition -Condition ([CampusGuardianInstallerSmoke.Native]::ControlState($consent, 0x00f0) -eq 0) -Message 'The unconfigured application must not have HTTP consent.'
    Wait-TestCondition -Condition {
        [CampusGuardianInstallerSmoke.Native]::ControlState($startup, 0x00f0) -eq 1
    } -Failure 'The startup checkbox did not reflect the actual installer Run entry.'
    Assert-EmptyConfiguration

    [void] [CampusGuardianInstallerSmoke.Native]::ShowWindowAsync($window, 0)
    Wait-TestCondition -Condition { -not [CampusGuardianInstallerSmoke.Native]::IsWindowVisible($window) } -Failure 'The test window could not be hidden.'
    Invoke-ExistingInstanceShow -Window $window
    [void] [CampusGuardianInstallerSmoke.Native]::ShowWindowAsync($window, 6)
    Wait-TestCondition -Condition { [CampusGuardianInstallerSmoke.Native]::IsIconic($window) } -Failure 'The test window could not be minimized.'
    Invoke-ExistingInstanceShow -Window $window
    Assert-EmptyConfiguration
}

function Close-TestApplication {
    if ($null -eq $script:testApplicationProcess) { return }
    try {
        if (-not $script:testApplicationProcess.HasExited) {
            $window = [CampusGuardianInstallerSmoke.Native]::WindowForProcess([uint32] $script:testApplicationProcess.Id)
            Assert-Condition -Condition ($window -ne [IntPtr]::Zero) -Message 'The test process has no verified guardian window for cleanup.'
            [void] [CampusGuardianInstallerSmoke.Native]::PostMessageW($window, 0x0111, [UIntPtr] 204, [IntPtr]::Zero)
            Assert-Condition -Condition $script:testApplicationProcess.WaitForExit(10000) -Message 'The unconfigured test application did not exit gracefully.'
        }
    } finally {
        $script:testApplicationProcess.Dispose()
        $script:testApplicationProcess = $null
    }
}

function Invoke-TestInstall {
    param([string] $Stage)
    Write-Host "Installer smoke test: $Stage"
    $logPath = Join-Path $smokeDirectory "$Stage.log"
    Invoke-TestProcess -Executable $setupExecutable -Stage $Stage -LogPath $logPath -Arguments @(
        '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-',
        "/DIR=`"$applicationDirectory`"", "/LOG=`"$logPath`""
    )
}

function Invoke-TestUninstall {
    param([string] $Stage)
    Assert-ChildPath -Path $applicationDirectory -Parent $smokeDirectory
    $uninstallers = @(Get-ChildItem -LiteralPath $applicationDirectory -Filter 'unins*.exe' -File)
    Assert-Condition -Condition ($uninstallers.Count -eq 1) -Message 'Exactly one test uninstaller must be present.'
    Write-Host "Installer smoke test: $Stage"
    $logPath = Join-Path $smokeDirectory "$Stage.log"
    Invoke-TestProcess -Executable $uninstallers[0].FullName -Stage $Stage -LogPath $logPath -Arguments @(
        '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/LOG=`"$logPath`""
    )
}

function Assert-InstalledFiles {
    Assert-Condition -Condition (Test-Path -LiteralPath $applicationExecutable -PathType Leaf) -Message 'The application was not installed in the current-user test location.'
    $version = (Get-Item -LiteralPath $applicationExecutable).VersionInfo
    Assert-Condition -Condition (Test-WindowsBinaryVersion -VersionInfo $version -ExpectedVersion $ExpectedVersion) -Message 'The installed application version does not match the installer.'
    Assert-Condition -Condition (Test-Path -LiteralPath (Join-Path $applicationDirectory 'LICENSE.txt') -PathType Leaf) -Message 'The installed license is missing.'
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($uninstallKeyPath, $false)
    Assert-Condition -Condition ($null -ne $key) -Message 'The installation was not registered under the current user.'
    try {
        $location = [string] $key.GetValue('InstallLocation', '')
        Assert-Condition -Condition ($location.TrimEnd('\') -ieq $applicationDirectory.TrimEnd('\')) -Message 'The current-user uninstall registration has the wrong installation path.'
    } finally {
        $key.Dispose()
    }
    Assert-Condition -Condition (Test-Path -LiteralPath $shortcutPath -PathType Leaf) -Message 'The Start menu shortcut is missing.'
    $shortcutBytes = [System.IO.File]::ReadAllBytes($shortcutPath)
    $shortcutLinkFlags = if ($shortcutBytes.Length -ge 24) { '0x{0:X8}' -f [BitConverter]::ToUInt32($shortcutBytes, 0x14) } else { 'Header shorter than 24 bytes' }
    $shortcutDiagnostic = [ordered]@{
        ShortcutPath = $shortcutPath
        Reader = 'IPersistFile.Load + IShellLinkW (Unicode, read only)'
        ShortcutByteLength = $shortcutBytes.Length
        ShellLinkFlags = $shortcutLinkFlags
        ShortcutSHA256 = (Get-FileHash -LiteralPath $shortcutPath -Algorithm SHA256).Hash.ToLowerInvariant()
        RawShortcutBase64 = [Convert]::ToBase64String($shortcutBytes)
        ExpectedTargetPath = $applicationExecutable
        ExpectedArguments = '--show'
        ExpectedWorkingDirectory = $applicationDirectory
    }
    try {
        $shortcut = [CampusGuardianInstallerSmoke.Native]::ReadShortcut($shortcutPath)
        $shortcutDiagnostic['ActualTargetPath'] = $shortcut[0]
        $shortcutDiagnostic['ActualArguments'] = $shortcut[1]
        $shortcutDiagnostic['ActualWorkingDirectory'] = $shortcut[2]
        $actualIdentity = [CampusGuardianInstallerSmoke.Native]::IdentifyPath($shortcut[0], $false)
        $expectedIdentity = [CampusGuardianInstallerSmoke.Native]::IdentifyPath($applicationExecutable, $false)
        $actualDirectory = [CampusGuardianInstallerSmoke.Native]::IdentifyPath($shortcut[2], $true)
        $expectedDirectory = [CampusGuardianInstallerSmoke.Native]::IdentifyPath($applicationDirectory, $true)
        $shortcutDiagnostic['ActualTargetIdentity'] = $actualIdentity[0]
        $shortcutDiagnostic['ExpectedTargetIdentity'] = $expectedIdentity[0]
        $shortcutDiagnostic['ActualTargetFinalPath'] = $actualIdentity[1]
        $shortcutDiagnostic['ExpectedTargetFinalPath'] = $expectedIdentity[1]
        $shortcutDiagnostic['ActualWorkingDirectoryIdentity'] = $actualDirectory[0]
        $shortcutDiagnostic['ExpectedWorkingDirectoryIdentity'] = $expectedDirectory[0]
        Assert-Condition -Condition ($actualIdentity[0] -ceq $expectedIdentity[0]) -Message 'The Start menu shortcut targets a different executable file identity.'
        Assert-Condition -Condition ($actualDirectory[0] -ceq $expectedDirectory[0]) -Message 'The Start menu shortcut uses a different working directory identity.'
        Assert-Condition -Condition ($shortcut[1] -ceq '--show') -Message 'The Start menu shortcut must explicitly open the application window.'
    } catch {
        $shortcutDiagnostic['ReadOrIdentityFailure'] = $_.Exception.Message
        throw
    } finally {
        New-Item -ItemType Directory -Path $diagnosticDirectory -Force | Out-Null
        Set-Content -LiteralPath (Join-Path $diagnosticDirectory 'shortcut-diagnostic.json') -Value ($shortcutDiagnostic | ConvertTo-Json -Depth 4) -Encoding utf8
        $visibleDiagnostic = [ordered]@{}
        foreach ($name in $shortcutDiagnostic.Keys) {
            if ($name -ne 'RawShortcutBase64') { $visibleDiagnostic[$name] = $shortcutDiagnostic[$name] }
        }
        Write-Host ($visibleDiagnostic | ConvertTo-Json -Depth 4)
    }
}

function Assert-ConfigurationPreserved {
    Assert-Condition -Condition (Test-Path -LiteralPath $configurationPath -PathType Leaf) -Message 'The installer removed the fake user configuration.'
    $hash = (Get-FileHash -LiteralPath $configurationPath -Algorithm SHA256).Hash
    Assert-Condition -Condition ($hash -ceq $configurationHash) -Message 'The installer changed the fake user configuration.'
}

function Assert-UninstalledFiles {
    Assert-Condition -Condition (-not (Test-Path -LiteralPath $applicationExecutable)) -Message 'Uninstall left the installed application executable.'
    Assert-Condition -Condition (-not (Test-Path -LiteralPath $shortcutPath)) -Message 'Uninstall left the Start menu shortcut.'
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($uninstallKeyPath, $false)
    if ($null -ne $key) {
        $key.Dispose()
        throw 'Uninstall left its current-user uninstall registration.'
    }
}

# Fail closed before writing anything if this disposable runner is not clean.
Assert-ChildPath -Path $smokeDirectory -Parent $runnerTemporaryDirectory
Assert-NoApplicationProcess
Assert-Condition -Condition ($null -eq (Read-RunCommand)) -Message 'A pre-existing guardian Run value was found; refusing to alter it.'
Assert-Condition -Condition (-not (Test-Path -LiteralPath $configurationDirectory)) -Message 'Pre-existing guardian user data was found; refusing to alter it.'
Assert-Condition -Condition (-not (Test-Path -LiteralPath $shortcutDirectory)) -Message 'A pre-existing guardian Start menu folder was found; refusing to alter it.'
foreach ($hive in @([Microsoft.Win32.Registry]::CurrentUser, [Microsoft.Win32.Registry]::LocalMachine)) {
    $key = $hive.OpenSubKey($uninstallKeyPath, $false)
    if ($null -ne $key) {
        $key.Dispose()
        throw 'A pre-existing guardian installation was found; refusing to alter it.'
    }
}

New-Item -ItemType Directory -Path $smokeDirectory -Force | Out-Null
try {
    # load_config rejects this incomplete object. startup_controller(None) does
    # not read credentials, and worker config=None returns before portal work.
    New-Item -ItemType Directory -Path $configurationDirectory | Out-Null
    Set-Content -LiteralPath $configurationPath -Value '{"smoke_test_only":true,"autostart":false}' -Encoding ascii
    $configurationCreated = $true
    $configurationHash = (Get-FileHash -LiteralPath $configurationPath -Algorithm SHA256).Hash

    Assert-FileIdentityVerification

    Invoke-TestInstall -Stage 'fresh-install'
    Assert-InstalledFiles
    Assert-Condition -Condition ((Read-RunCommand) -ceq $ownRunCommand) -Message 'A fresh install must default to the exact quoted current-user Run command.'
    Assert-ConfigurationPreserved

    Start-TestApplicationAndCheckUi

    # Simulate disabling startup in Settings after an installer selected it.
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($runKeyPath, $true)
    try { $key.DeleteValue($runValueName, $true) } finally { $key.Dispose() }
    Invoke-TestInstall -Stage 'upgrade-user-disabled-startup'
    Assert-Condition -Condition $testApplicationProcess.WaitForExit(10000) -Message 'The upgrade did not close the matching installed application gracefully.'
    Assert-Condition -Condition ($testApplicationProcess.ExitCode -eq 0) -Message 'The application did not exit cleanly during upgrade.'
    Close-TestApplication
    Assert-InstalledFiles
    Assert-Condition -Condition ($null -eq (Read-RunCommand)) -Message 'An upgrade re-enabled startup after the user disabled it.'
    Assert-ConfigurationPreserved

    Write-RunCommand -Command $ownRunCommand
    Invoke-TestUninstall -Stage 'uninstall-owned-startup'
    Assert-Condition -Condition ($null -eq (Read-RunCommand)) -Message 'Uninstall left the Run value belonging to this installation.'
    Assert-UninstalledFiles
    Assert-ConfigurationPreserved

    Invoke-TestInstall -Stage 'reinstall-for-foreign-startup'
    Assert-InstalledFiles
    Assert-Condition -Condition ((Read-RunCommand) -ceq $ownRunCommand) -Message 'A clean reinstall did not restore the default startup choice.'
    Write-RunCommand -Command $foreignRunCommand
    Invoke-TestUninstall -Stage 'uninstall-preserves-foreign-startup'
    Assert-Condition -Condition ((Read-RunCommand) -ceq $foreignRunCommand) -Message 'Uninstall removed the Run value belonging to another executable location.'
    Assert-UninstalledFiles
    Assert-ConfigurationPreserved
    $smokeSucceeded = $true
    Write-Host 'Installer smoke test passed: silent install, version, shortcuts, actual Run checkbox, single-instance --show restoration, running-app upgrade, disabled-startup retention, owned/foreign Run cleanup, and user data retention.'
    Write-Host 'The application was launched only without valid configuration or HTTP consent; no authentication input, save, check, portal action, or Credential Manager operation was performed.'
} finally {
    # Clean up only paths/values created by this test on this hosted runner.
    try { Close-TestApplication } catch { $cleanupFailures.Add($_.Exception.Message) }
    try {
        if (Test-Path -LiteralPath $applicationDirectory -PathType Container) {
            $uninstallers = @(Get-ChildItem -LiteralPath $applicationDirectory -Filter 'unins*.exe' -File)
            if ($uninstallers.Count -eq 1) { Invoke-TestUninstall -Stage 'cleanup-uninstall' }
        }
    } catch { $cleanupFailures.Add($_.Exception.Message) }
    try { Remove-TestRunCommand } catch { $cleanupFailures.Add($_.Exception.Message) }
    try {
        if ($configurationCreated -and (Test-Path -LiteralPath $configurationPath -PathType Leaf)) {
            $currentHash = (Get-FileHash -LiteralPath $configurationPath -Algorithm SHA256).Hash
            if ($currentHash -ceq $configurationHash) {
                Remove-Item -LiteralPath $configurationPath -Force
                if (@(Get-ChildItem -LiteralPath $configurationDirectory -Force).Count -eq 0) {
                    Remove-Item -LiteralPath $configurationDirectory
                }
            }
        }
    } catch { $cleanupFailures.Add($_.Exception.Message) }
    if ($smokeSucceeded -and $cleanupFailures.Count -eq 0) {
        Assert-ChildPath -Path $smokeDirectory -Parent $runnerTemporaryDirectory
        Remove-Item -LiteralPath $smokeDirectory -Recurse -Force
    }
    if ($cleanupFailures.Count -gt 0) {
        throw ('Installer smoke-test cleanup failed: ' + ($cleanupFailures -join '; '))
    }
}
