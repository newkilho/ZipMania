//! 압축 백엔드 추상화, ArchiveBackend + 확장자(읽기), 출력 포맷(쓰기)으로 고르는 Router
//! 새 엔진 = 트레이트 구현 1개 + 등록 1줄, (D3.5)

/// 7z.dll(in-process COM), Windows 전용, 크레이트 내 유일한 OS 의존 모듈
/// 타 플랫폼: 컴파일 제외 → Router 가 unsupported 반환
#[cfg(windows)]
pub mod sevenzip;
pub mod unegg;
pub mod unzip;

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::error::ZipManiaError;
use crate::formats::{CompressFormat, OverwriteMode, ScanFn};
use crate::models::{ArchiveEntry, ScanEntry, TestEntry};

/// 해제 옵션
#[derive(Debug, Clone)]
pub struct ExtractOptions {
    pub archive: String,
    pub dest: String,
    pub keep_paths: bool,
    pub overwrite: OverwriteMode,
    pub password: Option<String>,
    pub selected: Vec<String>,
    pub decisions: std::collections::HashMap<String, OverwriteMode>,
}

/// 압축 생성 옵션
#[derive(Debug, Clone)]
pub struct CreateOptions {
    pub output: String,
    pub inputs: Vec<String>,
    pub format: CompressFormat,
    pub level: u8,
    pub password: Option<String>,
    pub encrypt_names: bool,
}

/// 해제 결과, 앱이 job:done / job:error 로 변환
pub enum ExtractResult {
    /// status = ok / warning(항목이 빠짐) / canceled
    Done { status: &'static str, message: String },
    Failed(ZipManiaError),
}

/// 아카이브 편집 옵션, 항목 추가/삭제
#[derive(Debug, Clone)]
pub struct EditOptions {
    pub archive: String,
    pub add: Vec<String>,
    pub remove: Vec<String>,
    pub password: Option<String>,
}

/// 압축 결과, 앱이 job:done / job:error 로 변환
pub enum CreateResult {
    /// status = ok / warning(항목이 빠짐) / canceled
    Done { status: &'static str, message: String },
    Failed(ZipManiaError),
}

/// (percent 0~100, 현재 파일명), 제네릭 아닌 트레이트 객체 = dyn ArchiveBackend 객체 안전성
pub type ProgressFn<'a> = dyn FnMut(u8, Option<String>) + 'a;

/// 압축 백엔드 공통 인터페이스, 라우팅 후 앱은 이것만 호출
/// Send + Sync = 소비자가 Router 를 static(OnceLock)으로 들기 위함
pub trait ArchiveBackend: Send + Sync {
    /// sevenzip, unegg 등
    fn id(&self) -> &'static str;
    /// 소문자, 점 없음
    fn read_exts(&self) -> &'static [&'static str];
    /// 소문자, 점 없음
    fn write_exts(&self) -> &'static [&'static str];

    /// 확장자 미상 파일 담당 여부, 내용(시그니처) 탐지 가능한 엔진만 true
    /// read_exts 밖의 파일 → 이 값이 참인 첫 백엔드, 없으면 unsupported
    fn accepts_unknown(&self) -> bool {
        false
    }

    /// 상태줄 표시용, 버전 개념 없는 백엔드는 기본 구현대로 오류
    fn engine_version(&self) -> Result<String, ZipManiaError> {
        Err(ZipManiaError::new(
            "unsupported",
            "이 백엔드는 버전 정보를 제공하지 않습니다.",
        ))
    }

    fn list(&self, archive: &str, password: Option<&str>) -> Result<Vec<ArchiveEntry>, ZipManiaError>;

    fn extract(
        &self,
        opts: &ExtractOptions,
        on_progress: &mut ProgressFn<'_>,
        cancel: Arc<AtomicBool>,
    ) -> ExtractResult;

    fn create(
        &self,
        opts: &CreateOptions,
        on_progress: &mut ProgressFn<'_>,
        cancel: Arc<AtomicBool>,
    ) -> CreateResult;

