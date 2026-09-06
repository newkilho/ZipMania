//! 스파스 MSIX 패키지 등록/해제, Win11 기본 우클릭 메뉴 진입로(D3.7)
//! 패키지 = 신원과 메뉴 선언만, 실행 파일과 DLL 은 설치 폴더(외부 위치)에 그대로 둔다
//! 파일명 = MSIX_FILE, 신원 = AppxManifest.xml 의 Identity 와 반드시 일치

#![allow(dead_code)]

/// 스파스 패키지 파일명, 자리는 설치 루트 아래 shell_reg::SHELLEXT_DIR
pub const MSIX_FILE: &str = "ZipManiaShell.msix";

/// AppxManifest.xml 의 Identity Name
const PKG_NAME: &str = "Kilhonet.ZipMania";

/// AppxManifest.xml 의 Identity Publisher, 코드 서명 인증서 Subject 와 동일
const PKG_PUBLISHER: &str = "CN=Kilho Oh, O=Kilho Oh, L=Seoul, S=Seoul, C=KR";

/// 외부 위치 패키지를 지원하는 최소 빌드(Windows 10 2004)
const MIN_BUILD: u32 = 19041;

/// Win11 판정 기준 빌드
const WIN11_BUILD: u32 = 22000;

// ── Windows 구현 ─────────────────────────────────────────────────────────────

#[cfg(windows)]
mod imp {
    use super::{MIN_BUILD, MSIX_FILE, PKG_NAME, PKG_PUBLISHER, WIN11_BUILD};
    use std::path::{Path, PathBuf};
    use windows::core::HSTRING;
    use windows::Foundation::Uri;
    use windows::Management::Deployment::{AddPackageOptions, PackageManager};

    // OS 빌드 번호, 조회 실패는 0
    // GetVersionExW 금지 — 호환성 매니페스트가 없으면 Windows 8(9200) 을 답한다
    pub(super) fn build_number() -> u32 {
        use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ};
        use winreg::RegKey;
        RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey_with_flags(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion", KEY_READ)
            .ok()
            .and_then(|k| k.get_value::<String, _>("CurrentBuildNumber").ok())
            .and_then(|v| v.trim().parse::<u32>().ok())
            .unwrap_or(0)
    }

    /// 외부 위치 패키지 지원 여부
    pub fn supported() -> bool {
        build_number() >= MIN_BUILD
    }

    /// Win11 여부, 클래식 핸들러와 패키지 중 무엇을 쓸지 가르는 기준
    pub fn is_win11() -> bool {
        build_number() >= WIN11_BUILD
    }

