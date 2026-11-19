; Build with tools/build-installer.ps1. This script never bundles user settings,
; credentials, logs, or the developer's running executable.
#ifndef AppVersion
  #define AppVersion "0.6.4"
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
SetupMutex=Local\CampusNetworkGuardian.Setup
Uninstallable=not IsPortableUpdate
CreateUninstallRegKey=not IsPortableUpdate
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
Source: "{#RepoRoot}\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion; Check: IsRegisteredMode
Source: "{#RepoRoot}\THIRD-PARTY-NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion; Check: IsRegisteredMode
Source: "{#RepoRoot}\installer\languages\INNO-LICENSE.txt"; DestDir: "{app}\licenses"; Flags: ignoreversion; Check: IsRegisteredMode
Source: "{#RepoRoot}\installer\languages\THIRD-PARTY.md"; DestDir: "{app}\licenses"; Flags: ignoreversion; Check: IsRegisteredMode

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExeName}"; Parameters: "--show"; WorkingDir: "{app}"; Check: IsRegisteredMode
Name: "{group}\卸载{#AppName}"; Filename: "{uninstallexe}"; Check: IsRegisteredMode
Name: "{userdesktop}\{#AppName}"; Filename: "{app}\{#AppExeName}"; Parameters: "--show"; WorkingDir: "{app}"; Tasks: desktopicon; Check: IsRegisteredMode

[Registry]
; Deliberately omit uninsdeletevalue: uninstall must not delete a Run entry
; subsequently changed to another copy of the application.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "CampusNetworkGuardian"; ValueData: """{app}\{#AppExeName}"""; Tasks: autostart; Check: IsRegisteredMode

[Run]
Filename: "{app}\{#AppExeName}"; Parameters: "--show"; Description: "启动校园网守护"; WorkingDir: "{app}"; Flags: postinstall nowait skipifsilent

[Code]
const
  GuardianWindowClass = 'CampusNetworkGuardianWindow';
  RunKey = 'Software\Microsoft\Windows\CurrentVersion\Run';
  RunValueName = 'CampusNetworkGuardian';
  UninstallKey = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\CampusNetworkGuardian.Windows_is1';
  UpdateReadyPrefix = 'CampusNetworkGuardian.UpdateReady.';
  GuardianQuitCommand = 204;
  GuardianCommandMessage = $0111;
  ProcessQueryLimitedInformation = $1000;
  ProcessSynchronize = $100000;
  WaitObject0 = 0;
  WaitTimeout = 258;
  EventModifyState = 2;
  MoveFileReplaceExisting = 1;
  MoveFileWriteThrough = 8;

var
  PendingAutostartSelection, DesiredAutostartSelection: Boolean;
  UpdateBackupPath, UpdateRestartPath: String;
  UpdateOwnerProcess: Cardinal;
  UpdateReadyEvent: String;
  UpdateCompleted: Boolean;

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
function MoveGuardianFile(const Source, Destination: String; Flags: Cardinal): Boolean;
  external 'MoveFileExW@kernel32.dll stdcall';
function GuardianIsAdmin: Boolean;
  external 'IsUserAnAdmin@shell32.dll stdcall';
function OpenGuardianEvent(Access: Cardinal; Inherit: Boolean; const Name: String): Cardinal;
  external 'OpenEventW@kernel32.dll stdcall';
function SignalGuardianEvent(Handle: Cardinal): Boolean;
  external 'SetEvent@kernel32.dll stdcall';

function HasSwitch(const SwitchName: String): Boolean;
var
  Index: Integer;
begin
  Result := False;
  for Index := 1 to ParamCount do
    if CompareText(ParamStr(Index), SwitchName) = 0 then begin
      Result := True;
      Exit;
    end;
end;

function IsAutomaticUpdate: Boolean;
begin
  Result := HasSwitch('/GUARDIANUPDATE');
end;