    /// 편집, 기존 항목 = 재압축 없이 복사, 추가분만 새로 압축, 7z, zip, tar 만
    fn edit(
        &self,
        opts: &EditOptions,
        on_progress: &mut ProgressFn<'_>,
        cancel: Arc<AtomicBool>,
    ) -> CreateResult {
        let _ = (on_progress, cancel);
        let _ = opts;
        CreateResult::Failed(ZipManiaError::new(
            "unsupported",
            "이 형식은 편집을 지원하지 않습니다.",
        ))
    }

    fn test(&self, archive: &str, password: Option<&str>) -> Result<(), ZipManiaError>;

    /// 무결성 테스트(상세), 암호 문제 = 전역 오류, 개별 파일 손상 = 항목 ok=false(실패 아닌 결과)
    /// 기본 구현 = test + 빈 목록
    fn test_report(
        &self,
        archive: &str,
        password: Option<&str>,
        on_progress: &mut ProgressFn<'_>,
        cancel: Arc<AtomicBool>,
    ) -> Result<Vec<TestEntry>, ZipManiaError> {
        let _ = (on_progress, cancel);
        self.test(archive, password)?;
        Ok(Vec::new())
    }

    /// 바이러스 검사(AMSI), 파일별 메모리 해제(max_size 미만) → 콜백, 7z 만
    fn scan_report(
        &self,
        archive: &str,
        password: Option<&str>,
        max_size: u64,
        scan: ScanFn,
        on_progress: &mut ProgressFn<'_>,
        cancel: Arc<AtomicBool>,
    ) -> Result<Vec<ScanEntry>, ZipManiaError> {
        let _ = (archive, password, max_size, scan, on_progress, cancel);
        Err(ZipManiaError::new(
            "unsupported",
            "이 형식은 바이러스 검사를 지원하지 않습니다.",
        ))
    }

    /// 뷰어용, 임시 해제 없이 내부 파일 1개의 바이트
    fn read_entry_to_memory(
        &self,
        archive: &str,
        inner_path: &str,
        password: Option<&str>,
    ) -> Result<Vec<u8>, ZipManiaError>;

    /// 드래그용, 단일 항목 → 파일 스트리밍 추출(메모리 전체 적재 없음), 대용량 Shell DnD 지연 렌더링
    fn extract_entry_to_file(
        &self,
        archive: &str,
        inner_path: &str,
        dest_file: &Path,
        password: Option<&str>,
    ) -> Result<(), ZipManiaError> {
        let _ = (archive, inner_path, dest_file, password);
        Err(ZipManiaError::new(
            "unsupported",
            "이 형식은 단일 항목 파일 추출을 지원하지 않습니다.",
        ))
    }

    /// 드래그용, 단일 항목 → 임의 writer, 임시 파일, 전체 메모리 없음
    fn extract_entry_to_writer(
        &self,
        archive: &str,
        inner_path: &str,
        writer: Box<dyn std::io::Write + Send>,
        password: Option<&str>,
    ) -> Result<(), ZipManiaError> {
        let _ = (archive, inner_path, writer, password);
        Err(ZipManiaError::new(
            "unsupported",
            "이 형식은 단일 항목 스트리밍 추출을 지원하지 않습니다.",
        ))
    }

    /// 해제 전 충돌 검사, 목록 vs 대상 폴더 → 이미 있는 파일의 내부 경로
    /// 목록만 쓰므로 백엔드 공통 기본 구현
    fn find_conflicts(
        &self,
        archive: &str,
        dest: &str,
        keep_paths: bool,
        selected: &[String],
        password: Option<&str>,
    ) -> Result<Vec<String>, ZipManiaError> {
        let entries = self.list(archive, password)?;
        Ok(compute_conflicts(&entries, dest, keep_paths, selected))
    }
}

