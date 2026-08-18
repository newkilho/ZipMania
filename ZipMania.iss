#define MySrcDir           "D:\Vendor\ZipMania"
#define MyAppExe           MySrcDir + "\ZipMania.exe"
#define MyShellDll         MySrcDir + "\ZipManiaShell.dll"
#define MyAppName          "ZipMania"
#define MyAppAuthor        "Kilhonet"
#define MyAppPublisherURL  "https://kilho.net"
#define StartYearCopyright "2026"
#define CurrentYear        GetDateTimeString('yyyy','','')

#define MyAppVersion() \
   ParseVersion(MyAppExe, Local[0], Local[1], Local[2], Local[3]), \
   Str(Local[0]) + "." + Str(Local[1]) + "." + Str(Local[2]) + "." + Str(Local[3])

#define MyShellDllVersion() \
   ParseVersion(MyShellDll, Local[0], Local[1], Local[2], Local[3]), \
   Str(Local[0]) + "." + Str(Local[1]) + "." + Str(Local[2]) + "." + Str(Local[3])

#include "D:\Component\InnoDependencyInstaller\CodeDependencies.iss"

[Files]
Source: "{#MyAppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#MySrcDir}\7z.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#MyShellDll}"; DestDir: "{app}"; Flags: ignoreversion; Check: ShouldInstallShellDll
Source: "{#MySrcDir}\THIRD-PARTY-NOTICES.txt"; DestDir: "{app}"; Flags: ignoreversion

