; ZipMania(집매니아) Inno Setup 설치 스크립트
;
; 빌드 순서: Build.bat -> 서명(D:\Vendor\ZipMania 에 복사) -> 이 스크립트 컴파일.
; 담는 파일은 전부 D:\Vendor\ZipMania 의 **서명본**이다. 그래서 이 스크립트는 cargo 를
; 부르지 않으며, 컴파일이 서명을 지울 일도 없다.
;
; 정해 둔 동작은 아래와 같다.
;   - 설치 폴더는 %LOCALAPPDATA%\ZipMania 이면서 **관리자 승격**을 받는다
;     (승격은 WebView2 설치에 필요하고, 설치 폴더는 앱이 자기 옆에 settings.toml 을 써야 해서 사용자 폴더다)
;   - 기존 설치가 있으면 아무것도 묻지 않고 그 위에 덮어 설치한다
;   - 실행 중인 앱은 묻지 않고 종료시킨다
;   - 완료 페이지를 띄우지 않고 끝나면 곧바로 앱을 실행한다
;   - 한국어로 설치하면 표시 이름이 "집매니아" 다(폴더·AppId 는 ZipMania 그대로 — 그건 신원이다)
;   - 셸 확장/파일 연결은 설치 프로그램이 직접 쓰지 않는다. ZipMania.exe /inst 를
;     **사용자 권한으로** 부른다(승격된 프로세스가 HKCU 를 쓰면 관리자 계정 하이브에 들어간다)

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

; 셸 확장 DLL 의 버전을 **컴파일 시점에 파일에서 직접 읽는다.** 손으로 옮겨 적으면 언젠가
; 반드시 어긋나고, 어긋나는 방향이 하필 "새 DLL 을 안 까는" 쪽이라 알아채기 어렵다.
; 버전을 고치는 곳은 shellext\ZipManiaShell.rc 한 곳뿐이다.
#define MyShellDllVersion() \
   ParseVersion(MyShellDll, Local[0], Local[1], Local[2], Local[3]), \
   Str(Local[0]) + "." + Str(Local[1]) + "." + Str(Local[2]) + "." + Str(Local[3])

; WebView2 런타임 설치. 정본은 D:\Component\InnoDependencyInstaller 이고 이 저장소로
; 복사하지 않는다 — 사본은 반드시 갈라진다. 아래 InitializeSetup 에서 호출한다.
#include "D:\Component\InnoDependencyInstaller\CodeDependencies.iss"

[Files]
Source: "{#MyAppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#MySrcDir}\7z.dll"; DestDir: "{app}"; Flags: ignoreversion
; 셸 확장 DLL 은 버전이 바뀌었을 때만 손댄다. 탐색기가 물고 있어 덮어쓰려면 탐색기를 껐다
; 켜야 하는데, 그건 작업 표시줄과 바탕화면이 사라졌다 돌아오는 눈에 띄는 동작이다.
; 내용이 같은 파일 때문에 그걸 매번 하지 않는다. (판정은 ShouldInstallShellDll)
Source: "{#MyShellDll}"; DestDir: "{app}"; Flags: ignoreversion; Check: ShouldInstallShellDll

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

