//! 탐색기 우클릭 통합 등록, IContextMenu 셸 확장(ZipManiaShell.dll, CLSID 1개) → HKCU
//! 자리 = *\ShellEx\ContextMenuHandlers\ZipMania, Directory\ShellEx\… CLSID 는 ZipManiaShell.cpp 와 동일(D3.7)
//! 등록 판정 = CLSID InprocServer32 + 핸들러 키 2개 + DLL 파일 실재, 셋 모두
//! 메뉴 경로 선택 = sync_all 한 곳, Win11 은 스파스 MSIX, 그 아래는 클래식 HKCU
//! UAC 해제(승격 셸)는 Win11 이어도 클래식, 승격 프로세스에 패키지 신원 미부여

#![allow(dead_code)]

/// 컨텍스트 메뉴 핸들러 CLSID — ZipManiaShell.cpp 의 상수와 반드시 일치
pub const CLSID_MENU: &str = "{02BEA257-B0A9-4B99-9A99-F3F61885D771}";

/// 셸 확장 DLL 파일명
pub const SHELLEXT_DLL: &str = "ZipManiaShell.dll";

/// 셸 확장 파일이 놓이는 설치 루트 기준 하위 폴더
pub const SHELLEXT_DIR: &str = r"shell\x64";

/// 컨텍스트 메뉴 핸들러 등록 이름(ShellEx\ContextMenuHandlers 하위 키)
const HANDLER_NAME: &str = "ZipMania";

// ── Windows 구현 ─────────────────────────────────────────────────────────────

#[cfg(windows)]
mod imp {
    use super::{CLSID_MENU, HANDLER_NAME};
    use std::io;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
    use winreg::RegKey;

    // 핸들러를 붙일 셸 컨텍스트(모든 파일 + 폴더)
    fn handler_targets() -> [String; 2] {
        [
            format!(r"Software\Classes\*\ShellEx\ContextMenuHandlers\{HANDLER_NAME}"),
            format!(r"Software\Classes\Directory\ShellEx\ContextMenuHandlers\{HANDLER_NAME}"),
        ]
    }

    fn exe_path() -> io::Result<String> {
        Ok(std::env::current_exe()?.to_string_lossy().to_string())
    }

    fn delete_tree(rel: &str) {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let _ = hkcu.delete_subkey_all(rel);
    }

    /// 현재 exe 와 DLL 경로로 등록
    pub fn register(dll: &str) -> io::Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);

        // DLL 이 실행할 exe 경로 기록(DLL 과 exe 폴더가 달라도 동작)
        let (meta, _) = hkcu.create_subkey(r"Software\ZipMania\ShellExt")?;
        meta.set_value("ExePath", &exe_path()?)?;

        // CLSID in-proc 서버
        let base = format!(r"Software\Classes\CLSID\{CLSID_MENU}");
        let (k, _) = hkcu.create_subkey(&base)?;
        k.set_value("", &"ZipMania Shell Menu")?;
        let (inproc, _) = hkcu.create_subkey(format!(r"{base}\InprocServer32"))?;
        inproc.set_value("", &dll)?;
        inproc.set_value("ThreadingModel", &"Apartment")?;

        // 컨텍스트 메뉴 핸들러 연결(모든 파일 + 폴더), (기본값 = CLSID)
        for rel in handler_targets() {
            let (h, _) = hkcu.create_subkey(&rel)?;
            h.set_value("", &CLSID_MENU)?;
        }

        Ok(())
    }

    /// 등록한 모든 키 제거
    pub fn unregister() -> io::Result<()> {
        for rel in handler_targets() {
            delete_tree(&rel);
        }
        delete_tree(&format!(r"Software\Classes\CLSID\{CLSID_MENU}"));
        delete_tree(r"Software\ZipMania\ShellExt");
        Ok(())
    }

    /// 등록 흔적 유무, CLSID 나 핸들러 키 중 하나라도 남아 있으면 true
    pub fn has_traces() -> bool {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let clsid = format!(r"Software\Classes\CLSID\{CLSID_MENU}");
        if hkcu.open_subkey_with_flags(&clsid, KEY_READ).is_ok() {
            return true;
        }
        handler_targets()
            .iter()
            .any(|rel| hkcu.open_subkey_with_flags(rel, KEY_READ).is_ok())
    }

    // 핸들러 키 2개가 모두 우리 CLSID 를 가리키는지
    fn handlers_linked() -> bool {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        handler_targets().iter().all(|rel| {
            hkcu.open_subkey_with_flags(rel, KEY_READ)
                .ok()
                .and_then(|k| k.get_value::<String, _>("").ok())
                .is_some_and(|v: String| v.eq_ignore_ascii_case(CLSID_MENU))
        })
    }

    /// 등록 여부, CLSID 와 핸들러 키 2개와 실제 DLL 파일이 모두 성립할 때만 true
    /// 셋 중 하나만 봐서는 부분 등록이 정상으로 읽혀 sync 가 영원히 복구하지 않음
    pub fn is_registered() -> bool {
        let Some(dll) = registered_dll() else {
            return false;
        };
        if !std::path::Path::new(&dll).is_file() {
            return false;
        }
        handlers_linked()
    }

    /// 현재 등록된 DLL 경로(InprocServer32 기본값)
    pub fn registered_dll() -> Option<String> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let k = hkcu
            .open_subkey_with_flags(
                format!(r"Software\Classes\CLSID\{CLSID_MENU}\InprocServer32"),
                KEY_READ,
            )
            .ok()?;
        k.get_value::<String, _>("").ok()
    }

    /// 셸에 연결 변경 통지
    pub fn notify_shell() {
        use windows::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};
        unsafe {
            SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
        }
    }
}

