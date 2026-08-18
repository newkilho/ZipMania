//! 파일 연결 등록, 앱이 HKCU 에 직접 기록(인스톨러 아님)
//! 등록 = .<ext> 기본값 + OpenWithProgids, 원래 값은 Software\ZipMania\FileAssoc 에 백업
//! (F) (D4)

#![allow(dead_code)]

/// 연결 대상 확장자 정본, 환경설정 목록 + 설치 시 기본값, 사본 = SettingsWindow.svelte 의 ASSOC_EXTS
/// READ_EXTS 와 다른 목록
pub const DEFAULT_ASSOC_EXTS: &[&str] =
    &["zip", "7z", "rar", "tar", "gz", "tgz", "bz2", "xz", "egg", "alz", "cbz"];

/// 확장자 → ProgID(ZipMania.zip)
fn prog_id(ext: &str) -> String {
    format!("ZipMania.{ext}")
}

/// 확장자 상태
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssocStatus {
    pub ext: String,
    pub registered: bool,
    pub ours: bool,
    pub other: Option<String>,
    pub hard: bool,
    pub broken: bool,
}

/// 탐색기 유형 열 이름, 등록 시점 언어로 고정, 모르는 언어는 영어
fn type_name(ext: &str, lang: &str) -> String {
    zipmania_i18n::text("assoc.typeName", lang).replace("{ext}", &ext.to_uppercase())
}

/// [기본 앱 선택] 창(IAssocHandler::GetUIName)이 보는 앱 이름, 등록 시점 언어로 고정(F)
fn app_name(lang: &str) -> String {
    zipmania_i18n::text("app.name", lang).into()
}

// ── Windows 구현 ─────────────────────────────────────────────────────────────

#[cfg(windows)]
mod imp {
    use super::{app_name, prog_id, type_name};
    use std::io;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
    use winreg::RegKey;

    /// 백업 키, 값 이름 = 확장자, 값 = 원래 ProgID
    const BACKUP_KEY: &str = r"Software\ZipMania\FileAssoc";

    fn exe_path() -> io::Result<String> {
        Ok(std::env::current_exe()?.to_string_lossy().to_string())
    }

    /// 앱 이름 기록 위치, exe 하나에 한 번 = 모든 확장자 적용
    /// <ProgID>\FriendlyAppName 은 먹지 않는다(F)
    fn app_name_key() -> io::Result<String> {
        let exe = std::env::current_exe()?;
        let file = exe
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        Ok(format!(r"Software\Classes\Applications\{file}"))
    }