function IsPortableUpdate: Boolean;
begin
  Result := IsAutomaticUpdate and HasSwitch('/GUARDIANPORTABLE');
end;

function IsRegisteredMode: Boolean;
begin
  Result := not IsPortableUpdate;
end;

function CanonicalExecutable(const Path: String): String;
begin
  Result := Path;
  if Copy(Result, 1, 4) = '\\?\' then
    Delete(Result, 1, 4);
  Result := ExpandFileName(Result);
end;

function InstalledExecutable: String;
begin
  Result := CanonicalExecutable(ExpandConstant('{app}\{#AppExeName}'));
end;

function ValidateUpdateHandoff: Boolean;
var
  OwnerText, Suffix: String;
  Owner, Index: Integer;
begin
  OwnerText := ExpandConstant('{param:GUARDIANOWNER|}');
  UpdateReadyEvent := ExpandConstant('{param:GUARDIANREADY|}');
  Result := (OwnerText = '') and (UpdateReadyEvent = '');
  if Result then
    Exit;
  Owner := StrToIntDef(OwnerText, 0);
  if (Owner <= 0) or (Length(UpdateReadyEvent) <> Length(UpdateReadyPrefix) + 32) or
    (Copy(UpdateReadyEvent, 1, Length(UpdateReadyPrefix)) <> UpdateReadyPrefix) then
    Exit;
  Suffix := UpperCase(Copy(UpdateReadyEvent, Length(UpdateReadyPrefix) + 1, 32));
  for Index := 1 to Length(Suffix) do
    if Pos(Copy(Suffix, Index, 1), '0123456789ABCDEF') = 0 then
      Exit;
  UpdateOwnerProcess := Owner;
  Result := True;
end;

function ValidateUpdateTarget: String;
var
  Directory, RegisteredDirectory, Target: String;
  IsRegistered: Boolean;
  CurrentVersion, SetupVersion: Int64;
begin
  Result := '';
  if GuardianIsAdmin then begin
    Result := '自动更新必须在普通用户权限下执行。';
    Exit;
  end;
  if not ValidateUpdateHandoff then begin
    Result := '自动更新的进程交接参数无效。';
    Exit;
  end;
  Directory := ExpandConstant('{param:DIR|}');
  Target := InstalledExecutable;
  if not WizardSilent or (Directory = '') or
    (CompareText(CanonicalExecutable(AddBackslash(Directory) + '{#AppExeName}'), Target) <> 0) then begin
    Result := '自动更新必须使用已验证的当前程序目录。';
    Exit;
  end;
  IsRegistered := RegQueryStringValue(HKCU, UninstallKey, 'InstallLocation', RegisteredDirectory) and
    (RegisteredDirectory <> '') and
    (CompareText(CanonicalExecutable(AddBackslash(RegisteredDirectory) + '{#AppExeName}'), Target) = 0);
  if IsRegistered = IsPortableUpdate then begin
    Result := '更新模式与当前程序的安装记录不一致。';
    Exit;
  end;
  if not GetPackedVersion(Target, CurrentVersion) or
    not GetPackedVersion(ExpandConstant('{srcexe}'), SetupVersion) or
    (ComparePackedVersion(CurrentVersion, SetupVersion) >= 0) then
    Result := '无法确认当前版本低于更新版本。';
end;

function NotifyUpdateOwner: Boolean;
var
  Signal: Cardinal;