// ── 공개 API(플랫폼 무관 래퍼) ───────────────────────────────────────────────

/// 탐색기 통합 등록(DLL 절대경로), 성공 시 셸 갱신
#[cfg(windows)]
pub fn register(dll: &str) -> std::io::Result<()> {
    let r = imp::register(dll);
    if r.is_ok() {
        imp::notify_shell();
    }
    r
}

/// 탐색기 통합 등록 제거
#[cfg(windows)]
pub fn unregister() -> std::io::Result<()> {
    let r = imp::unregister();
    imp::notify_shell();
    r
}

/// 현재 등록 여부
#[cfg(windows)]
pub fn is_registered() -> bool {
    imp::is_registered()
}

/// 설정에 맞춰 등록 동기화(dll = 셸 확장 절대경로)
#[cfg(windows)]
pub fn sync(enabled: bool, dll: &str) {
    if enabled {
        let need = !imp::is_registered() || imp::registered_dll().as_deref() != Some(dll);
        if need {
            let _ = register(dll);
        }
    } else if imp::has_traces() {
        let _ = unregister();
    }
}

/// 메뉴 등록 경로 선택, root = 설치 루트(ZipMania.exe 가 있는 자리 = 패키지 외부 위치)
/// dll = 셸 확장 절대경로(root 아래 SHELLEXT_DIR), 둘은 같은 폴더가 아니다
/// force = 이미 등록돼 있어도 다시 등록, 설치/갱신 경로 전용
/// Win11 은 패키지, 실패하면 클래식으로 물러난다 — 둘 다 없으면 메뉴가 통째로 사라진다
#[cfg(windows)]
pub fn sync_all(enabled: bool, root: &std::path::Path, dll: &str, force: bool) {
    if !enabled {
        let _ = crate::msix::unregister();
        sync(false, dll);
        return;
    }

    // 승격된 셸은 패키지 확장을 못 띄우므로 클래식으로 간다, 옛 등록은 치운다
    if crate::msix::is_win11() && crate::msix::shell_hosts_packages() {
        // 갱신에서 다시 걸지 않으면 매니페스트를 고쳐도 옛 등록이 그대로 남는다
        // 등록 자리가 바뀐 판으로 올라간 사용자는 메뉴가 조용히 죽는다
        let ok = if force || !crate::msix::is_registered() {
            crate::msix::register(root).is_ok()
        } else {
            true
        };
        if ok {
            // 같은 메뉴가 레거시에서 두 번 뜨지 않게 클래식은 내린다
            sync(false, dll);
            return;
        }
    } else {
        let _ = crate::msix::unregister();
    }

    sync(true, dll);
}

#[cfg(not(windows))]
pub fn sync_all(_enabled: bool, _root: &std::path::Path, _dll: &str, _force: bool) {}

// 비-Windows 스텁
#[cfg(not(windows))]
pub fn register(_dll: &str) -> std::io::Result<()> {
    Ok(())
}
#[cfg(not(windows))]
pub fn unregister() -> std::io::Result<()> {
    Ok(())
}
#[cfg(not(windows))]
pub fn is_registered() -> bool {
    false
}
#[cfg(not(windows))]
pub fn sync(_enabled: bool, _dll: &str) {}

/// 실제 HKCU 를 건드리므로 시작 전 상태를 떠 두고 Drop 에서 되돌린다
#[cfg(all(test, windows))]
mod reg_tests {
    use super::*;
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    struct Restore(Option<String>);

    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = imp::unregister();
            if let Some(dll) = self.0.take() {
                let _ = imp::register(&dll);
            }
        }
    }

    #[test]
    fn 부분_등록은_등록으로_치지_않는다() {
        let _restore = Restore(if is_registered() { imp::registered_dll() } else { None });

        // DLL 자리에 실재하는 파일이 필요, 테스트 실행 파일로 대신한다
        let dll = std::env::current_exe().unwrap().to_string_lossy().to_string();
        let dir_key = format!(
            r"Software\Classes\Directory\ShellEx\ContextMenuHandlers\{HANDLER_NAME}"
        );

        register(&dll).unwrap();
        assert!(is_registered());

        // 핸들러 키 소실 = 메뉴가 뜨지 않는 상태, CLSID 만 보면 정상으로 읽힌다
        RegKey::predef(HKEY_CURRENT_USER)
            .delete_subkey_all(&dir_key)
            .unwrap();
        assert!(!is_registered(), "핸들러 키가 없는데 등록으로 판정");
        assert!(imp::has_traces(), "CLSID 가 남았는데 흔적 없음으로 판정");

        // 판정이 옳아야 sync 가 스스로 복구한다
        sync(true, &dll);
        assert!(is_registered(), "sync 가 복구하지 못함");

        // DLL 파일 소실도 같다, CoCreateInstance 가 실패해 메뉴가 뜨지 않는다
        register(&format!(r"Z:\none\ZipManiaShell.dll")).unwrap();
        assert!(!is_registered(), "DLL 이 없는데 등록으로 판정");

        unregister().unwrap();
        assert!(!imp::has_traces());
    }
}
