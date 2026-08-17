# ZipMania 셸 확장 (ZipManiaShell.dll)

Windows 탐색기 우클릭 메뉴(`IExplorerCommand`)를 제공하는 C++/WinRT 인프로세스 COM DLL이다.
메뉴 라벨에 실제 파일명(`사진.zip으로 압축하기`, `사진에 풀기`)을 표시한다.

- **얇은 프록시**: 선택 항목을 모아 `ZipMania.exe --<스위치> "<경로>"…` 로 실행할 뿐, 실제 압축/해제/열기는 앱(`src-tauri/src/cli.rs`)이 처리한다.
- **등록**: 앱이 HKCU 에 자체 등록한다(`src-tauri/src/shell_reg.rs`). regsvr32·관리자 권한·MSIX 불필요. 환경설정 "탐색기 메뉴" 토글로 ON/OFF.
- **exe 경로**: 앱이 등록 시 `HKCU\Software\ZipMania\ShellExt\ExePath` 에 기록 → DLL 이 읽어 실행(DLL 과 exe 가 다른 폴더여도 동작).
- **배치 범위(1단계)**: 레거시 메뉴(Win10 기본 / Win11 "더 많은 옵션 표시" 하위). Win11 기본 메뉴는 후순위(Sparse MSIX + 코드서명).

## 빌드 (스크립트 3분할)

Visual Studio **Build Tools 2022** + **Windows SDK** 만 필요하다(NuGet·WindowsAppSDK 불필요).

| 스크립트 | 역할 |
|---|---|
| `shellext\build.bat` | **DLL만** 빌드 → `src-tauri\binaries\ZipManiaShell.dll` |
| `build.bat`(루트) | **앱 exe만** 빌드(`tauri build --no-bundle`) + 포터블 폴더 `Z:\Release` 조립 |
| `debug.bat`(루트) | dev 실행(`npm run tauri dev`) |

빌드 순서: **① `shellext\build.bat` → ② `build.bat`**. (build.bat 은 `binaries\ZipManiaShell.dll` 이 있어야 tauri 리소스 번들이 통과한다.)

- DLL: `cl.exe`(MSVC) + Windows SDK 의 cppwinrt(`winrt/base.h`)·`shobjidl_core.h` 로 컴파일.
- 포터블 산출물(**평면 배치** — 하위 폴더 없이 같은 폴더): `Z:\Release\ZipMania.exe` + `Z:\Release\ZipManiaShell.dll`. **`7z.dll` 은 직접** `Z:\Release\`(ZipMania.exe 옆)에 넣는다.
- cargo 빌드 target(수 GB)은 로컬 D: 에 둔다(Z: 는 소용량이라 최종 산출물만).
- `build.bat`·`shellext\build.bat` 의 `VCVARS` 경로(`D:\BuildTools\...`)는 설치 위치에 맞게 조정한다.

## 계약(반드시 동기화)

`ZipManiaShell.cpp` 와 `src-tauri/src/shell_reg.rs` 가 아래를 **동일**하게 유지해야 한다:

- CLSID 두 개: 압축 `{B7E5C9A2-…-…C01}`, 풀기 `{…C02}`.
- 아카이브 확장자 목록(`kArchiveExts` ↔ `ARCHIVE_EXTS`).
- CLI 스위치(`--compress-zip`, `--compress`, `--extract-here`, `--extract-smart`, `--extract-newfolder`, `--extract`, `--open`) ↔ `cli.rs` 파서.

## 메뉴 구성 (최상위 평면 — "집매니아" 서브메뉴 없음)

항목마다 독립 CLSID + verb 로 등록해 우클릭 메뉴에 **바로 나열**된다(계단식 아님).

- **파일·폴더**(아카이브에선 `AppliesTo` 로 숨김): `{이름}.zip으로 압축하기`(즉시) / `집매니아로 압축하기`(창)
- **아카이브**: `여기에 풀기` / `알아서 풀기` / `{이름}에 풀기` / `집매니아로 압축 풀기`(창) / `집매니아로 열기`

> CLSID 7개(`…C01`~`…C07`)와 verb 키 이름·순서는 `ZipManiaShell.cpp` ↔ `shell_reg.rs` 가 일치해야 한다.
> 등록 구조를 바꾼 뒤에는 환경설정에서 껐다 켜(재등록) 반영한다.

## 후순위 (2단계): Win11 기본 메뉴

동일 DLL 을 Sparse MSIX 로 패키징 + 코드서명하면 Win11 기본(간단) 메뉴에 1클릭 배치된다.
CLI 스위치·앱 라우팅·DLL 로직은 그대로 재사용한다.