    // UAC 정책값, 없거나 읽지 못하면 None
    fn policy_dword(name: &str) -> Option<u32> {
        use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ};
        use winreg::RegKey;
        RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey_with_flags(
                r"SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System",
                KEY_READ,
            )
            .ok()
            .and_then(|k| k.get_value::<u32, _>(name).ok())
    }

    /// 셸이 패키지 확장을 로드할 수 있는 환경 여부(D3.7)
    /// 승격된 셸 = 패키지 신원 미부여, UAC 해제와 내장 Administrator 둘 다 해당
    pub fn shell_hosts_packages() -> bool {
        hosts_packages(
            policy_dword("EnableLUA"),
            policy_dword("FilterAdministratorToken"),
            is_builtin_administrator(),
        )
    }

    /// 정책값과 계정 종류만으로 하는 판정, 레지스트리 접근 없음
    /// 애매하면 false — 클래식은 메뉴 자리만 나빠지고, 반대로 틀리면 메뉴가 사라진다
    pub(super) fn hosts_packages(
        enable_lua: Option<u32>,
        filter_admin_token: Option<u32>,
        builtin_admin: bool,
    ) -> bool {
        // UAC 해제 = 셸 전체가 승격 토큰, 값이 없으면 켜진 것이 기본
        if enable_lua.unwrap_or(1) == 0 {
            return false;
        }
        if !builtin_admin {
            return true;
        }
        // 내장 Administrator 는 승인 모드가 켜졌을 때만 비승격 토큰, 기본값은 꺼짐
        filter_admin_token.unwrap_or(0) != 0
    }

    /// 현재 계정이 내장 Administrator(RID 500) 인지, 판정 불가는 false
    /// 실패를 true 로 두면 모든 Win11 사용자가 클래식으로 떨어진다
    fn is_builtin_administrator() -> bool {
        use windows::Win32::Foundation::{CloseHandle, HANDLE};
        use windows::Win32::Security::{
            GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TokenUser,
            TOKEN_QUERY, TOKEN_USER,
        };
        use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

        const DOMAIN_USER_RID_ADMIN: u32 = 500;

        unsafe {
            let mut token = HANDLE::default();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
                return false;
            }

            // 1회차는 길이만, TOKEN_USER 는 SID 가 뒤에 붙는 가변 길이
            let mut len = 0u32;
            let _ = GetTokenInformation(token, TokenUser, None, 0, &mut len);
            let mut buf = vec![0u8; len as usize];
            let got = len as usize >= std::mem::size_of::<TOKEN_USER>()
                && GetTokenInformation(
                    token,
                    TokenUser,
                    Some(buf.as_mut_ptr().cast()),
                    len,
                    &mut len,
                )
                .is_ok();
            let _ = CloseHandle(token);
            if !got {
                return false;
            }

            let sid = (*buf.as_ptr().cast::<TOKEN_USER>()).User.Sid;
            let count = *GetSidSubAuthorityCount(sid);
            count > 0 && *GetSidSubAuthority(sid, u32::from(count) - 1) == DOMAIN_USER_RID_ADMIN
        }
    }

    // 경로 → file:// URI, Uri::CreateUri 는 절대 URI 만 받는다
    fn file_uri(path: &Path) -> windows::core::Result<Uri> {
        let s = path.to_string_lossy().replace('\\', "/");
        let s = s.strip_prefix("//?/").unwrap_or(&s);
        Uri::CreateUri(&HSTRING::from(format!("file:///{s}")))
    }

    fn manager() -> windows::core::Result<PackageManager> {
        PackageManager::new()
    }

    // 현재 사용자에게 등록된 우리 패키지의 전체 이름들
    fn installed_full_names() -> Vec<HSTRING> {
        let Ok(pm) = manager() else {
            return Vec::new();
        };
        // 사용자 범위 조회, 빈 SID = 현재 사용자
        // UserSecurityId 없는 변형은 전체 사용자 대상이라 관리자가 아니면 빈 목록을 준다
        let found = pm.FindPackagesByUserSecurityIdNamePublisher(
            &HSTRING::new(),
            &HSTRING::from(PKG_NAME),
            &HSTRING::from(PKG_PUBLISHER),
        );
        let Ok(found) = found else {
            return Vec::new();
        };
        found
            .into_iter()
            .filter_map(|p| p.Id().ok()?.FullName().ok())
            .collect()
    }

    /// 등록 여부
    pub fn is_registered() -> bool {
        !installed_full_names().is_empty()
    }

    /// 설치 루트를 외부 위치로 삼아 등록, 패키지 파일은 그 아래 셸 폴더
    pub fn register(root: &Path) -> Result<(), String> {
        if !supported() {
            return Err("이 Windows 버전은 외부 위치 패키지를 지원하지 않습니다.".into());
        }
        let msix: PathBuf = root
            .join(crate::shell_reg::SHELLEXT_DIR)
            .join(MSIX_FILE);
        if !msix.is_file() {
            return Err(format!("패키지 파일이 없습니다: {}", msix.display()));
        }

        let pm = manager().map_err(|e| format!("PackageManager 생성 실패: {e}"))?;
        let pkg_uri = file_uri(&msix).map_err(|e| format!("패키지 URI 생성 실패: {e}"))?;
        let ext_uri = file_uri(root).map_err(|e| format!("외부 위치 URI 생성 실패: {e}"))?;

        let opts = AddPackageOptions::new().map_err(|e| format!("옵션 생성 실패: {e}"))?;
        opts.SetExternalLocationUri(&ext_uri)
            .map_err(|e| format!("외부 위치 지정 실패: {e}"))?;
        // 같은 버전이 이미 있어도 덮어쓴다(설치 폴더가 바뀌었을 수 있다)
        let _ = opts.SetForceUpdateFromAnyVersion(true);

        let op = pm
            .AddPackageByUriAsync(&pkg_uri, &opts)
            .map_err(|e| format!("패키지 등록 호출 실패: {e}"))?;
        let result = op.get().map_err(|e| format!("패키지 등록 실패: {e}"))?;

        // ExtendedErrorCode 가 실패면 IsRegistered 가 아니라 이 값이 원인을 담는다
        if let Ok(err) = result.ExtendedErrorCode() {
            if err.is_err() {
                let text = result.ErrorText().unwrap_or_default();
                return Err(format!("패키지 등록 실패: {err:?} {text}"));
            }
        }
        Ok(())
    }

    /// 등록 해제, 등록된 것이 없으면 아무것도 하지 않는다
    pub fn unregister() -> Result<(), String> {
        let pm = manager().map_err(|e| format!("PackageManager 생성 실패: {e}"))?;
        for full in installed_full_names() {
            let op = pm
                .RemovePackageAsync(&full)
                .map_err(|e| format!("패키지 제거 호출 실패: {e}"))?;
            op.get().map_err(|e| format!("패키지 제거 실패: {e}"))?;
        }
        Ok(())
    }
}