    /// 앱 이름 등록
    pub fn set_app_name(lang: &str) -> io::Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (k, _) = hkcu.create_subkey(app_name_key()?)?;
        k.set_value("FriendlyAppName", &app_name(lang))?;
        Ok(())
    }

    /// 앱 이름 등록 제거, 우리 값만 지우고 키가 비면 키도 제거
    pub fn clear_app_name() {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let Ok(key) = app_name_key() else { return };
        if let Ok(k) = hkcu.open_subkey_with_flags(&key, winreg::enums::KEY_ALL_ACCESS) {
            let _ = k.delete_value("FriendlyAppName");
            if k.enum_values().next().is_none() && k.enum_keys().next().is_none() {
                drop(k);
                let _ = hkcu.delete_subkey(&key);
            }
        }
    }

    /// 우리가 잡고 있는 확장자 목록(백업 키 기준)
    pub fn registered_exts() -> Vec<String> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        match hkcu.open_subkey_with_flags(BACKUP_KEY, KEY_READ) {
            Ok(k) => k.enum_values().filter_map(|v| v.ok()).map(|(n, _)| n).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// 확장자 1개 연결
    pub fn register(ext: &str, lang: &str) -> io::Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let exe = exe_path()?;
        let pid = prog_id(ext);

        // 1) ProgID — 표시 이름, 아이콘, 실행 명령
        let (k, _) = hkcu.create_subkey(format!(r"Software\Classes\{pid}"))?;
        k.set_value("", &type_name(ext, lang))?;

        // 아이콘 = exe 에 박힌 확장자별 리소스, 전용 아이콘 없으면 기본, assoc_icon 참조
        let (icon, _) = hkcu.create_subkey(format!(r"Software\Classes\{pid}\DefaultIcon"))?;
        icon.set_value("", &crate::assoc_icon::icon_ref(&exe, ext))?;

        // --open 은 셸 통합이 이미 쓰는 스위치다(cli.rs Verb::Open)
        let (cmd, _) =
            hkcu.create_subkey(format!(r"Software\Classes\{pid}\shell\open\command"))?;
        cmd.set_value("", &format!("\"{exe}\" --open \"%1\""))?;

        // 2) 확장자 키 — 기본값을 우리 것으로, 원래 값은 되돌리기용으로 적어 둔다
        let ext_key = format!(r"Software\Classes\.{ext}");
        let previous = hkcu
            .open_subkey_with_flags(&ext_key, KEY_READ)
            .ok()
            .and_then(|k| k.get_value::<String, _>("").ok())
            .unwrap_or_default();

        // 빈 문자열 = 원래 HKCU 에 값 없음, 이미 우리 것이면 백업 덮어쓰기 금지
        if previous != pid {
            let (b, _) = hkcu.create_subkey(BACKUP_KEY)?;
            b.set_value(ext, &previous)?;
        }

        let (e, _) = hkcu.create_subkey(&ext_key)?;
        e.set_value("", &pid)?;

        // 3) 연결 프로그램 후보 등록, UserChoice 차단 시에도 목록에는 노출
        let (ow, _) = hkcu.create_subkey(format!(r"{ext_key}\OpenWithProgids"))?;
        ow.set_value(&pid, &"")?;

        Ok(())
    }

    /// 기본 앱 지정이 우리를 가리킬 때만 제거, 실패는 무시 — 화면이 broken 으로 포착
    fn clear_user_choice(ext: &str, pid: &str) {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let base = format!(r"{FILE_EXTS}\.{ext}");

        let points_to_us = |path: &str| -> bool {
            hkcu.open_subkey_with_flags(path, KEY_READ)
                .ok()
                .and_then(|k| k.get_value::<String, _>("ProgId").ok())
                .map(|v| v.eq_ignore_ascii_case(pid))
                .unwrap_or(false)
        };

        // Win11 은 ProgId 가 하위 키, 하나라도 우리를 가리키면 키째 제거
        if points_to_us(&format!(r"{base}\UserChoiceLatest\ProgId"))
            || points_to_us(&format!(r"{base}\UserChoiceLatest"))
        {
            let _ = hkcu.delete_subkey_all(format!(r"{base}\UserChoiceLatest"));
        }
        if points_to_us(&format!(r"{base}\UserChoice")) {
            let _ = hkcu.delete_subkey_all(format!(r"{base}\UserChoice"));
        }
    }

    /// 확장자 1개 연결 해제 + 원래 프로그램 복원
    pub fn unregister(ext: &str) -> io::Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let pid = prog_id(ext);
        let ext_key = format!(r"Software\Classes\.{ext}");

        // ProgID 삭제 전에 기본 앱 지정 해제, 순서 지킬 것
        clear_user_choice(ext, &pid);

        let backup = hkcu.open_subkey_with_flags(BACKUP_KEY, KEY_READ).ok();
        let previous: String = backup
            .as_ref()
            .and_then(|k| k.get_value(ext).ok())
            .unwrap_or_default();

        if let Ok(k) = hkcu.open_subkey_with_flags(&ext_key, winreg::enums::KEY_ALL_ACCESS) {
                    // 기본값이 우리 것일 때만 손댄다
            let current: String = k.get_value("").unwrap_or_default();
            if current == pid {
                if previous.is_empty() {
                    // 원래 값 없었음 → 삭제, 빈 문자열은 앱을 선택하세요를 부른다
                    let _ = k.delete_value("");
                } else {
                    let _ = k.set_value("", &previous);
                }
            }
            if let Ok(ow) =
                k.open_subkey_with_flags("OpenWithProgids", winreg::enums::KEY_ALL_ACCESS)
            {
                let _ = ow.delete_value(&pid);
                // 우리만 있던 목록이면 빈 키를 남기지 않는다
                if ow.enum_values().next().is_none() {
                    drop(ow);
                    let _ = k.delete_subkey("OpenWithProgids");
                }
            }
        }

        // 확장자 키가 완전히 비면 통째로 제거, 빈 껍데기는 다른 프로그램 판정에 간섭
        if let Ok(k) = hkcu.open_subkey_with_flags(&ext_key, KEY_READ) {
            if k.enum_values().next().is_none() && k.enum_keys().next().is_none() {
                drop(k);
                let _ = hkcu.delete_subkey(&ext_key);
            }
        }

        let _ = hkcu.delete_subkey_all(format!(r"Software\Classes\{pid}"));
        if let Ok(b) = hkcu.open_subkey_with_flags(BACKUP_KEY, winreg::enums::KEY_ALL_ACCESS) {
            let _ = b.delete_value(ext);
        }
        Ok(())
    }

    /// 탐색기 기본 앱이 기록되는 곳
    const FILE_EXTS: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts";

    /// 사용자가 고른 기본 앱 ProgID(없으면 None), Win11 = UserChoiceLatest, ProgId 가 하위 키
    fn user_choice_progid(ext: &str) -> Option<String> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let base = format!(r"{FILE_EXTS}\.{ext}");
        let read = |path: String| -> Option<String> {
            hkcu.open_subkey_with_flags(path, KEY_READ)
                .ok()
                .and_then(|k| k.get_value::<String, _>("ProgId").ok())
                .filter(|v| !v.trim().is_empty())
        };

        read(format!(r"{base}\UserChoiceLatest\ProgId"))
            // 변형 대비: UserChoiceLatest 자체에 값이 있는 경우
            .or_else(|| read(format!(r"{base}\UserChoiceLatest")))
            // 옛 윈도우
            .or_else(|| read(format!(r"{base}\UserChoice")))
    }

    /// 확장자 클래스 연결, HKCU → HKCR 순
    fn class_progid(ext: &str) -> String {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok(k) = hkcu.open_subkey_with_flags(format!(r"Software\Classes\.{ext}"), KEY_READ) {
            if let Ok(v) = k.get_value::<String, _>("") {
                if !v.trim().is_empty() {
                    return v.trim().to_string();
                }
            }
        }
        RegKey::predef(winreg::enums::HKEY_CLASSES_ROOT)
            .open_subkey_with_flags(format!(".{ext}"), KEY_READ)
            .and_then(|k| k.get_value::<String, _>(""))
            .map(|v| v.trim().to_string())
            .unwrap_or_default()
    }

    /// 마지막으로 연 exe 이름(FileExts MRU), UserChoice 없을 때 클래스 기본값을 덮는다
    fn open_with_mru_exe(ext: &str) -> Option<String> {
        let k = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(format!(r"{FILE_EXTS}\.{ext}\OpenWithList"), KEY_READ)
            .ok()?;
        let mru: String = k.get_value("MRUList").ok()?;
        let first = mru.chars().next()?;
        let exe: String = k.get_value(first.to_string()).ok()?;
        let exe = exe.trim();

        // 셸 자신의 "다른 앱 선택"({CLSID}\OpenWith.exe)은 프로그램이 아니다
        if exe.is_empty() || exe.to_lowercase().ends_with("openwith.exe") {
            return None;
        }
        Some(exe.to_string())
    }

    /// 이 앱의 exe 파일 이름, MRU 대조용
    fn our_exe_name() -> String {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_default()
    }

    /// 확장자 실제 상태, 우선순위 = UserChoice(Latest) > OpenWithList MRU > 클래스 연결
    /// 레지스트리 값만 읽는다, AssocQueryStringW 금지(캐시됨), (D4)
    pub fn status(ext: &str) -> super::AssocStatus {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let pid = prog_id(ext);

        let registered = hkcu
            .open_subkey_with_flags(format!(r"Software\Classes\.{ext}"), KEY_READ)
            .ok()
            .and_then(|k| k.get_value::<String, _>("").ok())
            .map(|v| v == pid)
            .unwrap_or(false);

        // 우리 ProgID 실재 여부, 부재 시 기본 앱이 우리여도 열기 불가
        let progid_exists = hkcu
            .open_subkey_with_flags(format!(r"Software\Classes\{pid}\shell\open\command"), KEY_READ)
            .is_ok();
        let mut broken = false;

        let (ours, other, hard) = match user_choice_progid(ext) {
            // 명시 지정 최우선, 우리가 아니면 등록으로 역전 불가
            Some(choice) => {
                let ours = choice.eq_ignore_ascii_case(&pid);
                broken = ours && !progid_exists;
                let other = (!ours).then(|| display_name(&choice));
                (ours, other, !ours)
            }
            // 명시 지정 없으면 사용 이력, 등록으로는 못 이겨 [강제설정] 필요
            None => match open_with_mru_exe(ext) {
                Some(exe) => {
                    let ours = exe.eq_ignore_ascii_case(&our_exe_name());
                    let other = (!ours).then(|| app_display_name(&exe));
                    (ours, other, !ours)
                }
                // 이력도 없으면 클래스 연결, 등록만으로 획득 가능
                None => {
                    let class = class_progid(ext);
                    let ours = class.eq_ignore_ascii_case(&pid);
                    let other = (!ours && !class.is_empty()).then(|| display_name(&class));
                    (ours, other, false)
                }
            },
        };

        super::AssocStatus { ext: ext.to_string(), registered, ours, other, hard, broken }
    }

    /// ProgID 보유 프로그램 이름, 실행 명령의 exe → 버전 리소스
    /// 순서: ProductName → FileDescription → exe 파일 이름 → ProgID
    fn display_name(prog_id: &str) -> String {
        open_command_exe(prog_id)
            .and_then(|exe| exe_product_name(&exe))
            .unwrap_or_else(|| prog_id.to_string())
    }

    /// exe 이름만 아는 경우의 프로그램 이름, HKCR\Applications\<exe> + App Paths\<exe> 조회
    fn app_display_name(exe_name: &str) -> String {
        const APP_PATHS: &str = r"Software\Microsoft\Windows\CurrentVersion\App Paths";

        let from_app_paths = || -> Option<String> {
            RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE)
                .open_subkey_with_flags(format!(r"{APP_PATHS}\{exe_name}"), KEY_READ)
                .ok()?
                .get_value::<String, _>("")
                .ok()
                .map(|v| v.trim().trim_matches('"').to_string())
                .filter(|v| !v.is_empty())
        };

        open_command_exe(&format!(r"Applications\{exe_name}"))
            .or_else(from_app_paths)
            .and_then(|exe| exe_product_name(&exe))
            .unwrap_or_else(|| {
                let name = exe_name.rsplit('\\').next().unwrap_or(exe_name);
                name.strip_suffix(".exe")
                    .or_else(|| name.strip_suffix(".EXE"))
                    .unwrap_or(name)
                    .to_string()
            })
    }

    /// shell\open\command → exe 경로
    fn open_command_exe(prog_id: &str) -> Option<String> {
        let cmd: String = RegKey::predef(winreg::enums::HKEY_CLASSES_ROOT)
            .open_subkey_with_flags(format!(r"{prog_id}\shell\open\command"), KEY_READ)
            .ok()?
            .get_value("")
            .ok()?;

        let cmd = cmd.trim();
        // 따옴표 감싼 경로가 표준, 아니면 첫 공백까지를 exe 로 판정
        let exe = if let Some(rest) = cmd.strip_prefix('"') {
            rest.split('"').next()?
        } else {
            cmd.split_whitespace().next()?
        };
        (!exe.is_empty()).then(|| exe.to_string())
    }

    /// exe 버전 리소스 ProductName → FileDescription → 파일 이름
    fn exe_product_name(exe: &str) -> Option<String> {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::{
            GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
        };

        let file_name = || {
            std::path::Path::new(exe)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .filter(|s| !s.is_empty())
        };

        let wide: Vec<u16> = std::ffi::OsStr::new(exe)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        unsafe {
            let size = GetFileVersionInfoSizeW(PCWSTR(wide.as_ptr()), None);
            if size == 0 {
                return file_name();
            }
            let mut buf = vec![0u8; size as usize];
            if GetFileVersionInfoW(
                PCWSTR(wide.as_ptr()),
                None,
                size,
                buf.as_mut_ptr() as *mut std::ffi::c_void,
            )
            .is_err()
            {
                return file_name();
            }

            // 기록 언어는 파일마다 다르다, Translation 표 첫 항목 → 없으면 흔한 조합 시도
            let mut langs: Vec<String> = Vec::new();
            let mut ptr = std::ptr::null_mut();
            let mut len = 0u32;
            let key: Vec<u16> = r"\VarFileInfo\Translation"
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            if VerQueryValueW(
                buf.as_ptr() as *const std::ffi::c_void,
                PCWSTR(key.as_ptr()),
                &mut ptr,
                &mut len,
            )
            .as_bool()
                && len >= 4
                && !ptr.is_null()
            {
                let pair = std::slice::from_raw_parts(ptr as *const u16, 2);
                langs.push(format!("{:04x}{:04x}", pair[0], pair[1]));
            }
            langs.push("040904b0".into()); // 영어(미국), 유니코드
            langs.push("041204b0".into()); // 한국어, 유니코드

            for field in ["ProductName", "FileDescription"] {
                for lang in &langs {
                    let key: Vec<u16> = format!(r"\StringFileInfo\{lang}\{field}")
                        .encode_utf16()
                        .chain(std::iter::once(0))
                        .collect();
                    let mut ptr = std::ptr::null_mut();
                    let mut len = 0u32;
                    if VerQueryValueW(
                        buf.as_ptr() as *const std::ffi::c_void,
                        PCWSTR(key.as_ptr()),
                        &mut ptr,
                        &mut len,
                    )
                    .as_bool()
                        && len > 0
                        && !ptr.is_null()
                    {
                        let s = String::from_utf16_lossy(std::slice::from_raw_parts(
                            ptr as *const u16,
                            len as usize,
                        ));
                        let s = s.trim_end_matches('\0').trim().to_string();
                        if !s.is_empty() {
                            return Some(s);
                        }
                    }
                }
            }
        }

        file_name()
    }

    /// 셸에 연결 변경 통지
    pub fn notify_shell() {
        use windows::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};
        unsafe {
            SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
        }
    }
}