/// 충돌 목록 계산(엔진 무관 순수 로직)
/// selected 빈 값 = 전체, 있으면 그 경로(폴더는 하위 포함)만
/// 대상 = keep_paths ? dest/<내부경로> : dest/<파일명>, 반환 = / 정규화 내부 경로(UI 표시용)
pub fn compute_conflicts(
    entries: &[ArchiveEntry],
    dest: &str,
    keep_paths: bool,
    selected: &[String],
) -> Vec<String> {
    // 비면 전체
    let sel_norm: Vec<String> = selected.iter().map(|s| s.replace('\\', "/")).collect();

    let dest_path = Path::new(dest);
    let mut conflicts = Vec::new();

    for e in entries {
        if e.is_dir {
            continue;
        }
        let path_norm = e.path.replace('\\', "/");

        // 선택 경로 자신 또는 그 하위만
        if !sel_norm.is_empty() {
            let in_scope = sel_norm
                .iter()
                .any(|s| path_norm == *s || path_norm.starts_with(&format!("{s}/")));
            if !in_scope {
                continue;
            }
        }

        // 대상 파일 경로 계산
        let rel = if keep_paths {
            path_norm.clone()
        } else {
            // 평면 = 파일명만
            path_norm.rsplit('/').next().unwrap_or(&path_norm).to_string()
        };
        // 해제와 같은 정규화 필수 — 아니면 미리보기와 실제 결과가 어긋남
        // 정규화 실패 항목은 해제도 건너뜀 → 충돌 아님
        let target = match crate::paths::sanitize(&rel)
            .and_then(|r| crate::paths::resolve_under(dest_path, &r))
        {
            Ok(t) => t,
            Err(_) => continue,
        };
        if target.exists() {
            conflicts.push(path_norm);
        }
    }

    conflicts
}

/// 무담당 확장자용 자리표시자, 전부 unsupported
/// 백엔드 0개인 플랫폼에서도 크레이트 컴파일 + 오류 반환 보장
pub struct Unsupported;

impl ArchiveBackend for Unsupported {
    fn id(&self) -> &'static str {
        "none"
    }
    fn read_exts(&self) -> &'static [&'static str] {
        &[]
    }
    fn write_exts(&self) -> &'static [&'static str] {
        &[]
    }
    fn list(&self, _a: &str, _p: Option<&str>) -> Result<Vec<ArchiveEntry>, ZipManiaError> {
        Err(unsupported_here())
    }
    fn extract(
        &self,
        _o: &ExtractOptions,
        _pr: &mut ProgressFn<'_>,
        _c: Arc<AtomicBool>,
    ) -> ExtractResult {
        ExtractResult::Failed(unsupported_here())
    }
    fn create(
        &self,
        _o: &CreateOptions,
        _pr: &mut ProgressFn<'_>,
        _c: Arc<AtomicBool>,
    ) -> CreateResult {
        CreateResult::Failed(unsupported_here())
    }
    fn test(&self, _a: &str, _p: Option<&str>) -> Result<(), ZipManiaError> {
        Err(unsupported_here())
    }
    fn read_entry_to_memory(
        &self,
        _a: &str,
        _i: &str,
        _p: Option<&str>,
    ) -> Result<Vec<u8>, ZipManiaError> {
        Err(unsupported_here())
    }
}

fn unsupported_here() -> ZipManiaError {
    ZipManiaError::new(
        "unsupported",
        "이 형식을 처리할 압축 엔진이 없습니다.",
    )
}

// ─────────────────────────── 백엔드 등록 (이식 지점) ───────────────────────────
// 새 엔진 = backend/<엔진>.rs 구현 + default_backends 에 push 한 줄(내용 탐지가 되면
// accepts_unknown() = true), 앱, UI, commands.rs 는 바뀌지 않는다

/// 백엔드 생성 설정, 엔진 추가 시 필드 추가
#[derive(Debug, Clone)]
pub struct BackendConfig {
    pub sevenzip_dll: PathBuf,
}

