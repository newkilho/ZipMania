//! 설치/제거 훅 headless 진입점(/inst, /uninst), Tauri 전에 창 없이 레지스트리만 처리
//! 스위치 이름은 ZipMania.iss 와 동일 필요(아래 테스트가 대조)
//! 셸 확장 = 항상 등록, 파일 연결 = file_assoc_initialized 거짓일 때만 DEFAULT_ASSOC_EXTS (D4, I2)

use std::path::PathBuf;

/// argv 의 유지보수 스위치, Some(true)=등록, Some(false)=해제, None=일반 실행
/// run 과 분리 = 부작용 없이 시험하기 위함
fn switch_of(argv: &[String]) -> Option<bool> {
    argv.iter().skip(1).find_map(|a| {
        if a.eq_ignore_ascii_case("/inst") {
            Some(true)
        } else if a.eq_ignore_ascii_case("/uninst") {
            Some(false)
        } else {
            None
        }
    })
}

/// 유지보수 스위치가 있으면 처리 후 종료 코드 반환, 없으면 None
pub fn run(argv: &[String]) -> Option<i32> {
    let register = switch_of(argv)?;
    Some(match apply(register) {
        Ok(()) => 0,
        Err(e) => {
            // 설치의 실패 표시 금지, 등록은 앱 시작 시 재시도
            eprintln!("[maintenance] {e}");
            0
        }
    })
}

#[cfg(windows)]
fn apply(register: bool) -> Result<(), String> {
    let path = crate::settings::settings_path_headless()?;
    let (mut settings, trusted) = crate::settings::load_at_checked(&path);
    // 읽지 못한 설정을 기본값으로 덮지 않는다 — .bad 로 치우고 새 설치처럼 진행
    // 옮기지 못했으면 설정은 두고 셸 확장만 등록(D3.5)
    let mut settings_usable = true;
    if !trusted {
        let bad = path.with_extension("toml.bad");
        match std::fs::rename(&path, &bad) {
            Ok(()) => settings = crate::settings::Settings::default(),
            Err(e) => {
                eprintln!("[maintenance] 손상된 설정을 옮기지 못했습니다: {e}");
                settings_usable = false;
            }
        }
    }
    let lang = crate::update::language_from(&settings.language);

    if register {
        // 셸 확장은 설정과 무관하게 등록
        crate::shell_reg::register(&dll_path()).map_err(|e| format!("셸 확장 등록 실패: {e}"))?;
        if !settings_usable {
            eprintln!("[maintenance] 설정을 읽지도 옮기지도 못해 파일 연결은 건너뜁니다.");
            return Ok(());
        }

        settings.shell_integration = true;
        // 최초 설치에서만 기본 연결 주입, 이후에는 사용자 지정 목록 그대로
        if !settings.file_assoc_initialized {
            settings.file_assoc = crate::file_assoc::DEFAULT_ASSOC_EXTS
                .iter()
                .map(|s| s.to_string())
                .collect();
            settings.file_assoc_initialized = true;
        }
        crate::settings::save_at(&path, &settings)?;

        crate::file_assoc::sync(&settings.file_assoc, &lang)
            .map_err(|e| format!("파일 연결 등록 실패: {e}"))?;
    } else {
        // 제거: 레지스트리 흔적만 삭제, 설정 파일은 .iss 의 UninstallDelete 담당, KUID 는 공유라 미삭제
        let _ = crate::shell_reg::unregister();
        // 빈 목록 동기화 시 우리가 잡고 있던 확장자가 원래 프로그램으로 복원
        let _ = crate::file_assoc::sync(&[], &lang);
    }
    Ok(())
}

#[cfg(not(windows))]
fn apply(_register: bool) -> Result<(), String> {
    Ok(())
}

/// 셸 확장 DLL = exe 와 같은 폴더에 평면 배치(포터블, 설치형 동일)
#[cfg(windows)]
fn dll_path() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(PathBuf::from))
        .unwrap_or_default()
        .join(crate::shell_reg::SHELLEXT_DLL)
        .to_string_lossy()
        .to_string()
}

#[cfg(test)]
mod tests {
    /// 스위치 부재 시 평소대로 앱 기동 필요
    #[test]
    fn 스위치가_없으면_통과시킨다() {
        let argv = vec!["ZipMania.exe".to_string(), "--open".into(), "a.zip".into()];
        assert!(super::run(&argv).is_none());
    }

    /// ZipMania.iss 가 넘기는 스위치의 파서 수용 여부 대조
    #[test]
    fn 설치_스크립트의_스위치를_받는다() {
        let iss = include_str!("../../ZipMania.iss");
        let mut found: Vec<String> = Vec::new();

        for line in iss.lines() {
            let t = line.trim_start();
            if t.starts_with(';') || t.starts_with("//") {
                continue; // 주석
            }
            if !(line.contains("ZipMania.exe") || line.contains("MyAppName}.exe")) {
                continue; // 우리 앱을 부르는 줄이 아니다
            }
            let bytes: Vec<char> = line.chars().collect();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == '/' {
                    let start = i + 1;
                    let mut j = start;
                    while j < bytes.len() && bytes[j].is_ascii_alphabetic() {
                        j += 1;
                    }
                    if j > start {
                        let tok: String = std::iter::once('/').chain(bytes[start..j].iter().copied()).collect();
                        if !found.contains(&tok) {
                            found.push(tok);
                        }
                    }
                    i = j;
                } else {
                    i += 1;
                }
            }
        }

        assert!(
            !found.is_empty(),
            ".iss 가 ZipMania.exe 에 스위치를 넘기지 않는다 — 등록이 아예 일어나지 않는다"
        );
        for sw in &found {
            let argv = vec!["ZipMania.exe".to_string(), sw.clone()];
            assert!(
                super::switch_of(&argv).is_some(),
                ".iss 는 {sw} 를 넘기는데 maintenance 가 모르는 스위치다 (아는 것: /inst, /uninst)"
            );
        }
        // 등록/해제 둘 다 호출 필요(한쪽만 남으면 제거 시 흔적 잔존)
        assert!(found.iter().any(|s| s == "/inst"), "설치 등록 호출이 없다: {found:?}");
        assert!(found.iter().any(|s| s == "/uninst"), "제거 해제 호출이 없다: {found:?}");
    }

    #[test]
    fn 스위치_해석() {
        let mk = |s: &str| vec!["ZipMania.exe".to_string(), s.to_string()];
        assert_eq!(super::switch_of(&mk("/inst")), Some(true));
        assert_eq!(super::switch_of(&mk("/UNINST")), Some(false)); // 대소문자 무시
        assert_eq!(super::switch_of(&mk("--register")), None); // 옛 이름은 더 이상 받지 않는다
        assert_eq!(super::switch_of(&mk("a.zip")), None);
    }
}
