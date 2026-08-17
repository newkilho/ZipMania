//! 탐색기 우클릭 통합 등록, IContextMenu 셸 확장(ZipManiaShell.dll, CLSID 1개) → HKCU
//! 자리 = *\ShellEx\ContextMenuHandlers\ZipMania, Directory\ShellEx\… CLSID 는 ZipManiaShell.cpp 와 동일(D3.7)

#![allow(dead_code)]

/// 컨텍스트 메뉴 핸들러 CLSID — ZipManiaShell.cpp 의 상수와 반드시 일치
pub const CLSID_MENU: &str = "{02BEA257-B0A9-4B99-9A99-F3F61885D771}";

/// 셸 확장 DLL 파일명(ZipMania.exe 와 같은 폴더에 평면 배치)
pub const SHELLEXT_DLL: &str = "ZipManiaShell.dll";

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

    /// 등록 여부(CLSID 존재로 판단)
    pub fn is_registered() -> bool {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        hkcu.open_subkey_with_flags(
            format!(r"Software\Classes\CLSID\{CLSID_MENU}\InprocServer32"),
            KEY_READ,
        )
        .is_ok()
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
    } else if imp::is_registered() {
        let _ = unregister();
    }
}

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