// ── 공개 API ─────────────────────────────────────────────────────────────────

/// 설정 목록에 맞춰 등록 동기화, 목록에 있으면 등록, 우리 것인데 없으면 해제, lang = 표시 이름 언어
#[cfg(windows)]
pub fn sync(wanted: &[String], lang: &str) -> std::io::Result<()> {
    let current = imp::registered_exts();
    let mut changed = false;

    for ext in &current {
        if !wanted.iter().any(|w| w == ext) {
            imp::unregister(ext)?;
            changed = true;
        }
    }
    for ext in wanted {
        // 등록 상태에도 재기록 — 포터블 이동 시 exe 경로 변동
        imp::register(ext, lang)?;
        changed = true;
    }

    // 선택 창 앱 이름, exe 하나에 한 번, 실패해도 등록은 세우지 않는다
    if wanted.is_empty() {
        imp::clear_app_name();
    } else {
        let _ = imp::set_app_name(lang);
    }

    if changed {
        imp::notify_shell();
    }
    Ok(())
}

/// 현재 우리가 연결을 잡고 있는 확장자 목록
#[cfg(windows)]
pub fn registered() -> Vec<String> {
    imp::registered_exts()
}

/// 확장자 실제 상태(화면 표시용)
#[cfg(windows)]
pub fn status(exts: &[String]) -> Vec<AssocStatus> {
    exts.iter().map(|e| imp::status(e)).collect()
}

