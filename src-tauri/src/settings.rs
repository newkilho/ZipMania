//! 설정 TOML 로드, 저장(exe 옆 settings.toml), 테마, 언어, 해제 기본값, 최근 파일
//! 없음과 판독 실패를 구분해 통지(load_at_checked)

#![allow(dead_code)]

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

/// 앱 설정
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: String,
    pub language: String,
    pub extract_create_subfolder: bool,
    pub extract_delete_after: bool,
    pub extract_auto_close: bool,
    pub extract_open_folder: bool,
    pub shell_integration: bool,
    pub file_assoc: Vec<String>,
    pub file_assoc_initialized: bool,
    pub assoc_banner_dismissed: bool,
    pub recent_files: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            language: "system".into(),
            extract_create_subfolder: true,
            extract_delete_after: false,
            extract_auto_close: false,
            extract_open_folder: false,
            shell_integration: false,
            file_assoc: Vec::new(),
            file_assoc_initialized: false,
            assoc_banner_dismissed: false,
            recent_files: Vec::new(),
        }
    }
}

/// 앱 식별자 — tauri.conf.json, 셸 확장과 동일 필요(설정 파일 경로에는 미사용)
pub const IDENTIFIER: &str = "net.kilho.zipmania";

/// 설정 파일 경로 = exe 옆 settings.toml(%APPDATA% 아님), 셸 확장 C++ 도 같은 규칙 — 아래 테스트가 대조
fn settings_file() -> Result<PathBuf, String> {
    let exe =
        std::env::current_exe().map_err(|e| format!("실행 파일 경로를 찾지 못했습니다: {e}"))?;
    let dir = exe.parent().ok_or_else(|| "실행 파일 폴더를 찾지 못했습니다".to_string())?;
    Ok(dir.join("settings.toml"))
}

/// 설정 파일 경로, AppHandle 은 쓰지 않지만 호출부 유지
fn settings_path(_app: &AppHandle) -> Result<PathBuf, String> {
    settings_file()
}

/// AppHandle 없이 같은 경로(설치/제거 훅 전용)
#[cfg(windows)]
pub fn settings_path_headless() -> Result<PathBuf, String> {
    settings_file()
}

/// 설정 읽기, 없거나 파싱 실패 = 기본값
pub fn load(app: &AppHandle) -> Settings {
    load_checked(app).0
}

/// 값 + 믿어도 되는지(load_at_checked 참조)
pub fn load_checked(app: &AppHandle) -> (Settings, bool) {
    match settings_path(app) {
        Ok(p) => load_at_checked(&p),
        Err(_) => (Settings::default(), true),
    }
}

/// 경로 지정 읽기
pub fn load_at(path: &std::path::Path) -> Settings {
    load_at_checked(path).0
}

/// 값 + 신뢰 여부, false = 파일은 있는데 읽지 못함 → 그 기본값으로 동기화 금지, 없는 파일만 true
pub fn load_at_checked(path: &std::path::Path) -> (Settings, bool) {
    match fs::read_to_string(path) {
        Ok(text) => match toml::from_str(&text) {
            Ok(s) => (s, true),
            Err(_) => (Settings::default(), false),
        },
        // 없음만 신뢰, 권한 오류와 잠김은 첫 실행 아님
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Settings::default(), true),
        Err(_) => (Settings::default(), false),
    }
}

/// 설정 저장
pub fn save(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    save_at(&settings_path(app)?, settings)
}

/// 경로 지정 저장, 같은 폴더 임시 파일 → rename, fs::write 금지(자른 뒤 기록)
pub fn save_at(path: &std::path::Path, settings: &Settings) -> Result<(), String> {
    let text = toml::to_string_pretty(settings).map_err(|e| format!("설정 직렬화 실패: {e}"))?;

    // 임시 파일은 같은 폴더 필수 — rename 은 볼륨 초과 불가
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, text).map_err(|e| format!("설정 저장 실패: {e}"))?;
    // Windows 의 rename = 기존 파일 덮어쓰기, 실패 시 임시 파일 정리 + 원본 유지
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("설정 저장 실패: {e}")
    })
}

