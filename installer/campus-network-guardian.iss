; Build with tools/build-installer.ps1. This script never bundles user settings,
; credentials, logs, or the developer's running executable.
#ifndef AppVersion
  #define AppVersion "0.5.1"
#endif
#ifndef RepoRoot
  #define RepoRoot ExtractFileDir(ExtractFileDir(__FILE__))
#endif
#ifndef SourceExe
  #define SourceExe RepoRoot + "\target\installer-build\x86_64-pc-windows-msvc\release\campus-network-guardian.exe"
#endif
#ifndef SetupOutputDir
  #define SetupOutputDir RepoRoot + "\dist"
#endif
#define AppExeName "campus-network-guardian.exe"
#define AppName "校园网守护"

[Setup]
AppId=CampusNetworkGuardian.Windows
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=Liunian520
AppPublisherURL=https://github.com/Liunian520
AppSupportURL=https://github.com/Liunian520/campus-network-guardian/issues
AppUpdatesURL=https://github.com/Liunian520/campus-network-guardian/releases/latest
VersionInfoVersion={#AppVersion}.0
VersionInfoProductVersion={#AppVersion}.0
VersionInfoProductName={#AppName}
VersionInfoDescription={#AppName} Windows 安装程序
DefaultDirName={localappdata}\Programs\CampusNetworkGuardian
DefaultGroupName={#AppName}
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; GetDpiForSystem is imported by the native UI and requires Windows 10 1607.
MinVersion=10.0.14393
DisableDirPage=yes
DisableProgramGroupPage=yes
DisableWelcomePage=yes
DisableReadyPage=yes
UsePreviousAppDir=yes
UsePreviousTasks=yes
WizardStyle=modern
SetupIconFile={#RepoRoot}\assets\guardian.ico
UninstallDisplayIcon={app}\{#AppExeName}
UninstallDisplayName={#AppName}
OutputDir={#SetupOutputDir}
OutputBaseFilename=campus-network-guardian-{#AppVersion}-windows-x64-setup
Compression=lzma2/max
SolidCompression=yes
CloseApplications=no
RestartApplications=no
SetupLogging=yes

[Languages]
Name: "chinesesimplified"; MessagesFile: "{#RepoRoot}\installer\languages\ChineseSimplified.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "autostart"; Description: "登录 Windows 后自动启动校园网守护"; GroupDescription: "启动选项："
Name: "desktopicon"; Description: "创建桌面快捷方式"; GroupDescription: "快捷方式："; Flags: unchecked

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "{#AppExeName}"; Flags: ignoreversion
Source: "{#RepoRoot}\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
Source: "{#RepoRoot}\THIRD-PARTY-NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#RepoRoot}\installer\languages\INNO-LICENSE.txt"; DestDir: "{app}\licenses"; Flags: ignoreversion
Source: "{#RepoRoot}\installer\languages\THIRD-PARTY.md"; DestDir: "{app}\licenses"; Flags: ignoreversion

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExeName}"; Parameters: "--show"; WorkingDir: "{app}"
Name: "{group}\卸载{#AppName}"; Filename: "{uninstallexe}"
Name: "{userdesktop}\{#AppName}"; Filename: "{app}\{#AppExeName}"; Parameters: "--show"; WorkingDir: "{app}"; Tasks: desktopicon

[Registry]
; Deliberately omit uninsdeletevalue: uninstall must not delete a Run entry
; subsequently changed to another copy of the application.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "CampusNetworkGuardian"; ValueData: """{app}\{#AppExeName}"""; Tasks: autostart

[Run]
Filename: "{app}\{#AppExeName}"; Parameters: "--show"; Description: "启动校园网守护"; WorkingDir: "{app}"; Flags: postinstall nowait skipifsilent

[Code]
const
  GuardianWindowClass = 'CampusNetworkGuardianWindow';
  RunKey = 'Software\Microsoft\Windows\CurrentVersion\Run';
  RunValueName = 'CampusNetworkGuardian';
  GuardianQuitCommand = 204;
  GuardianCommandMessage = $0111;
  ProcessQueryLimitedInformation = $1000;
  ProcessSynchronize = $100000;
  WaitObject0 = 0;
  WaitTimeout = 258;

var
  PendingAutostartSelection, DesiredAutostartSelection: Boolean;

function FindGuardianWindow(Parent, After: HWND; ClassName: String;
  WindowName: Cardinal): HWND;
  external 'FindWindowExW@user32.dll stdcall';
function GuardianWindowProcess(Window: HWND; var ProcessId: Cardinal): Cardinal;
  external 'GetWindowThreadProcessId@user32.dll stdcall';
function OpenGuardianProcess(Access: Cardinal; Inherit: Boolean;
  ProcessId: Cardinal): Cardinal;
  external 'OpenProcess@kernel32.dll stdcall';
function QueryGuardianPath(Process: Cardinal; Flags: Cardinal;
  Buffer: String; var Characters: Cardinal): Boolean;
  external 'QueryFullProcessImageNameW@kernel32.dll stdcall';
function WaitGuardianProcess(Process: Cardinal; Milliseconds: Cardinal): Cardinal;
  external 'WaitForSingleObject@kernel32.dll stdcall';
function CloseGuardianHandle(Handle: Cardinal): Boolean;
  external 'CloseHandle@kernel32.dll stdcall';

function InstalledExecutable: String;
begin
  Result := ExpandFileName(ExpandConstant('{app}\{#AppExeName}'));
end;

function CanonicalExecutable(const Path: String): String;
begin
  Result := Path;
  if Copy(Result, 1, 4) = '\\?\' then
    Delete(Result, 1, 4);
  Result := ExpandFileName(Result);
end;

function MatchesRunExecutable(const Command, Expected: String): Boolean;
var
  Value: String;
begin
  Value := Trim(Command);
  { Accept only the exact executable command with no additional arguments. }
  Result := (CompareText(Value, '"' + Expected + '"') = 0) or
    (CompareText(Value, Expected) = 0);
end;

function MatchesInstalledRun(const Command: String): Boolean;
begin
  Result := MatchesRunExecutable(Command, InstalledExecutable);
end;

function TasksWereExplicitlySelected: Boolean;
var
  Index: Integer;
  Argument: String;
begin
  Result := False;
  for Index := 1 to ParamCount do begin
    Argument := UpperCase(ParamStr(Index));
    if (Copy(Argument, 1, 7) = '/TASKS=') or
      ((Copy(Argument, 1, 12) = '/MERGETASKS=') and
       (Pos('AUTOSTART', Argument) > 0)) then begin
      Result := True;
      Exit;
    end;
  end;
end;

procedure InitializeWizard;
var
  Command, Expected: String;
begin
  { First install defaults to checked. On upgrades honor changes made in the
    application's settings since the last installation, unless /TASKS was set. }
  { The app constant is not initialized yet. WizardDirValue already includes
    the default, previous, or command-line directory selected by the wizard. }
  Expected := ExpandFileName(AddBackslash(WizardDirValue) + '{#AppExeName}');
  PendingAutostartSelection := False;
  if FileExists(Expected) and not TasksWereExplicitlySelected then begin
    DesiredAutostartSelection :=
      RegQueryStringValue(HKCU, RunKey, RunValueName, Command) and
      MatchesRunExecutable(Command, Expected);
    PendingAutostartSelection := True;
    if DesiredAutostartSelection then
      Log('Upgrade autostart preference: matching Run value exists; select autostart.')
    else
      Log('Upgrade autostart preference: no matching Run value; deselect autostart.');
  end else if FileExists(Expected) then
    Log('Upgrade autostart preference: honor explicit command-line task selection.')
  else
    Log('Fresh installation: keep the default or explicit command-line task selection.');
end;

procedure CurPageChanged(CurPageID: Integer);
begin
  { Inno creates task items only when entering this page, after
    InitializeWizard. Apply the saved preference once, after that creation;
    revisiting the page must keep any subsequent choice made by the user. }
  if (CurPageID = wpSelectTasks) and PendingAutostartSelection then begin
    if DesiredAutostartSelection then
      WizardSelectTasks('autostart')
    else
      WizardSelectTasks('!autostart');
    PendingAutostartSelection := False;
    if WizardIsTaskSelected('autostart') then
      Log('Applied upgrade autostart preference: task is selected.')
    else
      Log('Applied upgrade autostart preference: task is deselected.');
  end;
end;

procedure RemoveOwnedAutostart;
var
  Command: String;
begin
  if RegQueryStringValue(HKCU, RunKey, RunValueName, Command) and
    MatchesInstalledRun(Command) then begin
    if not RegDeleteValue(HKCU, RunKey, RunValueName) then
      RaiseException('无法移除本安装的自启动设置，请稍后重试。');
    Log('Removed the Run value belonging to this installation.');
  end;
end;

function CloseInstalledGuardian: String;
var
  Window, NextWindow: HWND;
  Process: Cardinal;
  ProcessId, VerifiedProcessId, Characters, WaitResult: Cardinal;
  Buffer, ActualPath: String;
  Attempt: Integer;
begin
  Result := '';
  Window := FindGuardianWindow(0, 0, GuardianWindowClass, 0);
  while Window <> 0 do begin
    NextWindow := FindGuardianWindow(0, Window, GuardianWindowClass, 0);
    ProcessId := 0;
    GuardianWindowProcess(Window, ProcessId);
    if ProcessId <> 0 then begin
      Process := OpenGuardianProcess(
        ProcessQueryLimitedInformation or ProcessSynchronize, False, ProcessId);
      if Process = 0 then begin
        Result := '无法确认某个校园网守护进程的路径。请从托盘退出该程序后重试。';
        Exit;
      end;
      try
        Characters := 32768;
        SetLength(Buffer, Characters);
        if not QueryGuardianPath(Process, 0, Buffer, Characters) then begin
          Result := '无法确认校园网守护进程的路径。请从托盘退出该程序后重试。';
          Exit;
        end;
        SetLength(Buffer, Characters);
        ActualPath := CanonicalExecutable(Buffer);
        if CompareText(ActualPath, InstalledExecutable) = 0 then begin
          { Recheck the window's owner immediately before posting the command. }
          VerifiedProcessId := 0;
          GuardianWindowProcess(Window, VerifiedProcessId);
          if VerifiedProcessId <> ProcessId then begin
            Result := '校园网守护进程发生变化。请从托盘退出该程序后重试。';
            Exit;
          end;
          { WM_CLOSE only hides this app. Use its existing graceful Quit command.
            This invalidates pending operations without logging out the network. }
          if not PostMessage(Window, GuardianCommandMessage, GuardianQuitCommand, 0) then begin
            Result := '无法正常退出已安装的校园网守护。请从托盘退出后重试。';
            Exit;
          end;
          WaitResult := WaitTimeout;
          for Attempt := 1 to 50 do begin
            WaitResult := WaitGuardianProcess(Process, 200);
            if WaitResult <> WaitTimeout then
              Break;
          end;
          if WaitResult <> WaitObject0 then begin
            Result := '校园网守护尚未退出。请等待网络操作结束，或从托盘退出后重试。';
            Exit;
          end;
          Log('The matching installed guardian exited gracefully.');
        end;
      finally
        CloseGuardianHandle(Process);
      end;
    end;
    Window := NextWindow;
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := CloseInstalledGuardian;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if (CurStep = ssPostInstall) and not WizardIsTaskSelected('autostart') then
    RemoveOwnedAutostart;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Error: String;
begin
  if CurUninstallStep = usUninstall then begin
    Error := CloseInstalledGuardian;
    if Error <> '' then begin
      MsgBox(Error, mbError, MB_OK);
      Abort;
    end;
    RemoveOwnedAutostart;
    { Application configuration and Windows Credential Manager entries are
      intentionally retained; uninstall removes only installed program files. }
  end;
end;
