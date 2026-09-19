**한국어** | [English](README.md) | [日本語](README.ja.md) | [简体中文](README.zh-CN.md)

# ZipMania

![Platform](https://img.shields.io/badge/platform-Windows-blue)
![Formats](https://img.shields.io/badge/read-50%2B_formats-green)
![License](https://img.shields.io/badge/license-Apache_2.0-lightgrey)

50여 가지 압축 파일을 열고 **7Z · ZIP · TAR**로 압축하는 Windows용 압축 프로그램.
빠른 ZIP 처리와 미리보기, 탐색기 우클릭 메뉴를 지원합니다.

## 기능

* ZIP, RAR, 7Z, EGG, ALZ, ISO 등 **50여 가지 형식 열기**
* **7Z · ZIP · TAR** 압축
* 이미지·텍스트를 압축 해제 없이 바로 미리보기
* 필요한 파일만 선택해서 압축 해제
* 압축 파일 안의 압축 파일 바로 열기
* 암호 및 분할 압축
* 명령줄 압축·해제(창 또는 콘솔, 토탈 커맨더 등 외부 호출)
* 압축 후 무결성 검사·원본 삭제
* 압축 파일 추가·삭제 및 무결성 검사
* Windows 보안 검사(AMSI)
* 탐색기 우클릭 메뉴 및 파일 연결
* 다크/라이트 테마
* **9개 언어 지원**

## 빠른 ZIP 처리

ZIP 전용 엔진을 사용해 빠르게 처리합니다.

| 작업            |   7z.dll 대비 |
| ------------- | ----------: |
| 큰 파일 압축       | **최대 7.6×** |
| 작은 파일 여러 개 압축 | **최대 6.5×** |
| 압축 해제         | **최대 2.2×** |

## 지원 형식

**열기:** 7Z, ZIP, ZIPX, JAR, RAR, EGG, ALZ, TAR, GZ, BZ2, XZ, ZST, ISO, IMG, WIM, DMG, MSI, RPM, DEB, CBZ, CBR 등 50여 종

**압축:** `7z` · `zip` · `tar`

## 명령줄

압축·해제·목록·검사를 명령줄로 하는 기능입니다. Bandizip 과 7-Zip 의 옵션 표기를 둘 다 받고, 실행 파일이 둘입니다 — 반디집의 `Bandizip.exe`/`bc.exe`, 7-Zip 의 `7zG.exe`/`7z.exe` 와 같은 나눔입니다.

* **`ZipMania.exe <명령> …`** — 진행을 **창**으로 보여 줍니다. 토탈 커맨더, 바로 가기처럼 콘솔이 없는 곳에 맞습니다. 압축·해제는 진행 창, 목록·검사는 메인 창에 그 아카이브가 열립니다.
* **`zm.exe <명령> …`** — **콘솔**에 출력합니다. cmd·스크립트가 끝날 때까지 기다리고 종료 코드(0/1/2)를 받습니다. `zm` 에 명령이 아닌 것을 주면 오류입니다.

`ZipMania.exe <파일>` 은 그 파일을 창으로 엽니다.

### 사용법

```
<exe> a|c [옵션] <아카이브> <입력...>     압축 / 추가
<exe> x|e [옵션] <아카이브> [항목...]     해제(x 경로 유지, e 평면)
<exe> bx  [옵션] <아카이브...>            아카이브마다 자기 이름 폴더에 해제
<exe> l   [옵션] <아카이브>               목록
<exe> t   [옵션] <아카이브>               무결성 검사
```

`<exe>` 는 `ZipMania.exe`(진행 창) 또는 `zm`(콘솔)이고 매개변수는 둘이 같습니다. 옵션은 `-이름:값`(Bandizip) 과 `-이름값`(7-Zip) 둘 다 되고, 아카이브 뒤의 항목은 압축이면 입력 파일·폴더, 해제면 꺼낼 항목(생략 = 전부)입니다. `zm` 만 치면 도움말이 나옵니다.

### 예

```
ZipMania.exe c -l:9 -fmt:7z -testdst -delsrc -date "backup_%y%m%d_%H%M.7z" "D:\Work"        ← 진행 창으로, 검사 후 원본 삭제
zm c -l:9 -fmt:7z -v:4GB -aou -testdst -delsrc -date "backup_%y%m%d_%H%M.7z" "D:\Work"   ← 같은 일을 콘솔로, 분할 7z
zm a -mx9 -psecret backup.7z D:\Work                                              ← 7-Zip 표기, 기존 아카이브에 추가
zm x -o:D:\Out -target:auto backup.7z                                             ← 아카이브 이름 폴더로 해제
zm e -y backup.zip docs\readme.txt                                                  ← 항목 하나만 경로 없이, 덮어씀
zm bx a.zip b.7z                                                                       ← 각각 a\, b\ 폴더에 해제
zm l backup.7z.001                                                                     ← 분할 첫 볼륨 목록
```

| 동사 | 뜻 |
| --- | --- |
| `a` / `c` | 압축(`a` 는 기존 아카이브가 있으면 추가) |
| `x` / `e` | 해제(`e` 는 폴더 구조 없이) |
| `bx` | 아카이브 여러 개를 각각 자기 이름 폴더에 해제(Bandizip 의 `bx`, `-target:name` 기본) |
| `l` / `t` | 목록 / 무결성 검사 |

| 옵션 | 뜻 |
| --- | --- |
| `-l:0..9` `-mx9` | 압축 레벨 |
| `-fmt:zip\|7z\|tar` `-t7z` | 형식(생략하면 아카이브 확장자) |
| `-v:700M` `-v700m` | 분할 크기(K/M/G) |
| `-p:암호` `-p암호` | 암호 |
| `-t:N` `-mmt=N` | 스레드 수(7z) |
| `-o:폴더` `-o폴더` | 해제 대상 폴더(기본 현재 폴더) |
| `-target:auto\|name\|none` | 아카이브 이름의 하위 폴더에 해제(`auto` = 최상위 항목이 둘 이상일 때만) |
| `-aoa` `-y` / `-aos` / `-aou` | 같은 이름이 있으면 덮어씀 / 건너뜀 / `이름 (2)` 로(압축 기본 덮어씀, 해제 기본 건너뜀) |
| `-testdst` | 압축 후 무결성 검사 |
| `-delsrc` `-sdel` | 검사를 통과하면 원본 삭제 |
| `-date` | 파일 이름의 `%Y %y %m %d %H %M %S` 를 현재 시각으로 |
| `-r` `-bd` `-bb*` `-bs*` `-cp:*` | 무시(7-Zip·Bandizip 호환용) |

`ZipMania.exe` 로 창을 띄울 때는 `-t:<n>`(스레드)을 무시하고, 해제의 덮어쓰기(`-aoa`/`-aos`/`-aou`)와 암호는 창이 묻습니다. 기존 아카이브에 추가(`a`)는 `zm` 만 됩니다.

종료 코드는 0(성공) / 1(경고: 빠진 항목, 검사 실패, 건너뜀) / 2(오류) 입니다. `.bat` 파일 안에서는 `%` 를 `%%` 로 씁니다(cmd 프롬프트·PowerShell·토탈 커맨더에서는 그대로).

토탈 커맨더 사용자 명령 예 — 명령 `ZipMania.exe`(진행 창) 또는 `zm.exe`(콘솔 창), 매개변수 `c -l:9 -fmt:7z -aou -testdst -delsrc -date "%T%S %y%m%d_%H%M".7z "%P%S"` (선택한 항목을 대상 창 폴더에 날짜 이름 7z 로).

## 시스템 요구사항

* Windows 10 이상
* Microsoft WebView2 Runtime

## 만든 이

**Kilho.net** · [https://v2.kilho.net/zipmania](https://v2.kilho.net/zipmania)

생애 첫 바이브 코딩으로 만든 프로그램입니다. 의견과 제안은 언제나 환영합니다.

## 라이선스

프로그램 자체는 **프리웨어**입니다. 회사, 집, 관공서, 학교 어디서든 제약 없이
무료로 쓸 수 있고 광고나 번들 설치가 없습니다.

소스 코드는 **Apache License 2.0** 을 따릅니다(`LICENSE`). `crates/` 의 재사용
크레이트는 **MIT 또는 Apache-2.0** 이중 라이선스입니다. "ZipMania", "집매니아"
이름과 로고, 아이콘은 Kilho.net 의 상표라 이 라이선스에 포함되지 않습니다 —
고쳐서 배포할 때는 다른 이름과 아이콘을 쓰세요.

7-Zip 의 `7z.dll`(LGPL)을 비롯한 오픈소스 구성 요소는
`THIRD-PARTY-NOTICES.txt` 에 있습니다.