/// 이 빌드의 백엔드 목록, 앞일수록 우선
/// Windows 만 sevenzip::SevenZip 보유, macOS, Linux = 슬롯 빔 → 읽기 Unsupported, (D3.5)
fn default_backends(cfg: &BackendConfig) -> Vec<Box<dyn ArchiveBackend>> {
    #[allow(unused_mut)]
    let mut v: Vec<Box<dyn ArchiveBackend>> = Vec::new();

    // ZIP 계열 순수 Rust 백엔드, 7z 보다 앞 필수 — 뒤에 두면 7z 가 확장자를 가로챔
    // 분할, 손상 zip 은 라우터가 아니라 이 백엔드 안에서 7z 로 위임
    #[cfg(windows)]
    v.push(Box::new(unzip::Unzip::with_fallback(Box::new(
        sevenzip::SevenZip::new(cfg.sevenzip_dll.clone()),
    ))));
    #[cfg(not(windows))]
    v.push(Box::new(unzip::Unzip::new()));

    // ── Windows: 7z.dll in-process COM ──
    #[cfg(windows)]
    v.push(Box::new(sevenzip::SevenZip::new(cfg.sevenzip_dll.clone())));

    // macOS/Linux 이식 슬롯, 구현체 완성 후 아래를 실제 등록으로 교체, (D3.5)
    //  #[cfg(unix)]
    // v.push(Box::new(libarchive::LibArchive::new()));

    // egg/alz 순수 Rust 백엔드, 잔여 항목 = (D3.9)
    v.push(Box::new(unegg::Unegg::new()));

    let _ = cfg; // 주 엔진 슬롯이 빈 플랫폼에서 미사용 경고 방지
    v
}

/// 확장자 → 백엔드 라우터, 등록 순서대로 탐색
/// 미선언 확장자 → accepts_unknown 인 첫 백엔드 → 없으면 Unsupported
/// 백엔드 구성은 플랫폼별로 달라도 공개 API 는 동일
pub struct Router {
    backends: Vec<Box<dyn ArchiveBackend>>,
    none: Unsupported,
}

impl Router {
    /// 7z.dll 경로 = Windows 백엔드 전용, 여기서 DLL 로드 안 함 — 로드는 실제 작업 호출 시점
    pub fn new(sevenzip_dll_path: PathBuf) -> Self {
        Router::with_config(BackendConfig {
            sevenzip_dll: sevenzip_dll_path,
        })
    }

    /// 설정 → 기본 구성 생성
    pub fn with_config(cfg: BackendConfig) -> Self {
        Router::with_backends(default_backends(&cfg))
    }

    /// 테스트, 특수 구성용 직접 주입, 앞일수록 우선
    pub fn with_backends(backends: Vec<Box<dyn ArchiveBackend>>) -> Self {
        Router {
            backends,
            none: Unsupported,
        }
    }

    /// 등록된 백엔드 식별자 목록(진단용)
    pub fn backend_ids(&self) -> Vec<&'static str> {
        self.backends.iter().map(|b| b.id()).collect()
    }

    /// 버전을 아는 첫 백엔드의 값, 상태줄 표시용
    pub fn engine_version(&self) -> Result<String, ZipManiaError> {
        for b in &self.backends {
            if let Ok(v) = b.engine_version() {
                return Ok(v);
            }
        }
        Err(ZipManiaError::new(
            "unsupported",
            "사용 가능한 압축 엔진이 없습니다.",
        ))
    }

    /// 확장자 → 읽기, 해제 백엔드
    pub fn for_archive(&self, archive: &str) -> &dyn ArchiveBackend {
        self.pick(&crate::formats::ext_of(archive), Capability::Read)
    }

    /// 출력 포맷 문자열 → 쓰기 백엔드
    pub fn for_format(&self, format: &str) -> &dyn ArchiveBackend {
        self.pick(&format.to_ascii_lowercase(), Capability::Write)
    }

    /// 확장자 → 백엔드
    /// 읽기: 선언 백엔드 → 내용 탐지 가능 백엔드
    /// 쓰기: 생성 가능 백엔드 → 그 포맷을 읽는 백엔드, 내용 탐지 폴백 금지, (D3.5)
    fn pick(&self, ext: &str, cap: Capability) -> &dyn ArchiveBackend {
        let find = |f: &dyn Fn(&dyn ArchiveBackend) -> bool| {
            self.backends.iter().find(|b| f(b.as_ref())).map(|b| b.as_ref())
        };
        match cap {
            Capability::Read => find(&|b| b.read_exts().contains(&ext))
                .or_else(|| find(&|b| b.accepts_unknown()))
                .unwrap_or(&self.none),
            Capability::Write => find(&|b| b.write_exts().contains(&ext))
                .or_else(|| find(&|b| b.read_exts().contains(&ext)))
                .unwrap_or(&self.none),
        }
    }
}