[Setup]
AppId={#MyAppName}
AppName={cm:MyAppName}
AppVersion={#MyAppVersion}
AppVerName={cm:MyAppName} {#MyAppVersion}

AppPublisher={#MyAppAuthor}
AppPublisherURL={#MyAppPublisherURL}
AppSupportURL={#MyAppPublisherURL}
AppUpdatesURL={#MyAppPublisherURL}
AppCopyright=Copyright (C) {#StartYearCopyright}-{#CurrentYear} {#MyAppAuthor}

VersionInfoDescription={#MyAppName} installer
VersionInfoVersion={#MyAppVersion}
VersionInfoCompany={#MyAppAuthor}
VersionInfoCopyright={#MyAppAuthor}
VersionInfoProductName={#MyAppName}

WizardStyle=modern
SetupIconFile=src-tauri\icons\icon.ico

ShowLanguageDialog=no
UsePreviousLanguage=no

ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

UsedUserAreasWarning=no
DefaultDirName={localappdata}\{#MyAppName}
DefaultGroupName={cm:MyAppName}
OutputDir="Z:\Release"
OutputBaseFilename={#MyAppName}Setup
UninstallDisplayIcon={app}\{#MyAppName}.exe
UninstallDisplayName={cm:MyAppName}

PrivilegesRequired=admin
DisableStartupPrompt=true
DisableProgramGroupPage=true
Compression=lzma/ultra64
UsePreviousAppDir=true
DisableDirPage=auto
UserInfoPage=false
ShowTasksTreeLines=false
AlwaysShowDirOnReadyPage=false
AlwaysShowGroupOnReadyPage=false
FlatComponentsList=true
DisableFinishedPage=True
InternalCompressLevel=ultra64
SolidCompression=true
UninstallFilesDir={app}
AllowCancelDuringInstall=false
CreateUninstallRegKey=true
UninstallLogMode=overwrite
UpdateUninstallLogAppName=false
RestartIfNeededByRun=true
WizardImageStretch=true
SetupLogging=false
AppendDefaultDirName=false
DisableReadyPage=True

[Languages]
Name: en; InfoBeforeFile: "ZipMania(en).txt"; MessagesFile: "compiler:Default.isl"
Name: ko; InfoBeforeFile: "ZipMania(ko).txt"; MessagesFile: "compiler:Languages\Korean.isl"

[CustomMessages]
en.MyAppName=ZipMania
ko.MyAppName=집매니아
en.RegisteringShell=Registering Explorer integration...
ko.RegisteringShell=탐색기 통합을 등록하는 중...
en.RestartingExplorer=Explorer is using the shell extension. Restarting it briefly...
ko.RestartingExplorer=탐색기가 셸 확장을 사용 중입니다. 잠시 다시 시작합니다...

[Icons]
Name: {group}\{cm:MyAppName}; Filename: {app}\{#MyAppName}.exe

[Run]
Filename: "{app}\{#MyAppName}.exe"; Parameters: "/inst"; \
  StatusMsg: "{cm:RegisteringShell}"; Flags: runasoriginaluser runhidden waituntilterminated
Filename: "{app}\{#MyAppName}.exe"; Flags: nowait postinstall skipifsilent runasoriginaluser; \
  Description: "{cm:MyAppName}"

[UninstallDelete]
Name: "{app}"; Type: filesandordirs

[Code]
const
  SHELL_DLL       = 'ZipManiaShell.dll';
  SHELL_DLL_OLD   = 'ZipManiaShell.old.dll';

var
  ExplorerStopped: Boolean;
  ShellDllNeeded: Boolean;
  ShellDllChecked: Boolean;

procedure TaskKill(FileName: String);
var
  ErrorCode: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/f /im "' + FileName + '"', '',
       SW_HIDE, ewWaitUntilTerminated, ErrorCode);
end;

function CanWriteFile(const FileName: String): Boolean;
begin
  Result := SaveStringToFile(FileName, '', True);
end;

function ShellDllPath: String;
begin
  Result := ExpandConstant('{app}\') + SHELL_DLL;
end;

function InstalledShellDllVersion: String;
begin
  if not GetVersionNumbersString(ShellDllPath, Result) then
    Result := '';
end;

function ShouldInstallShellDll: Boolean;
begin
  if not ShellDllChecked then begin
    ShellDllNeeded := InstalledShellDllVersion <> '{#MyShellDllVersion}';
    ShellDllChecked := True;
  end;
  Result := ShellDllNeeded;
end;

function ReleaseShellDll: Boolean;
var
  Path: String;
  Loop: Integer;
begin
  Result := True;
  Path := ShellDllPath;
  if not FileExists(Path) then
    exit;
  if CanWriteFile(Path) then
    exit;

  TaskKill('explorer.exe');
  ExplorerStopped := True;

  for Loop := 0 to 19 do begin
    Sleep(250);
    if CanWriteFile(Path) then
      exit;
  end;

  Result := False;
end;

procedure RestartExplorer;
var
  ErrorCode: Integer;
begin
  if not ExplorerStopped then
    exit;
  ExplorerStopped := False;
  if FindWindowByClassName('Shell_TrayWnd') <> 0 then
    exit;
  Exec(ExpandConstant('{win}\explorer.exe'), '', '', SW_SHOWNORMAL, ewNoWait, ErrorCode);
end;

procedure PrepareShellDllSlot;
var
  OldPath: String;
begin
  OldPath := ExpandConstant('{app}\') + SHELL_DLL_OLD;
  DeleteFile(OldPath);

  if not ReleaseShellDll then
    RenameFile(ShellDllPath, OldPath);
end;

function InitializeSetup: Boolean;
begin
  Dependency_AddWebView2;
  Result := True;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := '';

  TaskKill('ZipMania.exe');

  if ShouldInstallShellDll then begin
    WizardForm.StatusLabel.Caption := ExpandConstant('{cm:RestartingExplorer}');
    PrepareShellDllSlot;
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
    RestartExplorer;
end;

function InitializeUninstall: Boolean;
var
  ErrorCode: Integer;
begin
  Result := True;

  TaskKill('ZipMania.exe');

  Exec(ExpandConstant('{app}\ZipMania.exe'), '/uninst', '',
       SW_HIDE, ewWaitUntilTerminated, ErrorCode);
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    PrepareShellDllSlot;

  if CurUninstallStep = usPostUninstall then
    RestartExplorer;
end;