/// 셸에 연결 변경 통지, 사용자가 선택 창에서 바꾼 뒤에도 호출
#[cfg(windows)]
pub fn notify_shell() {
    imp::notify_shell();
}

// 비-Windows 스텁
#[cfg(not(windows))]
pub fn sync(_wanted: &[String], _lang: &str) -> std::io::Result<()> {
    Ok(())
}
#[cfg(not(windows))]
pub fn registered() -> Vec<String> {
    Vec::new()
}
#[cfg(not(windows))]
pub fn status(_exts: &[String]) -> Vec<AssocStatus> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::{app_name, prog_id, type_name};

    /// 정본과 프런트 사본이 어긋나면 설치 목록과 화면 체크박스가 달라진다
    #[test]
    fn 프런트_목록이_정본과_일치() {
        let src = include_str!("../../src/components/SettingsWindow.svelte");
        let line = src
            .lines()
            .find(|l| l.contains("const ASSOC_EXTS"))
            .expect("SettingsWindow.svelte 에서 ASSOC_EXTS 를 찾지 못했다");
        let body = line
            .split_once('[')
            .and_then(|(_, r)| r.split_once(']'))
            .map(|(inner, _)| inner)
            .expect("ASSOC_EXTS 배열 리터럴을 파싱하지 못했다");
        let front: Vec<String> = body
            .split(',')
            .map(|s| s.trim().trim_matches(['"', '\'']).to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let ours: Vec<String> = super::DEFAULT_ASSOC_EXTS.iter().map(|s| s.to_string()).collect();
        assert_eq!(front, ours, "프런트 ASSOC_EXTS 가 DEFAULT_ASSOC_EXTS 와 다르다");
    }

    /// 연결 목록은 전부 압축모듈이 여는 확장자여야 함
    #[test]
    fn 연결_확장자는_전부_열_수_있다() {
        for ext in super::DEFAULT_ASSOC_EXTS {
            assert!(
                zipmania_archive::READ_EXTS.contains(ext),
                "{ext} 는 READ_EXTS 에 없다 — 연결해도 열지 못한다"
            );
        }
    }

    #[test]
    fn progid_는_앱_접두사를_갖는다() {
        assert_eq!(prog_id("zip"), "ZipMania.zip");
        assert_eq!(prog_id("7z"), "ZipMania.7z");
    }

    #[test]
    fn 표시_이름은_언어를_따른다() {
        assert_eq!(type_name("zip", "ko"), "ZIP 압축 파일");
        assert_eq!(type_name("zip", "en"), "ZIP Archive");
        assert_eq!(type_name("zip", "ja"), "ZIP アーカイブ");
        assert_eq!(type_name("zip", "zh"), "ZIP 压缩文件");
        // 모르는 언어는 영어로 떨어진다(빈 문자열 포함)
        assert_eq!(type_name("egg", ""), "EGG Archive");
    }

    /// 표에서 빠진 언어는 조용히 영어가 된다, 그걸 잡는다
    #[test]
    fn 지원_언어는_영어로_떨어지지_않는다() {
        for lang in ["ko", "ja", "zh", "ru", "it", "fr", "es", "ar"] {
            assert_ne!(
                type_name("zip", lang),
                type_name("zip", "xx"),
                "{lang} 유형 이름이 없다"
            );
        }
    }

    /// 앱 이름과 유형 이름은 다른 값
    #[test]
    fn 앱_이름은_유형_이름과_다르다() {
        assert_eq!(app_name("ko"), "집매니아");
        assert_eq!(app_name("en"), "ZipMania");
        assert_eq!(app_name(""), "ZipMania");
        assert_ne!(app_name("ko"), type_name("zip", "ko"));
    }
}