/// 백엔드 선택 시 요구 능력
#[derive(Clone, Copy)]
enum Capability {
    Read,
    Write,
}

// 백엔드 추가에도 선택 규칙 고정, 가짜 백엔드라 플랫폼 무관

#[cfg(test)]
mod router_tests {
    use super::*;

    /// 선언 확장자만 다른 가짜 백엔드
    struct Fake {
        id: &'static str,
        read: &'static [&'static str],
        write: &'static [&'static str],
        unknown: bool,
    }

    impl ArchiveBackend for Fake {
        fn id(&self) -> &'static str {
            self.id
        }
        fn read_exts(&self) -> &'static [&'static str] {
            self.read
        }
        fn write_exts(&self) -> &'static [&'static str] {
            self.write
        }
        fn accepts_unknown(&self) -> bool {
            self.unknown
        }
        fn list(&self, _a: &str, _p: Option<&str>) -> Result<Vec<ArchiveEntry>, ZipManiaError> {
            Ok(Vec::new())
        }
        fn extract(
            &self,
            _o: &ExtractOptions,
            _pr: &mut ProgressFn<'_>,
            _c: Arc<AtomicBool>,
        ) -> ExtractResult {
            ExtractResult::Done { status: "ok", message: String::new() }
        }
        fn create(
            &self,
            _o: &CreateOptions,
            _pr: &mut ProgressFn<'_>,
            _c: Arc<AtomicBool>,
        ) -> CreateResult {
            CreateResult::Done { status: "ok", message: String::new() }
        }
        fn test(&self, _a: &str, _p: Option<&str>) -> Result<(), ZipManiaError> {
            Ok(())
        }
        fn read_entry_to_memory(
            &self,
            _a: &str,
            _i: &str,
            _p: Option<&str>,
        ) -> Result<Vec<u8>, ZipManiaError> {
            Ok(Vec::new())
        }
    }

    /// 주 엔진(내용 탐지 가능) + 자리표시자 엔진 구성
    fn router() -> Router {
        Router::with_backends(vec![
            Box::new(Fake { id: "main", read: &["7z", "zip"], write: &["7z", "zip"], unknown: true }),
            Box::new(Fake { id: "alt", read: &["egg"], write: &[], unknown: false }),
        ])
    }

    #[test]
    fn 읽기는_확장자를_선언한_백엔드로_간다() {
        assert_eq!(router().for_archive("a.egg").id(), "alt");
        assert_eq!(router().for_archive("a.7z").id(), "main");
        assert_eq!(router().for_archive("A.ZIP").id(), "main"); // 대소문자 무관
    }

    #[test]
    fn 모르는_확장자는_내용탐지_백엔드가_맡는다() {
        // 미선언 파일 → 시그니처 판별
        assert_eq!(router().for_archive("확장자없음").id(), "main");
        assert_eq!(router().for_archive("a.unknown").id(), "main");
    }

    #[test]
    fn 내용탐지_백엔드가_없으면_미지원() {
        let r = Router::with_backends(vec![Box::new(Fake {
            id: "alt",
            read: &["egg"],
            write: &[],
            unknown: false,
        })]);
        assert_eq!(r.for_archive("a.unknown").id(), "none");
    }

    #[test]
    fn 생성은_생성가능_백엔드로_간다() {
        assert_eq!(router().for_format("zip").id(), "main");
    }

    #[test]
    fn 생성_미지원_포맷은_그_포맷을_읽는_백엔드가_받는다() {
        // egg 생성 가능 백엔드 없음 → 7z 아니라 포맷 주인(alt)이 받아야 함
        assert_eq!(router().for_format("egg").id(), "alt");
    }

    #[test]
    fn 아무도_모르는_포맷_생성은_미지원() {
        assert_eq!(router().for_format("모름").id(), "none");
    }

    #[test]
    fn 라우터는_static_으로_보관할_수_있다() {
        // 소비자의 static R: OnceLock<Router> 사용 보장
        // ArchiveBackend: Send + Sync 바운드가 빠지면 여기서 컴파일 실패
        fn 공유가능<T: Send + Sync>() {}
        공유가능::<Router>();

        static R: std::sync::OnceLock<Router> = std::sync::OnceLock::new();
        let r = R.get_or_init(|| Router::new(PathBuf::from("7z.dll")));
        // 2번째 호출이 같은 인스턴스인가 = 1회만 구성되는가
        assert!(std::ptr::eq(r, R.get().unwrap()));
    }

    #[test]
    fn 백엔드가_하나도_없어도_동작한다() {
        // 이식 초기(백엔드 미구현) → 패닉 없이 unsupported
        let r = Router::with_backends(Vec::new());
        assert_eq!(r.for_archive("a.7z").id(), "none");
        assert!(r.for_archive("a.7z").list("a.7z", None).is_err());
        assert!(r.engine_version().is_err());
    }
}