// ── 공개 API(플랫폼 무관 래퍼) ───────────────────────────────────────────────

#[cfg(windows)]
pub use imp::{is_registered, is_win11, register, shell_hosts_packages, unregister};

#[cfg(not(windows))]
pub fn is_win11() -> bool {
    false
}
#[cfg(not(windows))]
pub fn is_registered() -> bool {
    false
}
#[cfg(not(windows))]
pub fn shell_hosts_packages() -> bool {
    true
}
#[cfg(not(windows))]
pub fn register(_root: &std::path::Path) -> Result<(), String> {
    Ok(())
}
#[cfg(not(windows))]
pub fn unregister() -> Result<(), String> {
    Ok(())
}

/// 매니페스트는 빌드에 참여하지 않으므로 어긋나도 컴파일이 통과한다, 값으로 대조
#[cfg(test)]
mod tests {
    const MANIFEST: &str = include_str!("../../msix/AppxManifest.xml");
    const SHELL_CPP: &str = include_str!("../../shellext/ZipManiaShell.cpp");
    const SELF: &str = include_str!("msix.rs");
    const ISS: &str = include_str!("../../ZipMania.iss");

    /// 신원이 어긋나면 등록이 실패하거나 다른 패키지가 되어 옛것이 남는다
    #[test]
    fn 매니페스트_신원이_코드와_같다() {
        let name = format!("Name=\"{}\"", super::PKG_NAME);
        assert!(MANIFEST.contains(&name), "AppxManifest 의 Identity Name 이 {name} 과 다르다");
        let pubr = format!("Publisher=\"{}\"", super::PKG_PUBLISHER);
        assert!(
            MANIFEST.contains(&pubr),
            "AppxManifest 의 Publisher 가 코드 상수와 다르다 — 서명 인증서 Subject 와도 같아야 한다"
        );
    }

    /// CLSID 가 어긋나면 메뉴 항목이 뜨고도 CoCreateInstance 가 실패한다
    #[test]
    fn 매니페스트_clsid_가_셸확장과_같다() {
        let clsid = "F58510CC-5B40-48D9-A9FE-120FD5F23DAE";
        assert!(MANIFEST.contains(clsid), "AppxManifest 에 IExplorerCommand CLSID 가 없다");
        assert!(
            SHELL_CPP.contains("0xF58510CC, 0x5B40, 0x48D9"),
            "ZipManiaShell.cpp 의 CLSID_ZipManiaCommand 가 매니페스트와 다르다"
        );
        // 경로는 외부 위치(= 설치 루트) 기준이므로 하위 폴더까지 포함해야 한다
        let want = format!(
            "Path=\"{}\\{}\"",
            crate::shell_reg::SHELLEXT_DIR,
            crate::shell_reg::SHELLEXT_DLL
        );
        assert!(
            MANIFEST.contains(&want),
            "AppxManifest 의 SurrogateServer Path 가 {want} 와 다르다"
        );
    }