; **설치 폴더는 사용자 폴더인데 권한은 관리자다.** 둘이 짝이 안 맞아 보이지만 의도한 것이다.
; 승격이 필요한 이유는 WebView2 런타임을 시스템에 깔기 때문이고, 폴더가 사용자 것인 이유는
; 앱이 자기 옆에 settings.toml 을 쓰기 때문이다(포터블 배포본과 같은 구조).
; 한계: 표준 사용자가 다른 관리자 계정 암호로 승격하면 그 관리자의 폴더가 된다.
; 승격과 사용자별 폴더를 함께 쓰는 한 피할 수 없다(1인 PC 를 전제한 결정이다).
; 그래서 "관리자인데 사용자 영역을 쓴다"는 경고가 나오는데, 알고 하는 것이라 끈다.
UsedUserAreasWarning=no
DefaultDirName={localappdata}\{#MyAppName}
DefaultGroupName={cm:MyAppName}
OutputDir="Z:\Release"
; **버전 없는 고정 이름으로 나간다** — 다른 제품과 같은 규칙(내려받기 주소가 버전을 타지 않게).
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
Name: en; MessagesFile: "compiler:Default.isl"
Name: ko; MessagesFile: "compiler:Languages\Korean.isl"

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
; 셸 확장·파일 연결 등록. **반드시 사용자 권한(runasoriginaluser)으로** 부른다 —
; 이 설치 프로그램은 승격돼 있고, 승격된 프로세스가 HKCU 를 쓰면 관리자 계정 하이브에
; 써 버려 "설치했는데 우클릭 메뉴가 없다"가 된다. 원인을 찾기 매우 어려운 종류다.
Filename: "{app}\{#MyAppName}.exe"; Parameters: "/inst"; \
  StatusMsg: "{cm:RegisteringShell}"; Flags: runasoriginaluser runhidden waituntilterminated
Filename: "{app}\{#MyAppName}.exe"; Flags: nowait postinstall skipifsilent runasoriginaluser; \
  Description: "{cm:MyAppName}"

[UninstallDelete]
; settings.toml 은 설치 폴더 안에 있다(포터블 배포본과 같은 구조 — settings.rs 의 settings_file).
; 지울 때 폴더째 치운다.
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

// 잠금 확인. **버전만 봐서는 부족하다** — 새 버전이어도 탐색기가 아직 안 물고 있으면
// (처음 설치, 또는 우클릭 메뉴를 한 번도 안 쓴 경우) 탐색기를 건드릴 이유가 없다.
// 쓰기 모드로 열어 보는 것이 유일하게 확실한 판정이다. 빈 문자열을 덧붙이므로 내용은 그대로다.
function CanWriteFile(const FileName: String): Boolean;
begin
  Result := SaveStringToFile(FileName, '', True);
end;

function ShellDllPath: String;
begin
  Result := ExpandConstant('{app}\') + SHELL_DLL;
end;

// 설치된 DLL 의 버전. 없거나 버전 리소스가 없으면 빈 문자열.
function InstalledShellDllVersion: String;
begin
  if not GetVersionNumbersString(ShellDllPath, Result) then
    Result := '';
end;

// [Files] 의 Check. 버전이 같으면 아예 복사하지 않는다.
// Inno 는 Check 를 여러 번 부를 수 있으므로 판정을 한 번만 하고 기억한다 —
// 첫 호출 시점 이후에는 파일이 이미 바뀌어 있을 수 있다.
function ShouldInstallShellDll: Boolean;
begin
  if not ShellDllChecked then begin
    ShellDllNeeded := InstalledShellDllVersion <> '{#MyShellDllVersion}';
    ShellDllChecked := True;
  end;
  Result := ShellDllNeeded;
end;

// 탐색기가 물고 있는 DLL 의 자리를 비운다. True = 비웠다.
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

  // **taskkill 이 돌아왔다고 잠금이 풀린 것은 아니다.** 프로세스가 완전히 사라지고 DLL
  // 매핑이 해제되기까지 시간이 더 걸린다. 고정 시간을 기다리면 빠른 PC 에서는 낭비고
  // 느린 PC 에서는 모자라니, 열릴 때까지 확인하며 기다린다(0.25초 간격, 최대 약 5초).
  for Loop := 0 to 19 do begin
    Sleep(250);
    if CanWriteFile(Path) then
      exit;
  end;

  Result := False;
end;

// 우리가 껐을 때만, 그리고 **아직 안 떠 있을 때만** 다시 켠다.
// 윈도우는 셸이 죽으면 스스로 다시 띄우는 설정(AutoRestartShell)이 기본이라, 확인 없이
// 실행하면 이미 떠 있는 셸 위에 탐색기 창이 하나 뜬다.
// 작업 표시줄(Shell_TrayWnd)이 있으면 셸이 살아 있는 것이다.
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

// 자리를 비우고, 끝내 안 풀리면 옆으로 치운다.
// **로드된 DLL 도 이름은 바꿀 수 있다.** 치운 파일은 다음 설치나 제거 때 지운다.
procedure PrepareShellDllSlot;
var
  OldPath: String;
begin
  OldPath := ExpandConstant('{app}\') + SHELL_DLL_OLD;
  DeleteFile(OldPath);

  if not ReleaseShellDll then
    RenameFile(ShellDllPath, OldPath);
end;

// ── 설치 ─────────────────────────────────────────────────────────────────────

function InitializeSetup: Boolean;
begin
  // WebView2 런타임. 이미 있으면 아무 일도 하지 않는다(HKLM/HKCU 양쪽을 본다).
  Dependency_AddWebView2;
  Result := True;
end;

// CodeDependencies.iss 도 PrepareToInstall 을 쓰지만 event 특성으로 등록돼 있어
// 이 함수와 함께 불린다(의존성 설치가 먼저 끝난다).
function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := '';

  // 실행 중이어도 **묻지 않고** 종료시킨다. 설치를 시작한 시점에 의사는 이미 분명하다.
  TaskKill('ZipMania.exe');

  // 셸 확장을 갈아 끼울 때만 탐색기를 건드린다.
  if ShouldInstallShellDll then begin
    WizardForm.StatusLabel.Caption := ExpandConstant('{cm:RestartingExplorer}');
    PrepareShellDllSlot;
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  // 파일 복사가 끝난 뒤에 되돌린다. [Run] 의 /inst 보다 먼저 와야 탐색기가
  // 새 DLL 을 처음부터 로드한다.
  if CurStep = ssPostInstall then
    RestartExplorer;
end;

// ── 제거 ─────────────────────────────────────────────────────────────────────

function InitializeUninstall: Boolean;
var
  ErrorCode: Integer;
begin
  Result := True;

  TaskKill('ZipMania.exe');

  // **파일을 지우기 전에** 등록을 해제해야 한다. exe 가 사라지면 되돌릴 방법이 없어져,
  // 없는 프로그램을 가리키는 파일 연결과 셸 확장이 그대로 남는다. 파일 연결의 경우
  // UserChoice 가 우리를 가리킨 채 대상이 사라지므로 **등록 전보다 나쁜 상태**가 된다.
  //
  // **여기서는 ExecAsOriginalUser 를 쓸 수 없다.** 제거 과정에서 부르면 컴파일은 통과하고
  // 실행 중에 "Cannot call ExecAsOriginalUser function during Uninstall" 로 죽는다.
  // (설치 쪽 [Run] 의 runasoriginaluser 는 그대로 쓸 수 있다 — 그건 Setup 이 부른다.)
  //
  // 그래서 그냥 Exec 다. 제거기는 승격돼 있으므로 승격한 계정의 HKCU 를 손대게 되는데,
  // **설치 폴더가 이미 그 계정의 %LOCALAPPDATA% 이므로 앞뒤가 맞는다.** 어긋나는 경우는
  // 표준 사용자가 다른 관리자 계정 암호로 승격했을 때뿐이고, 그건 설치할 때부터 이미
  // 그 관리자의 폴더에 깔린다(1인 PC 를 전제한 결정 — [Setup] 의 DefaultDirName 주석 참조).
  Exec(ExpandConstant('{app}\ZipMania.exe'), '/uninst', '',
       SW_HIDE, ewWaitUntilTerminated, ErrorCode);
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  // 등록을 지워도 탐색기가 이미 로드한 DLL 은 계속 물고 있다. 잠겨 있으면 파일이 지워지지
  // 않아 폴더가 남으므로, 여기서 자리를 비운다(제거할 때는 버전을 보지 않는다 — 무조건 지운다).
  if CurUninstallStep = usUninstall then
    PrepareShellDllSlot;

  if CurUninstallStep = usPostUninstall then
    RestartExplorer;
end;
