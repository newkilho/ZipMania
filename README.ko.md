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

## 시스템 요구사항

* Windows 10 이상
* Microsoft WebView2 Runtime

## 만든 이

**Kilho.net** · [https://v2.kilho.net](https://v2.kilho.net)

## 라이선스

프로그램 자체는 **프리웨어**입니다. 회사, 집, 관공서, 학교 어디서든 제약 없이
무료로 쓸 수 있고 광고나 번들 설치가 없습니다.

소스 코드는 **Apache License 2.0** 을 따릅니다(`LICENSE`). `crates/` 의 재사용
크레이트는 **MIT 또는 Apache-2.0** 이중 라이선스입니다. "ZipMania", "집매니아"
이름과 로고, 아이콘은 Kilho.net 의 상표라 이 라이선스에 포함되지 않습니다 —
고쳐서 배포할 때는 다른 이름과 아이콘을 쓰세요.

7-Zip 의 `7z.dll`(LGPL)을 비롯한 오픈소스 구성 요소는
`THIRD-PARTY-NOTICES.txt` 에 있습니다.