    /// 설치 파일이 담는 이름과 앱이 찾는 이름은 같아야 한다
    /// 어긋나면 설치는 되는데 패키지 파일이 없어 등록만 조용히 실패한다
    #[test]
    fn 설치_스크립트가_패키지를_담는다() {
        let want = format!(r"\{}", super::MSIX_FILE);
        assert!(
            ISS.contains(&want),
            "ZipMania.iss 의 [Files] 에 {} 가 없다",
            super::MSIX_FILE
        );
        // 설치 자리도 코드와 같아야 한다, 다르면 설치는 되는데 앱이 못 찾는다
        let sub = format!(r"\{}", crate::shell_reg::SHELLEXT_DIR);
        assert!(
            ISS.contains(&sub),
            "ZipMania.iss 의 셸 폴더가 {} 와 다르다",
            crate::shell_reg::SHELLEXT_DIR
        );
    }

    /// GetVersionExW 는 호환성 매니페스트가 없으면 9200(Windows 8) 을 답한다
    /// 그 값으로 판정하면 Win11 에서도 패키지를 등록하지 않는다
    #[cfg(windows)]
    #[test]
    fn 빌드_번호는_속지_않는다() {
        let n = super::imp::build_number();
        assert!(
            n >= 10240,
            "빌드 번호 {n} — GetVersionExW 계열로 되돌아갔을 가능성"
        );
    }

    /// UserSecurityId 없는 조회는 전체 사용자 대상이라 관리자가 아니면 빈 목록을 준다
    /// 그러면 매 실행 재등록하고, 제거할 때 아무것도 찾지 못해 유령 패키지가 남는다
    #[test]
    fn 패키지_조회는_사용자_범위다() {
        assert!(
            SELF.contains("FindPackagesByUserSecurityIdNamePublisher"),
            "사용자 범위 조회가 아니면 등록해도 조회되지 않는다"
        );
    }

    /// 패키지에 실행 파일을 담지 않는다, 담으면 설치 산출물 구조가 통째로 바뀐다
    #[test]
    fn 외부_콘텐츠_선언이_있다() {
        assert!(
            MANIFEST.contains("<uap10:AllowExternalContent>true</uap10:AllowExternalContent>"),
            "AllowExternalContent 가 없으면 외부 위치 등록이 거부된다"
        );
    }

    /// 승격된 셸에는 패키지 신원이 없다, UAC 정책은 실물로 만들 수 없어 판정 함수로 고정
    #[cfg(windows)]
    #[test]
    fn 승격된_셸은_패키지를_쓰지_않는다() {
        use super::imp::hosts_packages;

        // 보통 사용자 — 패키지가 Win11 기본 메뉴에 오른다
        assert!(hosts_packages(Some(1), Some(0), false));
        assert!(hosts_packages(None, None, false), "정책값이 없으면 UAC 켜짐이 기본");

        // UAC 해제 — 셸 전체가 승격 토큰
        assert!(!hosts_packages(Some(0), Some(0), false));
        assert!(!hosts_packages(Some(0), Some(1), true));

        // 내장 Administrator — UAC 가 켜져 있어도 승인 모드 밖이면 승격 토큰
        assert!(
            !hosts_packages(Some(1), Some(0), true),
            "여기서 참을 주면 패키지만 걸고 클래식을 내려 메뉴가 통째로 사라진다"
        );
        assert!(!hosts_packages(Some(1), None, true), "FilterAdministratorToken 기본값은 꺼짐");
        assert!(hosts_packages(Some(1), Some(1), true), "승인 모드면 비승격 토큰");
    }
}