begin
  Result := False;
  Signal := OpenGuardianEvent(EventModifyState, False, 'Local\' + UpdateReadyEvent);
  if Signal = 0 then
    Exit;
  try
    Result := SignalGuardianEvent(Signal);
  finally
    CloseGuardianHandle(Signal);
  end;
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
  if IsPortableUpdate then
    Log('Portable update: preserve installation and startup registration.')
  else if FileExists(Expected) and not TasksWereExplicitlySelected then begin
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
    if (ProcessId <> 0) and ((UpdateOwnerProcess = 0) or (ProcessId = UpdateOwnerProcess)) then begin
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
          if IsAutomaticUpdate then begin
            { Keep a same-volume backup before asking the application to exit. }
            UpdateBackupPath := GenerateUniqueName(ExtractFileDir(ActualPath), '.guardian-backup');
            if not FileCopy(ActualPath, UpdateBackupPath, True) then begin
              UpdateBackupPath := '';
              Result := '无法备份现有程序，自动更新已取消。';
              Exit;
            end;
          end;
          { Recheck the window's owner immediately before posting the command. }
          VerifiedProcessId := 0;
          GuardianWindowProcess(Window, VerifiedProcessId);
          if VerifiedProcessId <> ProcessId then begin
            Result := '校园网守护进程发生变化。请从托盘退出该程序后重试。';
            Exit;
          end;
          { The elevated owner closes its own window after our independent
            verification and backup. An ordinary updater can request Quit. }
          if UpdateOwnerProcess <> 0 then begin
            if not NotifyUpdateOwner then begin
              Result := '无法通知校园网守护接续自动更新，已取消更新。';
              Exit;
            end;
          end else if not PostMessage(Window, GuardianCommandMessage, GuardianQuitCommand, 0) then begin
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
          if IsAutomaticUpdate then begin
            UpdateRestartPath := ActualPath;
            Log('The matching guardian exited gracefully for automatic update.');
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
  if IsAutomaticUpdate then begin
    Result := ValidateUpdateTarget;
    if (Result <> '') or (UpdateRestartPath <> '') then
      Exit;
  end;
  Result := CloseInstalledGuardian;
  if IsAutomaticUpdate and (Result = '') and (UpdateRestartPath = '') then
    Result := '自动更新要求运行中的校园网守护与目标路径一致。';
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if (CurStep = ssPostInstall) and IsRegisteredMode and not WizardIsTaskSelected('autostart') then
    RemoveOwnedAutostart;
  if CurStep = ssDone then
    UpdateCompleted := True;
end;

function RestoreUpdateBackup: Boolean;
begin
  Result := MoveGuardianFile(UpdateBackupPath, UpdateRestartPath, MoveFileReplaceExisting or MoveFileWriteThrough);
  if Result then begin
    UpdateBackupPath := '';
    Log('Restored the previous guardian executable after an automatic update failure.');
  end else begin
    { Replacement can fail without touching the original. Launch it again only
      when it is byte-identical to the backup taken before the app exited. }
    if FileExists(UpdateRestartPath) then begin
      try
        Result := GetSHA256OfFile(UpdateBackupPath) = GetSHA256OfFile(UpdateRestartPath);
      except
        Result := False;
      end;
    end;
    if Result then
      Log('The original guardian executable is intact; restart it after the update failed.')
    else
      Log('Could not restore the previous executable; kept the automatic update backup.');
  end;
end;

procedure DeinitializeSetup;
var
  ResultCode: Integer;
begin
  if UpdateBackupPath = '' then
    Exit;
  if UpdateRestartPath <> '' then begin
    if not UpdateCompleted and not RestoreUpdateBackup then
      Exit;
    { Automatic Setup and its restored application run as the ordinary user. }
    if not Exec(UpdateRestartPath, '--from-update', ExtractFileDir(UpdateRestartPath), SW_SHOWNORMAL, ewNoWait, ResultCode) then begin
      Log('Could not restart the updated guardian: ' + SysErrorMessage(ResultCode));
      if UpdateBackupPath <> '' then begin
        if not RestoreUpdateBackup then
          Exit;
        if not Exec(UpdateRestartPath, '--from-update', ExtractFileDir(UpdateRestartPath), SW_SHOWNORMAL, ewNoWait, ResultCode) then
          Log('Could not restart the restored guardian: ' + SysErrorMessage(ResultCode));
      end;
    end;
  end;
  if UpdateBackupPath <> '' then
    DeleteFile(UpdateBackupPath);
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