// 슬롯 빈 플랫폼(macOS, Linux) 계약: Router 생성 패닉 없음 + 요청은 unsupported
// 슬롯을 채우면 이 테스트 갱신

#[cfg(test)]
mod platform_tests {
    use super::*;

    fn default_router() -> Router {
        Router::new(PathBuf::from("7z.dll"))
    }

    #[test]
    #[cfg(windows)]
    fn windows_는_7z_백엔드를_등록한다() {
        let ids = default_router().backend_ids();
        assert!(ids.contains(&"sevenzip"), "등록된 백엔드: {ids:?}");
        assert_eq!(default_router().for_archive("a.7z").id(), "sevenzip");
        // 확장자 미상이어도 7z 가 내용으로 판별
        assert_eq!(default_router().for_archive("확장자없음").id(), "sevenzip");
    }

    #[test]
    #[cfg(not(windows))]
    fn 슬롯이_빈_플랫폼은_미지원으로_떨어진다() {
        // macOS, Linux: 주 엔진 슬롯 빈 현재 상태의 계약
        let r = default_router();
        assert_eq!(r.for_archive("a.7z").id(), "none");
        assert!(r.for_archive("a.7z").list("a.7z", None).is_err());
        assert!(r.engine_version().is_err());
        // egg/alz 는 플랫폼 무관 등록
        assert_eq!(r.for_archive("a.egg").id(), "unegg");
    }

    #[test]
    fn 라우터_생성은_어느_플랫폼에서도_패닉하지_않는다() {
        let _ = default_router().backend_ids();
    }
}

// ────────────────── egg/alz 가 unegg 백엔드로 가는지 확인 ──────────────────

#[cfg(test)]
mod unegg_routing_tests {
    use super::*;

    #[test]
    fn egg_와_alz_는_unegg_백엔드가_맡는다() {
        // 7z 가 먼저 등록 → 그쪽이 egg/alz 를 선언하면 여기서 sevenzip 이 나옴
        let r = Router::new(PathBuf::from("7z.dll"));
        assert_eq!(r.for_archive("a.egg").id(), "unegg");
        assert_eq!(r.for_archive("a.alz").id(), "unegg");
        assert_eq!(r.for_archive("A.EGG").id(), "unegg"); // 대소문자 무관
    }

    #[test]
    #[cfg(windows)]
    fn 나머지_확장자는_7z_백엔드가_맡는다() {
        let r = Router::new(PathBuf::from("7z.dll"));
        assert_eq!(r.for_archive("a.7z").id(), "sevenzip");
        assert_eq!(r.for_archive("a.rar").id(), "sevenzip");
    }

    /// zip 계열 = unzip 전담, 7z 보다 뒤에 등록되면 영영 호출 안 됨 → 등록 순서 역전 검사
    #[test]
    fn zip_계열은_unzip_백엔드가_맡는다() {
        let r = Router::new(PathBuf::from("7z.dll"));
        for p in ["a.zip", "a.zipx", "a.jar", "a.cbz", "A.ZIP"] {
            assert_eq!(r.for_archive(p).id(), "unzip", "{p}");
        }
        assert_eq!(r.for_format("zip").id(), "unzip");
    }

    #[test]
    fn egg_생성은_unegg_가_받아_거부한다() {
        // 7z 가 .egg 생성 시도하면 안 됨
        let r = Router::new(PathBuf::from("7z.dll"));
        assert_eq!(r.for_format("egg").id(), "unegg");
    }
}