#[cfg(test)]
mod tests {
    /// 깨진 설정 파일은 "기본값"이 아니라 "모른다"다(없는 파일과 읽지 못한 파일의 구분)
    #[test]
    fn 깨진_설정은_믿을_수_없다고_알린다() {
        let mut dir = std::env::temp_dir();
        dir.push(format!("zipmania_settings_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("settings.toml");

        // 없는 파일 = 기본값이 정답
        let _ = std::fs::remove_file(&path);
        assert!(super::load_at_checked(&path).1, "없는 파일을 못 믿는다고 했다");

        // 정상 파일: 믿는다
        std::fs::write(&path, "language = \"ko\"\n").unwrap();
        assert!(super::load_at_checked(&path).1, "정상 파일을 못 믿는다고 했다");

        // 잘린 파일 = 신뢰 금지
        std::fs::write(&path, "language = \"ko\"\nfile_assoc = [\"zi").unwrap();
        let (s, trusted) = super::load_at_checked(&path);
        assert!(!trusted, "깨진 설정을 믿는다고 했다 — 파일 연결이 통째로 해제된다");
        assert!(s.file_assoc.is_empty(), "깨진 설정에서 값을 건져 올렸다");

        // 읽지 못한 경우(권한, 잠김)도 "없음"이 아니다 — 폴더를 파일 자리에 두어 흉내 낸다
        let blocked = dir.join("blocked.toml");
        std::fs::create_dir_all(&blocked).unwrap();
        assert!(
            !super::load_at_checked(&blocked).1,
            "읽지 못한 설정을 첫 실행으로 취급했다"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// IDENTIFIER 가 tauri.conf.json 과 어긋나면 설정 폴더가 갈라진다
    #[test]
    fn 식별자가_tauri_conf_와_같다() {
        let conf = include_str!("../tauri.conf.json");
        let want = format!("\"identifier\": \"{}\"", super::IDENTIFIER);
        assert!(
            conf.contains(&want),
            "tauri.conf.json 의 identifier 가 settings::IDENTIFIER({}) 와 다르다",
            super::IDENTIFIER
        );
    }

    /// 셸 확장(C++)도 같은 규칙으로 설정 파일 탐색 — 실행 파일 옆의 settings.toml
    #[test]
    fn 셸확장도_실행파일_옆_설정을_읽는다() {
        let src = include_str!("../../shellext/ZipManiaShell.cpp");
        assert!(
            src.contains(r#"ParentDir(ExePath()) + L"\\settings.toml""#),
            "ZipManiaShell.cpp 가 실행 파일 옆의 settings.toml 을 읽지 않는다"
        );
        assert!(
            !src.contains("RoamingAppData"),
            "ZipManiaShell.cpp 에 옛 %APPDATA% 경로가 남아 있다"
        );
    }

    /// 저장 = 임시 파일 → rename, .tmp 미잔존 + 내용 온전 필요
    #[test]
    fn 저장은_임시파일을_남기지_않는다() {
        let dir = std::env::temp_dir().join(format!("zipmania_set_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.toml");

        let mut s = super::Settings::default();
        s.language = "ko".into();
        s.file_assoc = vec!["zip".into(), "7z".into()];
        super::save_at(&path, &s).expect("저장 실패");

        assert!(path.is_file(), "설정 파일이 없다");
        assert!(!dir.join("settings.toml.tmp").exists(), "임시 파일이 남았다");
        let back = super::load_at(&path);
        assert_eq!(back.language, "ko");
        assert_eq!(back.file_assoc, vec!["zip".to_string(), "7z".to_string()]);

        // 덮어쓰기도 같은 경로(기존 파일 존재 시에도 rename 이 대체)
        s.language = "en".into();
        super::save_at(&path, &s).expect("덮어쓰기 실패");
        assert_eq!(super::load_at(&path).language, "en");
        assert!(!dir.join("settings.toml.tmp").exists(), "임시 파일이 남았다");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
