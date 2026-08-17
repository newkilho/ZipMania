//! IPC command 핸들러, zipmania-archive 위의 얇은 어댑터
//! dll 해석 → Router 주입 → 크레이트 호출, 작업 큐, 스레드, 이벤트는 앱 몫
//! command 시그니처 = 프런트 계약

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use zipmania_archive::{
    ArchiveEntry, CompressFormat, CreateOptions, CreateResult, EditOptions, ExtractOptions,
    ExtractResult, ZipManiaError, OverwriteMode, Router, SevenZipError,
};

use crate::jobs::JobManager;
use crate::models::{
    CompressBatchItem, CompressLaunch, CompressTake, DirNode, ExtractBatchItem, ExtractContext,
    FolderFile, JobDone, JobErrorEvent, JobProgress, JobStarted, PathInfo, QuickAccess,
    ScanReportEvent, TestReportEvent,
};
use crate::settings::Settings;

/// 압축 창 요청 보관소, 창 생성 직전 적재 → mount 때 take_compress_inputs 로 회수
/// 이벤트 = 가져가라 신호, 값은 언제나 적재, ready 는 값과 같은 자물쇠, 합치지 말고 큐, (D3.5)
#[derive(Default)]
pub struct PendingCompressInputs {
    pub state: Mutex<PendingCompress>,
    pub create: Mutex<()>,
}

/// 대여 이력, 번호, 세대, 세션, 전달 여부 넷 필수(D3.5)
#[derive(Clone)]
pub struct Lease {
    pub gen: u64,
    pub session: String,
    pub dispatched: bool,
}

/// 큐 항목, 값 + 전달 이력
pub struct QueuedLaunch {
    pub id: u64,
    pub launch: CompressLaunch,
    pub lease: Option<Lease>,
}

/// PendingCompressInputs 내용물, 요청 큐 + ready
#[derive(Default)]
pub struct PendingCompress {
    pub queue: std::collections::VecDeque<QueuedLaunch>,
    pub ready: bool,
    next_id: u64,
    next_gen: u64,
    recent: std::collections::VecDeque<(u64, u64)>,
}

/// 마감, 전달 결과, Ok / Already(응답 유실 재시도) / Stale(남의 것, 지난 세대)
/// 참거짓 하나로 합치기 금지(D3.5)
#[derive(serde::Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum LeaseAck {
    Ok,
    Already,
    Stale,
}

impl PendingCompress {
    /// 큐 뒤에 추가, 덮거나 합치지 않음
    fn push(&mut self, launch: CompressLaunch) {
        self.next_id += 1;
        self.queue.push_back(QueuedLaunch {
            id: self.next_id,
            launch,
            lease: None,
        });
    }

    /// 최근 마감 기록 추가, 오래된 것부터 폐기
    fn remember_ack(&mut self, id: u64, gen: u64) {
        const KEEP: usize = 32;
        self.recent.push_back((id, gen));
        while self.recent.len() > KEEP {
            self.recent.pop_front();
        }
    }

    fn was_acked(&self, id: u64, gen: u64) -> bool {
        self.recent.iter().any(|&(i, g)| i == id && g == gen)
    }

    /// 맨 앞 요청 대여, 큐에서 빼는 것은 ack 뿐, 같은 창 재시도 = 같은 번호, 세대, 남의 것은 안 뺏음
    fn lease(&mut self, session: &str) -> CompressTake {
        let more = self.queue.len() > 1;
        let Some(front) = self.queue.front_mut() else {
            return CompressTake::default();
        };
        match &front.lease {
            // 같은 창 재시도 → 그대로 반환(멱등)
            Some(l) if l.session == session => {
                return CompressTake {
                    id: front.id,
                    gen: l.gen,
                    launch: Some(front.launch.clone()),
                    more,
                };
            }
            // 살아 있는 다른 창 보유 → 줄 것 없음
            Some(_) => return CompressTake::default(),
            None => {}
        }
        self.next_gen += 1;
        let gen = self.next_gen;
        front.lease = Some(Lease {
            gen,
            session: session.to_string(),
            dispatched: false,
        });
        CompressTake {
            id: front.id,
            gen,
            launch: Some(front.launch.clone()),
            more,
        }
    }

    /// 실행 개시 통지(적용 직전), 창 사망 시 되돌림/버림을 가름
    fn dispatch(&mut self, id: u64, gen: u64) -> LeaseAck {
        // 마감 기록을 큐보다 먼저 확인
        let acked = self.was_acked(id, gen);
        match self.queue.front_mut() {
            Some(front) if front.id == id => match &mut front.lease {
                Some(l) if l.gen == gen => {
                    if l.dispatched {
                        LeaseAck::Already
                    } else {
                        l.dispatched = true;
                        LeaseAck::Ok
                    }
                }
                _ => LeaseAck::Stale,
            },
            // 마감된 요청 재통지(응답 유실 재시도)
            _ if acked => LeaseAck::Already,
            _ => LeaseAck::Stale,
        }
    }

    /// 대여 요청 마감, 번호, 세대 일치 시에만, 이미 마감 = Already
    fn ack(&mut self, id: u64, gen: u64) -> LeaseAck {
        let matched = matches!(
            self.queue.front(),
            Some(f) if f.id == id && f.lease.as_ref().is_some_and(|l| l.gen == gen)
        );
        if matched {
            self.queue.pop_front();
            self.remember_ack(id, gen);
            return LeaseAck::Ok;
        }
        if self.was_acked(id, gen) {
            return LeaseAck::Already;
        }
        LeaseAck::Stale
    }

    /// 창 사망 처리, 넘기기 전 = 되돌림, 넘긴 뒤 = 버림, 반환 = 되돌린 개수
    fn release_session(&mut self, session: &str) -> usize {
        let mut restored = 0;
        self.queue.retain_mut(|q| {
            let Some(l) = &q.lease else { return true };
            if l.session != session {
                return true;
            }
            if l.dispatched {
                false
            } else {
                q.lease = None;
                restored += 1;
                true
            }
        });
        restored
    }

    /// 아무도 안 든 요청 유무(= 창을 열어야 하나)
    fn has_unleased(&self) -> bool {
        self.queue.iter().any(|q| q.lease.is_none())
    }
}

/// 창 세션 신원(compress#3), label 은 재사용되므로 소유 판정은 전부 이 값(D3.5)
#[derive(Default)]
pub struct WindowSessions {
    next: std::sync::atomic::AtomicU64,
    current: Mutex<std::collections::HashMap<String, String>>,
}

impl WindowSessions {
    /// 세션 발급, 같은 label 의 앞 세션 대체
    pub fn begin(&self, label: &str) -> String {
        let n = self
            .next
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1;
        let session = format!("{label}#{n}");
        let mut map = self.current.lock().unwrap_or_else(|e| e.into_inner());
        map.insert(label.to_string(), session.clone());
        session
    }

    /// label 의 현재 세션, 없으면 label 그대로
    pub fn current(&self, label: &str) -> String {
        let map = self.current.lock().unwrap_or_else(|e| e.into_inner());
        map.get(label).cloned().unwrap_or_else(|| label.to_string())
    }

    /// 창 사망, 아직 현재 세션이면 제거
    pub fn end(&self, label: &str, session: &str) {
        let mut map = self.current.lock().unwrap_or_else(|e| e.into_inner());
        if map.get(label).map(|s| s.as_str()) == Some(session) {
            map.remove(label);
        }
    }

    /// label 의 창 생존 여부, 파괴 직후 잔존 구간 판별
    pub fn is_live(&self, label: &str) -> bool {
        let map = self.current.lock().unwrap_or_else(|e| e.into_inner());
        map.contains_key(label)
    }

    /// 창이 주장한 세션 토큰 → 소유자 문자열(토큰 없으면 label)
    /// label 재조회 금지 — 지연 IPC 가 새 창의 작업으로 등록(D3.5)
    pub fn resolve(
        &self,
        label: &str,
        claimed: Option<&str>,
    ) -> Result<String, zipmania_archive::ZipManiaError> {
        let map = self.current.lock().unwrap_or_else(|e| e.into_inner());
        let now = map.get(label).cloned().unwrap_or_else(|| label.to_string());
        match claimed {
            Some(c) if c != now => Err(zipmania_archive::ZipManiaError::new(
                "window_closed",
                "창이 닫혀 작업을 시작할 수 없습니다.",
            )),
            _ => Ok(now),
        }
    }
}

/// 해제 창 초기 컨텍스트(아카이브, 선택 항목), open_extract_window 적재 → take_extract_context 1회 회수
#[derive(Default)]
pub struct PendingExtractContext(pub Mutex<Option<ExtractContext>>);

/// 시작 --open 아카이브 보관소, take_startup_open 으로 1회 회수, 이벤트는 신호만(D3.5)
#[derive(Default)]
pub struct PendingStartupOpen(pub Mutex<Option<String>>);

/// 뷰어 창(viewer-N) 상태, next = label 번호, pending = mount 때 회수할 경로, 모달 아님
#[derive(Default)]
pub struct ViewerWindows {
    next: std::sync::atomic::AtomicU32,
    pending: Mutex<std::collections::HashMap<String, String>>,
    opened: Mutex<std::collections::HashMap<String, String>>,
}

/// 경로 비교용 정규화, 대소문자 무시 + 구분자 통일
fn same_path(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.replace('/', "\\").to_lowercase();
    norm(a) == norm(b)
}

/// 세션 아카이브 암호 (경로, 암호), 성공 시점 저장, 모든 창 공유, 다른 아카이브 열면 폐기
#[derive(Default)]
pub struct SessionPassword(pub Mutex<Option<(String, String)>>);

/// 세션 임시 루트(%TEMP%\Ara_<랜덤>), 시작 시 1회 생성, 종료 시 cleanup_temp_root 가 통째 삭제
pub struct TempRoot(pub PathBuf);

impl TempRoot {
    /// 세션 임시 루트 생성
    pub fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("Ara_{:016x}", random_u64()));
        let _ = std::fs::create_dir_all(&dir);
        TempRoot(dir)
    }
}

impl Default for TempRoot {
    fn default() -> Self {
        Self::new()
    }
}

/// 난수 u64. RandomState 시드 이용(외부 crate 없음)
fn random_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish()
}

/// 아카이브 경로 → 임시 폴더명(금지 문자, 끝 공백, 마침표 정리), 같은 항목 = 같은 경로 → 재사용
pub(crate) fn archive_folder_name(archive: &str) -> String {
    let base = archive
        .rsplit(['/', '\\'])
        .find(|s| !s.is_empty())
        .unwrap_or(archive);
    // 금지 문자, 제어 문자 → _
    let name: String = base
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if (c as u32) < 0x20 => '_',
            c => c,
        })
        .collect();
    // 끝 공백, 마침표 제거
    let trimmed = name.trim_end_matches([' ', '.']);
    if trimmed.is_empty() {
        "archive".to_string()
    } else {
        trimmed.to_string()
    }
}

/// 아카이브 절대 경로 해시, 64비트 그대로 쓸 것 — 잘라 쓰면 충돌해 엉뚱한 파일을 연다
pub(crate) fn archive_hash(archive: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    archive.hash(&mut h);
    format!("{:016x}", h.finish())
}

/// 아카이브 전용 임시 폴더 경로(Ara_<랜덤>/<짧은해시>/<압축 파일명>), 폴더 생성은 호출측
pub(crate) fn archive_temp_dir(app: &tauri::AppHandle, archive: &str) -> PathBuf {
    let root = app.state::<TempRoot>().0.clone();
    root.join(archive_hash(archive))
        .join(archive_folder_name(archive))
}

/// 내부 경로 → 안전한 상대 경로(폴더 구조 유지, 안전하지 않으면 None)
/// zipmania_archive::paths::sanitize 사용, 자체 구현 금지
pub(crate) fn inner_rel_path(inner: &str) -> Option<PathBuf> {
    zipmania_archive::paths::sanitize(inner).ok()
}

/// 내부 경로 → base 하위 실제 출력 경로, sanitize + resolve_under 둘 다, 루트 자신이 링크면 거부
/// 앱 열기, 드래그, CF_HDROP 공용 진입점, 분기 사용 금지(D3.14)
pub(crate) fn inner_dest_path(base: &Path, inner: &str) -> Option<PathBuf> {
    if std::fs::symlink_metadata(base)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return None;
    }
    let rel = inner_rel_path(inner)?;
    zipmania_archive::paths::resolve_under(base, &rel).ok()
}

/// 세션 암호 조회(아카이브 일치 시)
fn session_pw_get(app: &tauri::AppHandle, archive: &str) -> Option<String> {
    let st = app.state::<SessionPassword>();
    let guard = st.0.lock().ok()?;
    match &*guard {
        Some((a, p)) if a == archive => Some(p.clone()),
        _ => None,
    }
}

/// 세션 암호 저장(빈 문자열 무시)
fn session_pw_set(app: &tauri::AppHandle, archive: &str, pw: &str) {
    if pw.is_empty() {
        return;
    }
    let st = app.state::<SessionPassword>();
    let mut guard = match st.0.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    *guard = Some((archive.to_string(), pw.to_string()));
}

/// 다른 아카이브의 세션 암호 폐기
fn session_pw_clear_if_other(app: &tauri::AppHandle, archive: &str) {
    let st = app.state::<SessionPassword>();
    let mut guard = match st.0.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    let is_other = matches!(&*guard, Some((a, _)) if a != archive);
    if is_other {
        *guard = None;
    }
}

/// 번들 7z.dll 경로, dev = CARGO_MANIFEST_DIR/binaries, 배포 = exe 옆
pub fn sevenzip_dll_path(app: &tauri::AppHandle) -> Result<PathBuf, SevenZipError> {
    #[cfg(debug_assertions)]
    {
    // app = 릴리스 경로 해석 전용
        let _ = app;
        Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("binaries")
            .join("7z.dll"))
    }

    #[cfg(not(debug_assertions))]
    {
        // 포터블 배포 = exe 옆 평면 배치
        let _ = app;
        let dir = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|p| p.to_path_buf()))
            .ok_or_else(|| SevenZipError::Resource("실행 파일 폴더를 찾지 못했습니다".to_string()))?;
        Ok(dir.join("7z.dll"))
    }
}

/// 7z.dll 경로 주입 Router 생성
fn router(app: &tauri::AppHandle) -> Result<Router, ZipManiaError> {
    let dll = sevenzip_dll_path(app)?;
    Ok(Router::new(dll))
}

/// ZipManiaShell.dll 절대경로, 7z.dll 과 같은 규약(dev = binaries/, 배포 = exe 옆)
pub fn shellext_dll_path(app: &tauri::AppHandle) -> String {
    #[cfg(debug_assertions)]
    {
        let _ = app;
        return PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("binaries")
            .join("ZipManiaShell.dll")
            .to_string_lossy()
            .to_string();
    }
    #[cfg(not(debug_assertions))]
    {
        // 포터블 배포 = exe 옆
        let _ = app;
        std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|d| d.join("ZipManiaShell.dll")))
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "ZipManiaShell.dll".to_string())
    }
}

/// 7z.dll 버전 → 배너 문자열, 예: 7-Zip 26.02 (x64)
#[tauri::command]
pub fn sevenzip_version(app: tauri::AppHandle) -> Result<String, String> {
    let router = router(&app).map_err(|e| e.message)?;
    router.engine_version().map_err(|e| e.message)
}

/// 아카이브 항목 전체 목록, password = 헤더 암호 재시도용(첫 호출 None), 실패 = ZipManiaError
#[tauri::command]
pub fn open_archive(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    path: String,
    password: Option<String>,
) -> Result<Vec<ArchiveEntry>, ZipManiaError> {
    // 창별 열린 파일 기록, 같은 파일 재클릭 시 그 창을 찾는 데 사용
    if let Ok(mut m) = app.state::<ViewerWindows>().opened.lock() {
        m.insert(window.label().to_string(), path.clone());
    }
    list_archive(&app, &path, password)
}

/// open_archive 알맹이, 창 무관 목록 조회, 내부 호출용, 커맨드 직접 호출 금지
pub(crate) fn list_archive(
    app: &tauri::AppHandle,
    path: &str,
    password: Option<String>,
) -> Result<Vec<ArchiveEntry>, ZipManiaError> {
    let path = path.to_string();
    // 다른 아카이브 → 이전 세션 암호 폐기
    session_pw_clear_if_other(app, &path);
    // 암호 미지정 → 세션 암호 폴백
    let password = password.or_else(|| session_pw_get(app, &path));
    let router = router(app)?;
    let entries = router.for_archive(&path).list(&path, password.as_deref())?;
    // 성공 시 세션 보관 → 이후 재사용
    if let Some(pw) = &password {
        session_pw_set(app, &path, pw);
    }
    Ok(entries)
}

/// 무결성 테스트 job 시작 → job_id 즉시 반환
/// 이벤트: job:progress, test:report{entries:[{path,isDir,expectedCrc,actualCrc,ok}]}, job:error
/// 개별 CRC 오류 = 항목 ok=false, 암호 없으면 세션 값 보충, 동시 1작업
#[tauri::command]
pub fn start_test(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    jobs: tauri::State<'_, JobManager>,
    archive: String,
    password: Option<String>,
    // 창 세션 토큰, 창이 닫힌 뒤 도착한 지연 호출 차단, label 재조회 금지
    session: Option<String>,
) -> Result<String, ZipManiaError> {
    let dll = sevenzip_dll_path(&app)?;
    let password = password.or_else(|| session_pw_get(&app, &archive));

    let job_id = jobs.next_id();
    let cancel = jobs.start(&job_id, job_info("test", &archive, session.as_deref(), &webview, &app)?)?;

    let _ = app.emit(
        "job:started",
        JobStarted {
            job_id: job_id.clone(),
            kind: "test".to_string(),
        },
    );

    let app_bg = app.clone();
    let job_for_thread = job_id.clone();
    let archive_bg = archive.clone();
    let pw_bg = password.clone();

    // 가드는 스레드 밖에서 생성, 안에서 만들면 spawn 실패 시 등록이 남아 영구 job_busy
    let job = crate::jobs::JobGuard::new(app.clone(), job_id.clone());
    std::thread::spawn(move || {
        let job = job;
        let router = Router::new(dll);
        let mut on_progress = |percent: u8, current_file: Option<String>| {
            let _ = app_bg.emit(
                "job:progress",
                JobProgress {
                    job_id: job_for_thread.clone(),
                    percent,
                    current_file: current_file.unwrap_or_default(),
                },
            );
        };
        let result = router.for_archive(&archive_bg).test_report(
            &archive_bg,
            pw_bg.as_deref(),
            &mut on_progress,
            cancel,
        );

        // 결과 통지 전에 해제
        job.release();

        match result {
            Ok(entries) => {
                // 성공 시 세션 암호 보관
                if let Some(pw) = &pw_bg {
                    session_pw_set(&app_bg, &archive_bg, pw);
                }
                let _ = app_bg.emit(
                    "test:report",
                    TestReportEvent {
                        job_id: job_for_thread.clone(),
                        entries,
                    },
                );
            }
            Err(err) => {
                let _ = app_bg.emit(
                    "job:error",
                    JobErrorEvent {
                        job_id: job_for_thread.clone(),
                        code: err.code,
                        message: err.message,
                    },
                );
            }
        }
    });

    Ok(job_id)
}

/// AMSI 바이러스 검사 job → job_id 즉시 반환
/// 이벤트: job:progress, scan:report{entries:[{path,isDir,size,status}]}, job:error
/// status = clean|malware|error|skipped, 10MB 미만만 메모리 해제, 나머지 skipped
/// 요건: Windows 10+, AMSI 지원 백신 + 실시간 보호, 동시 1작업
#[tauri::command]
pub fn start_scan(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    jobs: tauri::State<'_, JobManager>,
    archive: String,
    password: Option<String>,
    // 창 세션 토큰, 창이 닫힌 뒤 도착한 지연 호출 차단, label 재조회 금지
    session: Option<String>,
) -> Result<String, ZipManiaError> {
    /// 검사 대상 최대 크기(10MB)
    const MAX_SCAN_SIZE: u64 = 10 * 1024 * 1024;

    let dll = sevenzip_dll_path(&app)?;
    let password = password.or_else(|| session_pw_get(&app, &archive));

    let job_id = jobs.next_id();
    let cancel = jobs.start(&job_id, job_info("scan", &archive, session.as_deref(), &webview, &app)?)?;

    let _ = app.emit(
        "job:started",
        JobStarted {
            job_id: job_id.clone(),
            kind: "scan".to_string(),
        },
    );

    let app_bg = app.clone();
    let job_for_thread = job_id.clone();
    let archive_bg = archive.clone();
    let pw_bg = password.clone();

    // 가드는 스레드 밖에서 생성, 안에서 만들면 spawn 실패 시 등록이 남아 영구 job_busy
    let job = crate::jobs::JobGuard::new(app.clone(), job_id.clone());
    std::thread::spawn(move || {
        let job = job;
        // AMSI 초기화, 제공자(백신)가 없으면 검사 불가
        let session = match crate::amsi::AmsiSession::new("ZipMania") {
            Some(s) => s,
            None => {
                job.release();
                let _ = app_bg.emit(
                    "job:error",
                    JobErrorEvent {
                        job_id: job_for_thread.clone(),
                        code: "amsi_unavailable".to_string(),
                        message: "AMSI 바이러스 검사를 사용할 수 없습니다.".to_string(),
                    },
                );
                return;
            }
        };

        let router = Router::new(dll);
        let mut on_progress = |percent: u8, current_file: Option<String>| {
            let _ = app_bg.emit(
                "job:progress",
                JobProgress {
                    job_id: job_for_thread.clone(),
                    percent,
                    current_file: current_file.unwrap_or_default(),
                },
            );
        };
        // 검사 콜백(각 파일 바이트를 AMSI 로 검사)
        let scan: Box<dyn FnMut(&str, &[u8]) -> String + Send> =
            Box::new(move |name, data| session.scan(name, data));

        let result = router.for_archive(&archive_bg).scan_report(
            &archive_bg,
            pw_bg.as_deref(),
            MAX_SCAN_SIZE,
            scan,
            &mut on_progress,
            cancel,
        );

        // 결과 통지 전에 해제
        job.release();

        match result {
            Ok(entries) => {
                if let Some(pw) = &pw_bg {
                    session_pw_set(&app_bg, &archive_bg, pw);
                }
                let _ = app_bg.emit(
                    "scan:report",
                    ScanReportEvent {
                        job_id: job_for_thread.clone(),
                        entries,
                    },
                );
            }
            Err(err) => {
                let _ = app_bg.emit(
                    "job:error",
                    JobErrorEvent {
                        job_id: job_for_thread.clone(),
                        code: err.code,
                        message: err.message,
                    },
                );
            }
        }
    });

    Ok(job_id)
}

/// 아카이브 편집(add/remove) job → job_id 즉시 반환, 이벤트: job:progress, job:done, job:error
/// 기존 항목 = 재압축 없이 복사, 추가분만 압축, 7z/zip/tar 만, 동시 1작업
// 인자는 평평하게 수신, 구조체 묶기 금지 — 프런트 호출 형태 변경
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub fn start_edit(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    jobs: tauri::State<'_, JobManager>,
    archive: String,
    add: Vec<String>,
    remove: Vec<String>,
    password: Option<String>,
    // 창 세션 토큰, 창이 닫힌 뒤 도착한 지연 호출 차단, label 재조회 금지
    session: Option<String>,
) -> Result<String, ZipManiaError> {
    let dll = sevenzip_dll_path(&app)?;
    let password = password.or_else(|| session_pw_get(&app, &archive));

    let job_id = jobs.next_id();
    let cancel = jobs.start(&job_id, job_info("edit", &archive, session.as_deref(), &webview, &app)?)?;

    let _ = app.emit(
        "job:started",
        JobStarted {
            job_id: job_id.clone(),
            kind: "edit".to_string(),
        },
    );

    let app_bg = app.clone();
    let job_for_thread = job_id.clone();
    let archive_bg = archive.clone();
    let pw_bg = password.clone();

    // 가드는 스레드 밖에서 생성, 안에서 만들면 spawn 실패 시 등록이 남아 영구 job_busy
    let job = crate::jobs::JobGuard::new(app.clone(), job_id.clone());
    std::thread::spawn(move || {
        let job = job;
        let router = Router::new(dll);
        let mut on_progress = |percent: u8, current_file: Option<String>| {
            let _ = app_bg.emit(
                "job:progress",
                JobProgress {
                    job_id: job_for_thread.clone(),
                    percent,
                    current_file: current_file.unwrap_or_default(),
                },
            );
        };
        let opts = EditOptions {
            archive: archive_bg.clone(),
            add,
            remove,
            password: pw_bg.clone(),
        };
        let result = router
            .for_archive(&archive_bg)
            .edit(&opts, &mut on_progress, cancel);

        // 결과 통지 전에 해제
        job.release();

        // 성공(부분 포함)이면 세션 암호 보관
        if let CreateResult::Done { .. } = &result {
            if let Some(pw) = &pw_bg {
                session_pw_set(&app_bg, &archive_bg, pw);
            }
        }
        emit_create_result(&app_bg, &job_for_thread, result);
    });

    Ok(job_id)
}

/// 해제 전 충돌 검사 → 대상 폴더에 이미 있는 내부 경로 목록
/// 암호는 extract 와 같이 세션 값으로 보충 — 한쪽만 보충 시 묻기인데도 덮어쓰기(D3.5)
#[tauri::command]
pub fn check_conflicts(
    app: tauri::AppHandle,
    archive: String,
    dest: String,
    keep_paths: bool,
    selected: Vec<String>,
    password: Option<String>,
) -> Result<Vec<String>, ZipManiaError> {
    let router = router(&app)?;
    let password = password.or_else(|| session_pw_get(&app, &archive));
    router.for_archive(&archive).find_conflicts(
        &archive,
        &dest,
        keep_paths,
        &selected,
        password.as_deref(),
    )
}

/// 해제 job → job_id 즉시 반환
/// 이벤트: job:progress, job:done{status: ok|warning|canceled}, job:error
/// selected 빈 값 = 전체, overwrite = 확정 정책, 동시 1작업
// 인자는 평평하게 수신, 구조체 묶기 금지
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub fn extract(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    jobs: tauri::State<'_, JobManager>,
    archive: String,
    dest: String,
    selected: Vec<String>,
    keep_paths: bool,
    overwrite: String,
    decisions: Option<std::collections::HashMap<String, String>>,
    password: Option<String>,
    // 창 세션 토큰, 창이 닫힌 뒤 도착한 지연 호출 차단, label 재조회 금지
    session: Option<String>,
) -> Result<String, ZipManiaError> {
    // dll 경로 선해석 → 실패 즉시 통지, 백엔드 주입은 스레드 안에서
    let dll = sevenzip_dll_path(&app)?;
    // 암호 없으면 세션 값 재사용, 저장은 성공 뒤에
    let password = password.or_else(|| session_pw_get(&app, &archive));
    // 파일별 충돌 선택, 없으면 overwrite 정책
    let decisions = decisions
        .unwrap_or_default()
        .into_iter()
        .map(|(k, v)| (k.replace('\\', "/"), OverwriteMode::from_str(&v)))
        .collect();
    let opts = ExtractOptions {
        archive,
        dest,
        keep_paths,
        overwrite: OverwriteMode::from_str(&overwrite),
        password,
        selected,
        decisions,
    };

    // 작업 등록(동시 1작업 제한), 실패 시 즉시 오류 반환
    let job_id = jobs.next_id();
    // 임시 파일 = 대상 폴더 옆, 종료 기록이 가리킬 자리
    let cancel = jobs.start(&job_id, job_info("extract", &opts.dest, session.as_deref(), &webview, &app)?)?;

    // 작업 시작 브로드캐스트 → 메인 창 진행률 표시
    let _ = app.emit(
        "job:started",
        JobStarted {
            job_id: job_id.clone(),
            kind: "extract".to_string(),
        },
    );

    let app_bg = app.clone();
    let job_for_thread = job_id.clone();

    // 블로킹 해제 = 별도 스레드, 가드는 스레드 밖에서 생성(spawn 실패 시 영구 job_busy)
    let job = crate::jobs::JobGuard::new(app.clone(), job_id.clone());
    std::thread::spawn(move || {
        let job = job;
        let router = Router::new(dll);
        let mut on_progress = |percent: u8, current_file: Option<String>| {
            let payload = JobProgress {
                job_id: job_for_thread.clone(),
                percent,
                current_file: current_file.unwrap_or_default(),
            };
            let _ = app_bg.emit("job:progress", payload);
        };
        let result = router
            .for_archive(&opts.archive)
            .extract(&opts, &mut on_progress, cancel);

            // 성공 시에만 세션 암호 보관
        if let (Some(pw), ExtractResult::Done { status, .. }) = (&opts.password, &result) {
            if *status == "ok" || *status == "warning" {
                session_pw_set(&app_bg, &opts.archive, pw);
            }
        }

        // 결과 통지 전에 작업 맵에서 제거
        job.release();

        emit_extract_result(&app_bg, &job_for_thread, result);
    });

    Ok(job_id)
}

/// 아카이브 생성 job → job_id 즉시 반환, 이벤트는 해제와 동일
/// output = 확정 경로(덮어쓰기 확인 완료), inputs = basename 이 내부 경로, 동시 1작업
// 인자는 평평하게 수신, 구조체 묶기 금지
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub fn create_archive(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    jobs: tauri::State<'_, JobManager>,
    output: String,
    inputs: Vec<String>,
    format: String,
    level: u8,
    password: Option<String>,
    encrypt_names: bool,
    // 창 세션 토큰, 창이 닫힌 뒤 도착한 지연 호출 차단, label 재조회 금지
    session: Option<String>,
) -> Result<String, ZipManiaError> {
    let dll = sevenzip_dll_path(&app)?;
    let format_str = format.clone();
    let opts = CreateOptions {
        output,
        inputs,
        format: CompressFormat::from_str(&format),
        level,
        password,
        encrypt_names,
    };

    // 작업 등록(동시 1작업 제한), 실패 시 즉시 오류 반환
    let job_id = jobs.next_id();
    // 임시 파일 = 산출물 옆, 종료 기록이 가리킬 자리
    let cancel = jobs.start(&job_id, job_info("compress", &opts.output, session.as_deref(), &webview, &app)?)?;

    // 작업 시작 브로드캐스트 → 메인 창 진행률 표시
    let _ = app.emit(
        "job:started",
        JobStarted {
            job_id: job_id.clone(),
            kind: "compress".to_string(),
        },
    );

    let app_bg = app.clone();
    let job_for_thread = job_id.clone();

    // 가드는 스레드 밖에서 생성, 안에서 만들면 spawn 실패 시 등록이 남아 영구 job_busy
    let job = crate::jobs::JobGuard::new(app.clone(), job_id.clone());
    std::thread::spawn(move || {
        let job = job;
        let router = Router::new(dll);
        let mut on_progress = |percent: u8, current_file: Option<String>| {
            let payload = JobProgress {
                job_id: job_for_thread.clone(),
                percent,
                current_file: current_file.unwrap_or_default(),
            };
            let _ = app_bg.emit("job:progress", payload);
        };
        let result = router
            .for_format(&format_str)
            .create(&opts, &mut on_progress, cancel);

        // 결과 통지 전에 작업 맵에서 제거
        job.release();

        emit_create_result(&app_bg, &job_for_thread, result);
    });

    Ok(job_id)
}

/// 해제 결과 → job:done/job:error 발행
fn emit_extract_result(app: &tauri::AppHandle, job_id: &str, result: ExtractResult) {
    match result {
        ExtractResult::Done { status, message } => {
            let _ = app.emit(
                "job:done",
                JobDone {
                    job_id: job_id.to_string(),
                    status: status.to_string(),
                    message,
                },
            );
        }
        ExtractResult::Failed(err) => {
            let _ = app.emit(
                "job:error",
                JobErrorEvent {
                    job_id: job_id.to_string(),
                    code: err.code,
                    message: err.message,
                },
            );
        }
    }
}

/// 압축 결과 → job:done/job:error 발행
fn emit_create_result(app: &tauri::AppHandle, job_id: &str, result: CreateResult) {
    match result {
        CreateResult::Done { status, message } => {
            let _ = app.emit(
                "job:done",
                JobDone {
                    job_id: job_id.to_string(),
                    status: status.to_string(),
                    message,
                },
            );
        }
        CreateResult::Failed(err) => {
            let _ = app.emit(
                "job:error",
                JobErrorEvent {
                    job_id: job_id.to_string(),
                    code: err.code,
                    message: err.message,
                },
            );
        }
    }
}

/// 작업 취소, 완료는 job:done(status=canceled), 없는 job_id 는 무시
#[tauri::command]
pub fn cancel_job(jobs: tauri::State<'_, JobManager>, job_id: String) -> Result<(), ZipManiaError> {
    jobs.cancel(&job_id);
    Ok(())
}

/// 경로들의 표시용 메타데이터(이름, 크기, 폴더 여부), 디렉터리 = 재귀 없음, size 0. 입력 순서, 개수 보존
#[tauri::command]
pub fn stat_paths(paths: Vec<String>) -> Vec<PathInfo> {
    paths
        .into_iter()
        .map(|p| {
            let pb = PathBuf::from(&p);
            let name = pb
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.clone());
            match std::fs::metadata(&pb) {
                Ok(meta) => {
                    let is_dir = meta.is_dir();
                    PathInfo {
                        name,
                        path: p,
                        size: if is_dir { 0 } else { meta.len() },
                        is_dir,
                    }
                }
                Err(_) => PathInfo {
                    name,
                    path: p,
                    size: 0,
                    is_dir: false,
                },
            }
        })
        .collect()
}

/// 폴더 하위 파일 목록(재귀) → (rel, size), 파일 경로 = 빈 목록, 링크 미추적
#[tauri::command]
pub fn list_folder_files(path: String) -> Vec<FolderFile> {
    let root = PathBuf::from(&path);
    if !root.is_dir() {
        return Vec::new();
    }
    let mut out = Vec::new();
    walk_folder(&root, "", &mut out);
    out
}

/// 폴더 재귀 → FolderFile{rel,size}, 링크 제외, 실패 항목 건너뜀
fn walk_folder(dir: &std::path::Path, rel_prefix: &str, out: &mut Vec<FolderFile>) {
    let read = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return,
    };
    let sep = std::path::MAIN_SEPARATOR;
    for entry in read.flatten() {
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let rel = if rel_prefix.is_empty() {
            name
        } else {
            format!("{rel_prefix}{sep}{name}")
        };
        if meta.is_dir() {
            walk_folder(&entry.path(), &rel, out);
        } else if meta.is_file() {
            out.push(FolderFile { rel, size: meta.len() });
        }
    }
}

/// 하위 디렉터리 목록(이름 순, 파일 제외), path 빈 값 = 루트, has_children = 얕은 판정
#[tauri::command]
pub fn list_dir_children(path: Option<String>) -> Vec<DirNode> {
    match path.as_deref() {
        None | Some("") => dir_roots(),
        Some(p) => read_subdirs(std::path::Path::new(p)),
    }
}

/// 폴더 브라우저 바로가기, 즐겨찾기 + 드라이브 루트, 라벨 번역은 프런트가 kind 로
#[tauri::command]
pub fn list_quick_access() -> Vec<QuickAccess> {
    let mut out = Vec::new();
    if let Some(home) = home_dir() {
        // 즐겨찾기(존재하는 것만), 파일시스템 폴더명은 로케일과 무관하게 영어
        for (kind, sub) in [
            ("desktop", "Desktop"),
            ("documents", "Documents"),
            ("downloads", "Downloads"),
        ] {
            let p = home.join(sub);
            if p.is_dir() {
                out.push(QuickAccess {
                    kind: kind.to_string(),
                    name: sub.to_string(),
                    path: p.to_string_lossy().into_owned(),
                });
            }
        }
        // 홈 폴더 자체
        out.push(QuickAccess {
            kind: "home".to_string(),
            name: home
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| home.to_string_lossy().into_owned()),
            path: home.to_string_lossy().into_owned(),
        });
    }
    // 드라이브 루트(Windows) / /(기타)
    for d in dir_roots() {
        out.push(QuickAccess {
            kind: "drive".to_string(),
            name: d.name,
            path: d.path,
        });
    }
    out
}

/// [새 폴더], parent/name 생성 → 경로 반환, 빈 이름, 구분자, 금지 문자, 중복 = 오류
#[tauri::command]
pub fn create_directory(parent: String, name: String) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("폴더 이름을 입력하세요.".into());
    }
    // 구분자, 금지 문자 차단 → 상위 탈출 방지
    if trimmed.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']) {
        return Err("폴더 이름에 사용할 수 없는 문자가 있습니다.".into());
    }
    let dir = std::path::Path::new(&parent).join(trimmed);
    if dir.exists() {
        return Err("이미 같은 이름의 폴더가 있습니다.".into());
    }
    std::fs::create_dir(&dir).map_err(|e| format!("폴더를 만들지 못했습니다: {e}"))?;
    Ok(dir.to_string_lossy().into_owned())
}

/// 하위 디렉터리 → DirNode(이름 순, 실패, 숨김 제외)
fn read_subdirs(dir: &std::path::Path) -> Vec<DirNode> {
    let read = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for entry in read.flatten() {
        // metadata() = 링크 추적 판정
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if !meta.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        // 숨김 폴더 제외
        if is_hidden(&name, &meta) {
            continue;
        }
        let path = entry.path();
        out.push(DirNode {
            has_children: has_subdir(&path),
            name,
            path: path.to_string_lossy().into_owned(),
        });
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

/// 하위 디렉터리 유무(숨김 제외, 첫 항목에서 중단)
fn has_subdir(dir: &std::path::Path) -> bool {
    match std::fs::read_dir(dir) {
        Ok(read) => read.flatten().any(|e| match e.metadata() {
            Ok(m) => m.is_dir() && !is_hidden(&e.file_name().to_string_lossy(), &m),
            Err(_) => false,
        }),
        Err(_) => false,
    }
}

/// 숨김 폴더 여부, , 시작 또는 Windows 숨김 속성
fn is_hidden(name: &str, meta: &std::fs::Metadata) -> bool {
    if name.starts_with('.') {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
        return meta.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0;
    }
    #[cfg(not(windows))]
    {
        let _ = meta;
        false
    }
}

/// 파일시스템 루트, Windows = 드라이브 문자, 기타 = /
#[cfg(windows)]
fn dir_roots() -> Vec<DirNode> {
    let mut out = Vec::new();
    for c in b'A'..=b'Z' {
        let root = format!("{}:\\", c as char);
        if std::fs::metadata(&root).is_ok() {
            out.push(DirNode {
                name: format!("{}:", c as char),
                path: root,
                has_children: true,
            });
        }
    }
    out
}

/// 파일시스템 루트(비-Windows) = /
#[cfg(not(windows))]
fn dir_roots() -> Vec<DirNode> {
    vec![DirNode {
        name: "/".to_string(),
        path: "/".to_string(),
        has_children: true,
    }]
}

/// 홈 디렉터리, Windows = %USERPROFILE%, 기타 = $HOME
fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE").map(PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

/// 압축 창(label compress) 열기, 이미 열려 있으면 큐 적재 + compress:take-inputs 신호만
/// 모달 = parent(&main) + set_enabled(false) + Destroyed 복구, async 필수(tauri#13963)
#[tauri::command]
pub async fn open_compress_window(
    app: tauri::AppHandle,
    inputs: Vec<String>,
    format: Option<String>,
    output: Option<String>,
    auto_start: Option<bool>,
    batch: Option<Vec<CompressBatchItem>>,
) -> Result<(), String> {
    open_compress_window_inner(
        app,
        Some(CompressLaunch {
            inputs,
            format,
            output,
            auto_start: auto_start.unwrap_or(false),
            batch: batch.unwrap_or_default(),
        }),
    )
}

/// open_compress_window 본체, launch 없으면 창만 연다(죽은 창의 잔여 요청 인계용)
fn open_compress_window_inner(
    app: tauri::AppHandle,
    launch: Option<CompressLaunch>,
) -> Result<(), String> {
    // 창 생성~판정 직렬화, 동시 요청 둘이 각자 만들면 뒤엣것이 label 중복으로 사라진다
    let pending_state = app.state::<PendingCompressInputs>();
    let _create = pending_state
        .create
        .lock()
        .map_err(|e| e.to_string())?;

    // 이미 열림 → 큐 적재 + 신호 + 포커스, 새로 만들지 않는다
    if let Some(win) = app.get_webview_window("compress") {
        {
            let mut guard = pending_state.state.lock().map_err(|e| e.to_string())?;
            // 적재와 판정은 같은 자물쇠 안에서, 밖에서 보면 그 사이 mount 가 보관소를 비운다
            if let Some(l) = launch {
                guard.push(l);
            }
            // mount 전에는 미통지(회수 때 함께 회수)
            if guard.ready {
                let _ = win.emit("compress:take-inputs", ());
            }
        }
        let _ = win.set_focus();
        return Ok(());
    }

    // 모달 부모 = 메인 창
    let main_window = app.get_webview_window("main");

    // 창 생성 직전 초기 컨텍스트 보관
    {
        let mut guard = pending_state.state.lock().map_err(|e| e.to_string())?;
        // 새 창 = 아직 회수 준비 전(mount 의 lease_compress_launch 가 세운다)
        // 큐 비우기 금지 — 남은 요청은 이 창이 순서대로 회수
        guard.ready = false;
        if let Some(l) = launch {
            guard.push(l);
        }
    }

    // 메인 창과 같은 index.html, 창 구분 = 프런트의 label(URL 쿼리 회피)
    let mut builder = WebviewWindowBuilder::new(
        &app,
        "compress",
        WebviewUrl::App("index.html".into()),
    )
    .title("새 압축")
    // 단일 열 폼, 세로 620 유지
    .inner_size(680.0, 620.0)
    .min_inner_size(560.0, 620.0)
    .center()
    // 숨겨서 생성 → 다크 캡션 → 표시
    .visible(false)
    .resizable(true);

    // cli_mode = 메인에 종속시키지 않음
    let cli_mode = app.state::<crate::cli::CliMode>().get();

    // 부모 관계 = 메인 위 유지 + 함께 파괴, cli_mode 는 부모 없음
    if !cli_mode {
        if let Some(ref main) = main_window {
            builder = builder.parent(main).map_err(|e| e.to_string())?;
        }
    }

    // 세션은 build 보다 먼저 발급, 뒤에 두면 새 webview 의 회수가 이전 세션으로 읽힌다
    let session = app.state::<WindowSessions>().begin("compress");

    // build 선행, 실패 시 메인 비활성화 없이 즉시 반환
    let compress = match builder.build() {
        Ok(w) => w,
        Err(e) => {
            // 창 없음 = 세션도 없음
            app.state::<WindowSessions>().end("compress", &session);
            return Err(e.to_string());
        }
    };

    // 다크 캡션 적용 후 표시
    crate::wintheme::apply_window_chrome(&compress);
    let _ = compress.show();
    let _ = compress.set_focus();

    // 파괴 처리는 조건 없이 단다, 조건 안에 두면 작업 취소, 세션 정리, 재오픈이 통째로 빠진다
    let parent = if cli_mode { None } else { main_window.clone() };
    attach_job_window_close(&compress, &app, "compress", &session, parent.clone(), cli_mode);
    if let Some(main) = parent {
        // 모달 = 열린 동안 메인 비활성화
        let _ = main.set_enabled(false);
    }

    Ok(())
}

/// 작업 창의 파괴 처리 등록, 압축/해제, 뷰어 공용, 조건 없이 단다
/// cli_mode 종료는 재오픈이 예정돼 있으면 미룬다(D3.5)
fn attach_job_window_close(
    window: &tauri::WebviewWindow,
    app: &tauri::AppHandle,
    label: &str,
    session: &str,
    parent: Option<tauri::WebviewWindow>,
    cli_mode: bool,
) {
    let app = app.clone();
    let session = session.to_string();
    let label = label.to_string();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::Destroyed = event {
            if let Some(main) = &parent {
                let _ = main.set_enabled(true);
                let _ = main.set_focus();
            }
            let reopening = on_job_window_destroyed(&app, &label, &session);
            if cli_mode && !reopening {
                app.exit(0);
            }
        }
    });
}

/// 작업 창 파괴 공통 처리(판단은 세션으로)
/// ① 작업 취소 + 묘비 ② 보유 요청 정리(적용 전 = 되돌림, 뒤 = 버림) ③ 잔여 요청 있으면 재오픈
/// 반환 = ③ (D3.5)
fn on_job_window_destroyed(app: &tauri::AppHandle, label: &str, session: &str) -> bool {
    // 묘비와 취소는 한 호출, 분리 시 그 틈의 지연 IPC 가 고아 작업 등록
    app.state::<JobManager>().retire_session(session);
    app.state::<WindowSessions>().end(label, session);

    let reopen = {
        let pending = app.state::<PendingCompressInputs>();
        let mut guard = match pending.state.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        guard.release_session(session);
        guard.has_unleased()
    };
    if reopen {
        reopen_compress_after_retire(app.clone(), session.to_string());
    }
    reopen
}

/// 앞 작업이 물러난 뒤 압축 창 재오픈(wait_session_retired, 5초), 곧바로 열면 job_busy(D3.5)
fn reopen_compress_after_retire(app: tauri::AppHandle, session: String) {
    std::thread::spawn(move || {
        app.state::<JobManager>()
            .wait_session_retired(&session, std::time::Duration::from_secs(5));
        // 죽은 창의 매니저 이탈까지 대기, 잔존 시 이미 열림으로 판정돼 요청이 큐에 잔류
        let sessions = app.state::<WindowSessions>();
        for _ in 0..60 {
            match app.get_webview_window("compress") {
                None => break,
                Some(_) if sessions.is_live("compress") => break,
                Some(_) => std::thread::sleep(std::time::Duration::from_millis(50)),
            }
        }
        if open_compress_window_inner(app.clone(), None).is_err() {
            // 창 생성 실패 → CLI 모드면 종료
            if app.state::<crate::cli::CliMode>().get() {
                app.exit(0);
            }
        }
    });
}

/// 각각 압축 출력 경로 계산, <이름>.<ext> 를 각 입력 자리에, 겹치면 이름 (2).ext
/// crate::cli 의 셸 메뉴와 같은 규약, 부모 폴더 없는 경로는 건너뜀
#[tauri::command]
pub fn plan_each_compress(inputs: Vec<String>, format: String) -> Vec<CompressBatchItem> {
    let ext = match format.as_str() {
        "7z" => "7z",
        "tar" => "tar",
        _ => "zip",
    };
    let mut taken: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out = Vec::new();
    for input in inputs {
        let p = PathBuf::from(&input);
        let Some(parent) = p.parent().map(|d| d.to_path_buf()) else {
            continue;
        };
        let stem = p
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "archive".to_string());
        let mut candidate = parent.join(format!("{stem}.{ext}"));
        let mut n = 2;
        while candidate.exists()
            || taken.contains(&candidate.to_string_lossy().to_lowercase())
        {
            candidate = parent.join(format!("{stem} ({n}).{ext}"));
            n += 1;
        }
        taken.insert(candidate.to_string_lossy().to_lowercase());
        out.push(CompressBatchItem {
            input,
            output: candidate.to_string_lossy().to_string(),
        });
    }
    out
}

/// 보관된 요청 1개 대여(큐에서 빼는 것은 ack_compress_launch), 남은 것은 more 로 통지
#[tauri::command]
pub fn lease_compress_launch(
    webview: tauri::Webview,
    session: Option<String>,
    sessions: tauri::State<'_, WindowSessions>,
    pending: tauri::State<'_, PendingCompressInputs>,
) -> Result<CompressTake, ZipManiaError> {
    // 창이 실어 보낸 토큰 검증, label 로 되짚으면 죽은 창의 지연 호출이 새 창 세션으로 읽힌다
    let session = sessions.resolve(webview.label(), session.as_deref())?;
    let mut guard = match pending.state.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    // 이 호출 = 창 준비됨 신호, 대여와 같은 임계구역 필요
    guard.ready = true;
    Ok(guard.lease(&session))
}

/// 실행 개시 통지, 적용 전에 부른다 — 창 사망 시 되돌림/버림을 가른다(D3.5)
#[tauri::command]
pub fn dispatch_compress_launch(
    id: u64,
    gen: u64,
    pending: tauri::State<'_, PendingCompressInputs>,
) -> LeaseAck {
    let mut guard = match pending.state.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    guard.dispatch(id, gen)
}

/// 대여 요청 마감(큐에서 제거), 번호, 세대 일치 시에만
#[tauri::command]
pub fn ack_compress_launch(
    id: u64,
    gen: u64,
    pending: tauri::State<'_, PendingCompressInputs>,
) -> LeaseAck {
    let mut guard = match pending.state.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    guard.ack(id, gen)
}

/// 이 창의 세션 id, 창마다 다르고 재사용되지 않는다
#[tauri::command]
pub fn window_session(
    webview: tauri::Webview,
    sessions: tauri::State<'_, WindowSessions>,
) -> String {
    sessions.current(webview.label())
}

/// 큐 맨 앞 요청이 독립 작업인가(자기 출력, 배치 보유), 소유권은 큐에 둔다, 빈 큐 = false
#[tauri::command]
pub fn peek_compress_standalone(pending: tauri::State<'_, PendingCompressInputs>) -> bool {
    let guard = match pending.state.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    guard.queue.front().is_some_and(|q| is_standalone(&q.launch))
}

/// 요청의 독립 작업 여부, 프런트 compressPlan.js 의 isStandalone 과 동일 규칙 필요
fn is_standalone(l: &CompressLaunch) -> bool {
    !l.batch.is_empty()
        || (l.auto_start
            && l.output.as_deref().is_some_and(|o| !o.is_empty())
            && !l.inputs.is_empty())
}

/// 현재 설정, settings.toml 을 그때그때 읽음(유일 소스), 없거나 손상 시 기본값
#[tauri::command]
pub fn get_settings(app: tauri::AppHandle) -> Settings {
    crate::settings::load(&app)
}

/// 설정 저장, 저장 후 프런트가 settings:changed 를 방송해 다른 창이 즉시 재적용
#[tauri::command]
pub fn save_settings(app: tauri::AppHandle, settings: Settings) -> Result<(), String> {
    crate::settings::save(&app, &settings)
}

/// 셸 확장 레지스트리 ON/OFF(HKCU CLSID, verb), ON = 현재 exe, DLL 경로 등록, 플래그 저장은 save_settings
#[tauri::command]
pub fn sync_shell_integration(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    let dll = shellext_dll_path(&app);
    crate::shell_reg::sync(enabled, &dll);
    Ok(())
}

/// 환경설정 창(label settings) 열기, 초기 데이터는 프런트가 get_settings 로, async 필수(tauri#13963)
#[tauri::command]
pub async fn open_settings_window(app: tauri::AppHandle) -> Result<(), String> {
    // 이미 열려 있으면 포커스만
    if let Some(win) = app.get_webview_window("settings") {
        let _ = win.set_focus();
        return Ok(());
    }

    let main_window = app.get_webview_window("main");

    let mut builder = WebviewWindowBuilder::new(
        &app,
        "settings",
        WebviewUrl::App("index.html".into()),
    )
    .title("환경설정")
    .inner_size(720.0, 520.0)
    .min_inner_size(620.0, 460.0)
    .center()
    // 숨겨서 생성 → 다크 캡션 → 표시
    .visible(false)
    .resizable(true);

    if let Some(ref main) = main_window {
        builder = builder.parent(main).map_err(|e| e.to_string())?;
    }

    let settings_win = builder.build().map_err(|e| e.to_string())?;

    // 다크 캡션 적용 후 표시
    crate::wintheme::apply_window_chrome(&settings_win);
    let _ = settings_win.show();
    let _ = settings_win.set_focus();

        // 환경설정은 즉시 저장 + settings:changed 방송 → 닫을 때 되돌릴 것 없음, 모달 해제만
    if let Some(main) = main_window {
        let main_for_event = main.clone();
        settings_win.on_window_event(move |event| {
            if let tauri::WindowEvent::Destroyed = event {
                let _ = main_for_event.set_enabled(true);
                let _ = main_for_event.set_focus();
            }
        });
        let _ = main.set_enabled(false);
    }

    Ok(())
}

/// 해제 옵션 창(label extract) 열기, 대상은 PendingExtractContext → take_extract_context 회수
/// 이미 열려 있으면 포커스만, async 필수(tauri#13963)
#[tauri::command]
pub async fn open_extract_window(
    app: tauri::AppHandle,
    archive: String,
    selected: Vec<String>,
    dest: Option<String>,
    auto_start: Option<bool>,
    batch: Option<Vec<ExtractBatchItem>>,
) -> Result<(), String> {
    let main_window = app.get_webview_window("main");

    // 컨텍스트 선보관(mount 또는 이벤트에서 1회 회수)
    {
        let pending = app.state::<PendingExtractContext>();
        let mut guard = pending.0.lock().map_err(|e| e.to_string())?;
        *guard = Some(ExtractContext {
            archive,
            selected,
            auto_start: auto_start.unwrap_or(false),
            dest,
            batch: batch.unwrap_or_default(),
        });
    }

        // 포커스만 주고 종료 금지 — 새 요청이 버려져 정지 상태로 보임, 갱신 통지로 재시작 유도
    if let Some(win) = app.get_webview_window("extract") {
        let _ = win.emit("extract:context", ());
        let _ = win.set_focus();
        return Ok(());
    }

    let mut builder = WebviewWindowBuilder::new(
        &app,
        "extract",
        WebviewUrl::App("index.html".into()),
    )
    .title("압축 풀기")
    // 인라인 폴더 트리 포함 → 크게 연다
    .inner_size(640.0, 600.0)
    .min_inner_size(560.0, 520.0)
    .center()
    // 숨겨서 생성 → 다크 캡션 → 표시
    .visible(false)
    .resizable(true);

    // cli_mode = 메인에 종속시키지 않음
    let cli_mode = app.state::<crate::cli::CliMode>().get();

    if !cli_mode {
        if let Some(ref main) = main_window {
            builder = builder.parent(main).map_err(|e| e.to_string())?;
        }
    }

    // 세션은 build 보다 먼저 발급
    let session = app.state::<WindowSessions>().begin("extract");

    let extract = match builder.build() {
        Ok(w) => w,
        Err(e) => {
            app.state::<WindowSessions>().end("extract", &session);
            return Err(e.to_string());
        }
    };

    // 다크 캡션 적용 후 표시
    crate::wintheme::apply_window_chrome(&extract);
    let _ = extract.show();
    let _ = extract.set_focus();

    // 해제 작업도 창과 함께 멈춘다(압축 창과 같은 함수)
    let parent = if cli_mode { None } else { main_window.clone() };
    attach_job_window_close(&extract, &app, "extract", &session, parent.clone(), cli_mode);
    if let Some(main) = parent {
        let _ = main.set_enabled(false);
    }

    Ok(())
}

/// 아카이브를 새 독립 창(viewer-N)에서 열기, 중첩 아카이브용, 현재 창 유지, async 필수(tauri#13963)
#[tauri::command]
pub async fn open_archive_window(app: tauri::AppHandle, path: String) -> Result<(), String> {
    open_viewer_window(&app, path).await
}

/// open_archive_window 알맹이, crate::cli 의 열기 요청도 사용
pub(crate) async fn open_viewer_window(app: &tauri::AppHandle, path: String) -> Result<(), String> {
    use std::sync::atomic::Ordering;

    let state = app.state::<ViewerWindows>();
    let label = format!("viewer-{}", state.next.fetch_add(1, Ordering::Relaxed) + 1);

    // mount 때 회수할 경로 선보관
    state
        .pending
        .lock()
        .map_err(|e| e.to_string())?
        .insert(label.clone(), path.clone());

    // 제목 = 파일명
    let title = std::path::Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "집매니아".into());

    // 세션은 build 보다 먼저 발급 — 창이 곧바로 보내는 IPC 가 label 로 떨어지지 않게
    let session = app.state::<WindowSessions>().begin(&label);

    let window = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("index.html".into()))
        .title(title)
        // 메인 창과 같은 크기
        .inner_size(690.0, 600.0)
        .min_inner_size(690.0, 420.0)
        // 위치 미지정, 가운데 정렬 시 여러 창이 정확히 겹침, 숨겨서 생성 → 다크 캡션 → 표시
        .visible(false)
        .resizable(true)
        .build()
        .map_err(|e| {
            // 창 생성 실패 → 보관 경로 정리
            if let Ok(mut p) = app.state::<ViewerWindows>().pending.lock() {
                p.remove(&label);
            }
            // 창 없음 = 세션도 없음
            app.state::<WindowSessions>().end(&label, &session);
            e.to_string()
        })?;

    // 창 소멸 → 열림 기록 삭제 + 그 창이 시작한 작업 취소
    {
        let app_for_event = app.clone();
        let label_for_event = label.clone();
        window.on_window_event(move |event| {
            if let tauri::WindowEvent::Destroyed = event {
                if let Ok(mut m) = app_for_event.state::<ViewerWindows>().opened.lock() {
                    m.remove(&label_for_event);
                }
                on_job_window_destroyed(&app_for_event, &label_for_event, &session);
            }
        });
    }

    crate::wintheme::apply_window_chrome(&window);
    let _ = window.show();
    let _ = window.set_focus();
    Ok(())
}

/// 탐색기 --open 아카이브를 적절한 창에 연다
/// ① 이미 그 파일을 연 창 → 앞으로 ② 이 클릭으로 시작됨(startup) → 메인 창 ③ 그 외 → 새 창
pub(crate) async fn open_from_shell(app: &tauri::AppHandle, path: String, startup: bool) {
    // 1) 이미 열어 둔 창 찾기
    let existing = app
        .state::<ViewerWindows>()
        .opened
        .lock()
        .ok()
        .and_then(|m| {
            m.iter()
                .find(|(_, p)| same_path(p, &path))
                .map(|(label, _)| label.clone())
        });
    if let Some(label) = existing {
        if let Some(w) = app.get_webview_window(&label) {
            let _ = w.unminimize();
            let _ = w.show();
            let _ = w.set_focus();
            return;
        }
    }

    // 2) 시작 실행이면 메인 창에
    if startup {
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.unminimize();
            let _ = w.show();
            let _ = w.set_focus();
            // 값은 보관소, 신호만 발행, 리스너 미등록이어도 mount 때 회수
            if let Ok(mut g) = app.state::<PendingStartupOpen>().0.lock() {
                *g = Some(path);
            }
            let _ = app.emit_to("main", "shell:open-archive", ());
            return;
        }
    }

    // 3) 새 창
    let _ = open_viewer_window(app, path).await;
}

/// 뷰어 창 mount 시 자기 아카이브 경로 회수(회수 후 제거), 창 판별 = window.label()
#[tauri::command]
pub fn take_viewer_archive(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ViewerWindows>,
) -> Option<String> {
    state.pending.lock().ok()?.remove(window.label())
}

/// 메인 창의 시작 아카이브 회수(1회), mount 직후와 shell:open-archive 양쪽에서 호출
#[tauri::command]
pub fn take_startup_open(pending: tauri::State<'_, PendingStartupOpen>) -> Option<String> {
    let mut guard = match pending.0.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    guard.take()
}

/// 해제 창 mount 시 초기 컨텍스트 회수(회수 후 보관소 비움)
#[tauri::command]
pub fn take_extract_context(
    pending: tauri::State<'_, PendingExtractContext>,
) -> Option<ExtractContext> {
    let mut guard = match pending.0.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    guard.take()
}

/// 파일 1개 삭제, 없거나 실패 = 오류 메시지
#[tauri::command]
pub fn delete_file(path: String) -> Result<(), String> {
    std::fs::remove_file(&path).map_err(|e| format!("파일을 삭제하지 못했습니다: {e}"))
}

/// 폴더를 탐색기로 열기, [대상 폴더 열기] 용
#[tauri::command]
pub fn open_folder(path: String) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // 따옴표로 감싸 전달, raw_arg 는 추가 인용 없음
        std::process::Command::new("explorer")
            .raw_arg(format!("\"{path}\""))
            .spawn()
            .map_err(|e| format!("폴더를 열지 못했습니다: {e}"))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err("지원하지 않는 플랫폼입니다.".into())
    }
}

/// 항목을 세션 임시 루트 하위 결정적 경로에 풀고 실행, 재클릭 시 재사용
/// Ara_<랜덤>/<짧은해시>/<압축 파일명>/<내부경로>
/// 푼 것이 아카이브면 실행 대신 그 경로를 Some 으로 반환(판정 = is_archive_path)
#[tauri::command]
pub async fn open_entry(
    app: tauri::AppHandle,
    archive: String,
    inner_path: String,
) -> Result<Option<String>, String> {
    let dll = sevenzip_dll_path(&app).map_err(|e| e.to_string())?;
    // 세션 암호 재사용
    let password = session_pw_get(&app, &archive);

    // 결정적 경로 + 임시 루트 하위 확인(조상 링크 차단)
    let base = archive_temp_dir(&app, &archive);
    let Some(dest_file) = inner_dest_path(&base, &inner_path) else {
        return Err("잘못된 파일 경로입니다.".into());
    };

    // 이미 풀림 → 재추출 없이 실행
    if !dest_file.is_file() {
        if let Some(parent) = dest_file.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("임시 폴더를 만들지 못했습니다: {e}"))?;
        }
        // 블로킹 추출 = async 런타임 blocking 풀
        let archive_bg = archive.clone();
        let inner_bg = inner_path.clone();
        let dest_bg = dest_file.clone();
        let result = tauri::async_runtime::spawn_blocking(move || {
            let router = Router::new(dll);
            router.for_archive(&archive_bg).extract_entry_to_file(
                &archive_bg,
                &inner_bg,
                &dest_bg,
                password.as_deref(),
            )
        })
        .await
        .map_err(|e| e.to_string())?;

        if let Err(err) = result {
            // 대상을 지우지 않는다 — 백엔드가 StagedFile 로 쓰므로 그 자리의 파일은 정상 캐시다
            return Err(err.message);
        }
    }

    // 임시 루트는 종료 시 통째 삭제 → 개별 정리 불필요

    // 아카이브 외부 반출 금지 — 다른 압축 프로그램으로 이관
    let dest = dest_file.to_string_lossy().to_string();
    if zipmania_archive::is_archive_path(&dest) {
        return Ok(Some(dest));
    }

    // 그 외 = 기본 연결 프로그램 실행
    run_default(&dest_file)?;
    Ok(None)
}

/// 파일 연결 등록/해제(설정 목록 기준), 표시 이름은 등록 시점 언어로 고정
#[tauri::command]
pub fn sync_file_assoc(app: tauri::AppHandle, exts: Vec<String>) -> Result<(), String> {
    let lang = crate::update::language(&app);
    crate::file_assoc::sync(&exts, &lang).map_err(|e| e.to_string())
}

/// 파일 연결 대상 확장자(환경설정 표시 순서), 프런트 목록 중복 기재 금지 — 사본은 SettingsWindow.svelte 하나
#[tauri::command]
pub fn default_assoc_exts() -> Vec<String> {
    crate::file_assoc::DEFAULT_ASSOC_EXTS.iter().map(|s| s.to_string()).collect()
}

/// 확장자 연결 상태, 등록 여부와 UserChoice 차단은 다른 값
#[tauri::command]
pub fn file_assoc_status(exts: Vec<String>) -> Vec<crate::file_assoc::AssocStatus> {
    crate::file_assoc::status(&exts)
}

/// 확장자 1개짜리 [기본 앱 선택] 창, 최대 5초 대기, 실패 = false → 프런트가 open_default_apps 로 폴백
#[tauri::command]
pub fn open_default_app_picker(app: tauri::AppHandle, ext: String) -> Result<bool, String> {
    #[cfg(windows)]
    {
        // 임시 파일 이름에 들어감 → 확장자 모양만 허용
        if ext.is_empty() || !ext.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err("확장자가 올바르지 않습니다".into());
        }

        // 앵커는 settings → main 순으로 떠 있는 창을 찾는다, 화면 밖(-32000) 이동 금지
        let anchor = ["settings", "main"]
            .iter()
            .find_map(|label| {
                let w = app.get_webview_window(label)?;
                if !w.is_visible().unwrap_or(false) {
                    return None;
                }
                let p = w.outer_position().ok()?;
                let s = w.outer_size().ok()?;
                Some((p.x + s.width as i32 / 2, p.y + s.height as i32 / 2))
            })
            .unwrap_or((0, 0));

        Ok(crate::assoc_picker::show(&ext, anchor))
    }
    #[cfg(not(windows))]
    {
        let _ = (app, ext);
        Ok(false)
    }
}

/// 숨긴 속성 창, 임시 파일 정리, 창 재활성화 시 프런트가 호출
#[tauri::command]
pub fn finish_default_app_picker() {
    #[cfg(windows)]
    crate::assoc_picker::finish();
}

/// Windows 기본 앱 설정(ms-settings:defaultapps), 선택 창의 폴백
#[tauri::command]
pub fn open_default_apps() -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("explorer")
            .raw_arg("ms-settings:defaultapps")
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        Err("미지원".into())
    }
}

/// 파일을 기본 연결 프로그램으로 열기
fn run_default(path: &std::path::Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // 따옴표로 감싸 전달, raw_arg 는 추가 인용 없음
        std::process::Command::new("explorer")
            .raw_arg(format!("\"{}\"", path.display()))
            .spawn()
            .map_err(|e| format!("파일을 실행하지 못했습니다: {e}"))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err("지원하지 않는 플랫폼입니다.".into())
    }
}

/// 세션 임시 루트 통째 삭제, RunEvent::Exit 에서 1회, 실패는 무시
pub fn cleanup_temp_root(app: &tauri::AppHandle) {
    let root = app.state::<TempRoot>().0.clone();
    let _ = std::fs::remove_dir_all(&root);
}

/// 작업 신원 생성(종류, 임시 파일 자리, 세션), 창 토큰 검증 → 다르면 오류(D3.5)
fn job_info(
    kind: &'static str,
    target: &str,
    session: Option<&str>,
    webview: &tauri::Webview,
    app: &tauri::AppHandle,
) -> Result<crate::jobs::JobInfo, ZipManiaError> {
    let owner = app
        .state::<WindowSessions>()
        .resolve(webview.label(), session)?;
    Ok(crate::jobs::JobInfo {
        kind,
        target: target.to_string(),
        owner,
    })
}


/// 가상 파일 Shell DnD(지연 렌더링) 시작, 선택 경로 → 파일 항목 → 메인 UI 스레드에서 DoDragDrop
/// 드롭 대상이 요청할 때 추출(임시 파일 없음)
#[cfg(windows)]
#[tauri::command]
pub async fn begin_shell_drag(
    app: tauri::AppHandle,
    archive: String,
    inner_paths: Vec<String>,
    password: Option<String>,
) -> Result<(), String> {
    let dll = sevenzip_dll_path(&app).map_err(|e| e.to_string())?;
    // 암호 없으면 세션 값 재사용
    let password = password.or_else(|| session_pw_get(&app, &archive));

    // 선택 → 실제 파일 항목(폴더는 하위로 펼침)
    let entries = Router::new(dll.clone())
        .for_archive(&archive)
        .list(&archive, password.as_deref())
        .map_err(|e| e.message)?;
    let items = crate::shelldrag::resolve_items(&entries, &inner_paths);
    if items.is_empty() {
        return Err("드래그할 파일이 없습니다.".into());
    }

    let archive_for_drag = archive.clone();
    let app_for_drag = app.clone();
    app.run_on_main_thread(move || {
        // CF_HDROP 요청 시 임시 루트 하위로 추출 → 경로 전달
        let hr =
            crate::shelldrag::do_shell_drag(app_for_drag, archive_for_drag, dll, password, items);
        eprintln!("[shelldrag] DoDragDrop hr = 0x{:08X}", hr.0);
    })
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 비-Windows: 지원하지 않음
#[cfg(not(windows))]
#[tauri::command]
pub async fn begin_shell_drag(
    _app: tauri::AppHandle,
    _archive: String,
    _inner_paths: Vec<String>,
    _password: Option<String>,
) -> Result<(), String> {
    Err("지원하지 않는 플랫폼입니다.".into())
}

/// 탐색기에서 파일 선택 상태로 열기, [원본 파일] 용
#[tauri::command]
pub fn reveal_file(path: String) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // explorer /select,"경로" — 파일이 있는 폴더 열기 + 그 파일 선택
        std::process::Command::new("explorer")
            .raw_arg(format!("/select,\"{path}\""))
            .spawn()
            .map_err(|e| format!("파일 위치를 열지 못했습니다: {e}"))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err("지원하지 않는 플랫폼입니다.".into())
    }
}

/// 확장자, 폴더 여부 → Windows 시스템 아이콘 PNG 데이터 URI(16x16), 폴더 = 빈 ext, 실패 = None
#[tauri::command]
pub fn file_icon(ext: String, is_dir: bool) -> Option<String> {
    use base64::Engine;
    let png = crate::sysicon::icon_png(&ext, is_dir)?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(png);
    Some(format!("data:image/png;base64,{b64}"))
}

/// 미리보기용 내부 이미지 → data:<mime>;base64. 상한은 여기가 강제, 실제 풀린 바이트 기준
const PREVIEW_MAX_BYTES: u64 = 32 * 1024 * 1024;

#[tauri::command]
pub fn read_entry_preview(
    app: tauri::AppHandle,
    archive: String,
    inner_path: String,
    password: Option<String>,
) -> Result<String, ZipManiaError> {
    use base64::Engine;
    let router = router(&app)?;
    let bytes =
        router
            .for_archive(&archive)
            .read_entry_to_memory(&archive, &inner_path, password.as_deref())?;
    if bytes.len() as u64 > PREVIEW_MAX_BYTES {
        return Err(ZipManiaError::new(
            "too_large",
            format!(
                "미리보기 상한({} MiB)을 넘는 파일입니다.",
                PREVIEW_MAX_BYTES / (1024 * 1024)
            ),
        ));
    }
    let mime = image_mime_from_path(&inner_path);
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(format!("data:{mime};base64,{b64}"))
}

/// 확장자 → 이미지 MIME, 모르면 범용 바이너리
fn image_mime_from_path(path: &str) -> &'static str {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" | "jpe" | "jfif" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "svg" => "image/svg+xml",
        "tif" | "tiff" => "image/tiff",
        "avif" => "image/avif",
        _ => "application/octet-stream",
    }
}

// ─────────────────── 임시 해제 경로 정규화 ───────────────────

#[cfg(test)]
mod inner_rel_path_tests {

    /// 임시 캐시 폴더 이름 = 경로 전체 구분
    #[test]
    fn 캐시_해시는_경로를_구분한다() {
        use super::archive_hash;
        let a = archive_hash(r"C:\A\같은이름.zip");
        let b = archive_hash(r"C:\B\같은이름.zip");
        assert_ne!(a, b, "다른 폴더의 같은 이름이 같은 해시가 되었다");
        assert_eq!(a.len(), 16, "해시를 잘라 쓰고 있다: {a}");
        assert_eq!(a, archive_hash(r"C:\A\같은이름.zip"), "같은 경로인데 해시가 다르다");
    }
    use super::inner_rel_path;
    use std::path::{Path, PathBuf};

    fn s(inner: &str) -> Option<String> {
        inner_rel_path(inner).map(|p| p.to_string_lossy().replace('\\', "/"))
    }

    #[test]
    fn 폴더_구조는_유지한다() {
        assert_eq!(s("사진/2026/겨울.png").as_deref(), Some("사진/2026/겨울.png"));
        assert_eq!(s("a\\b\\c.txt").as_deref(), Some("a/b/c.txt"));
    }

    #[test]
    fn 상위_탈출은_거부한다() {
        assert_eq!(s("../탈출.txt"), None);
        assert_eq!(s("a/../../탈출.txt"), None);
        assert_eq!(s(".."), None);
    }

    /// 드라이브 접두사 push 금지, 결합 결과가 임시 루트 안인지 확인
    #[test]
    fn 드라이브_접두사는_상대화한다() {
        let base = Path::new(r"C:\Temp\Ara_x\hash\arc.zip");
        for evil in [r"C:\Windows\notepad.exe", "C:/Windows/notepad.exe", "/Windows/notepad.exe"] {
            let rel = inner_rel_path(evil).expect("상대화되어야 한다");
            assert!(rel.is_relative(), "{evil} → 상대경로가 아니다: {rel:?}");
            let joined: PathBuf = base.join(&rel);
            assert!(
                joined.starts_with(base),
                "{evil} → 임시 루트를 벗어났다: {joined:?}"
            );
        }
    }

    /// 조상에 낀 링크 거부
    #[cfg(windows)]
    #[test]
    fn 조상에_낀_링크는_거부한다() {
        use super::{inner_dest_path, inner_rel_path};
        let root = std::env::temp_dir().join(format!("zm_junc_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let base = root.join("base");
        let outside = root.join("outside");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("victim.txt"), "원본").unwrap();

        // 정션은 관리자 권한 없이 만들어진다(심볼릭 링크와 다르다)
        let link = base.join("link");
        let made = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&link)
            .arg(&outside)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        assert!(made, "정션을 만들지 못했다(테스트 전제가 깨졌다)");

        // 문자열 검사는 통과 — 이것만으로는 부족
        assert!(
            inner_rel_path("link/victim.txt").is_some(),
            "sanitize 는 통과해야 이 테스트가 의미가 있다"
        );
        // 루트 하위 확인에서 거부
        assert!(
            inner_dest_path(&base, "link/victim.txt").is_none(),
            "임시 루트 밖을 가리키는 경로를 내줬다"
        );
        // 링크 없는 평범한 경로는 그대로 통과
        let good = inner_dest_path(&base, "sub/ok.txt").expect("정상 경로를 거부했다");
        let root_abs = base.canonicalize().unwrap_or_else(|_| base.clone());
        assert!(good.starts_with(&root_abs), "정상 경로가 루트 밖이다: {good:?}");

        assert_eq!(
            std::fs::read_to_string(outside.join("victim.txt")).unwrap(),
            "원본",
            "검사만 했는데 대상이 바뀌었다"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 루트 자신이 링크여도 거부
    #[cfg(windows)]
    #[test]
    fn 루트_자신이_링크여도_거부한다() {
        use super::inner_dest_path;
        let root = std::env::temp_dir().join(format!("zm_junc_root_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let outside = root.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("victim.txt"), "원본").unwrap();

        // base 자리를 바깥으로 향하는 정션으로 생성(아직 생성 전 가정)
        let base = root.join("base");
        let made = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&base)
            .arg(&outside)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        assert!(made, "정션을 만들지 못했다(테스트 전제가 깨졌다)");

        assert!(
            inner_dest_path(&base, "victim.txt").is_none(),
            "루트가 정션인데 경로를 내줬다"
        );
        assert_eq!(
            std::fs::read_to_string(outside.join("victim.txt")).unwrap(),
            "원본"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}


// ─────────────────── 압축 요청 큐 ───────────────────

#[cfg(test)]
mod compress_queue_tests {
    use super::*;

    const W1: &str = "compress#1";
    const W2: &str = "compress#2";

    fn launch(name: &str) -> CompressLaunch {
        CompressLaunch {
            inputs: vec![format!("C:/{name}.txt")],
            format: Some("zip".to_string()),
            output: Some(format!("C:/{name}.zip")),
            auto_start: true,
            batch: Vec::new(),
        }
    }

    /// 정상 경로 한 벌, 대여 → 넘김 → 마감
    fn run(q: &mut PendingCompress, session: &str) -> CompressTake {
        let take = q.lease(session);
        let (id, gen) = (take.id, take.gen);
        assert_eq!(q.dispatch(id, gen), LeaseAck::Ok);
        assert_eq!(q.ack(id, gen), LeaseAck::Ok);
        take
    }

    /// 요청 하나 = 작업 하나, 병합 시 서로 다른 작업이 하나로
    #[test]
    fn 요청은_합쳐지지_않고_순서대로_나온다() {
        let mut q = PendingCompress::default();
        q.push(launch("A"));
        q.push(launch("B"));

        let first = q.lease(W1);
        assert!(first.more, "뒤에 남은 요청을 알려야 한다");
        let a = first.launch.clone().expect("첫 요청");
        assert_eq!(a.inputs, vec!["C:/A.txt"], "입력이 합쳐졌다");
        assert_eq!(a.output.as_deref(), Some("C:/A.zip"), "출력이 덮였다");
        assert_eq!(q.dispatch(first.id, first.gen), LeaseAck::Ok);
        assert_eq!(q.ack(first.id, first.gen), LeaseAck::Ok, "마감이 되지 않았다");

        let second = q.lease(W1);
        assert!(!second.more, "마지막이면 더 없다고 해야 한다");
        let b = second.launch.clone().expect("둘째 요청");
        assert_eq!(b.output.as_deref(), Some("C:/B.zip"));
        assert_eq!(q.ack(second.id, second.gen), LeaseAck::Ok);

        assert!(q.lease(W1).launch.is_none(), "다 비운 뒤에는 줄 것이 없다");
    }

    /// 넘기기 전에 창이 죽으면 다음 창이 다시 받는다
    #[test]
    fn 넘기기_전에_죽은_창의_요청은_되돌아온다() {
        let mut q = PendingCompress::default();
        q.push(launch("A"));

        let took = q.lease(W1);
        assert!(took.launch.is_some());
        assert_eq!(q.release_session(W1), 1, "넘기기 전 요청을 되돌려야 한다");
        assert!(q.has_unleased(), "아무도 들고 있지 않은 요청이 남아야 한다");

        let again = q.lease(W2);
        assert_eq!(
            again.launch.expect("다시 받아야 한다").output.as_deref(),
            Some("C:/A.zip")
        );
        assert_eq!(q.ack(again.id, again.gen), LeaseAck::Ok);
    }

    /// 넘긴 뒤 죽은 창의 요청은 폐기(되돌리면 두 번 압축)
    #[test]
    fn 넘긴_요청은_되돌리지_않는다() {
        let mut q = PendingCompress::default();
        q.push(launch("A"));
        let l = q.lease(W1);
        assert_eq!(q.dispatch(l.id, l.gen), LeaseAck::Ok);

        assert_eq!(q.release_session(W1), 0, "넘긴 것을 되돌렸다");
        assert!(q.queue.is_empty(), "넘긴 요청이 큐에 남았다");
        assert!(!q.has_unleased());
    }

    /// 지난 세대의 마감 무시
    #[test]
    fn 지난_세대의_마감은_무시한다() {
        let mut q = PendingCompress::default();
        q.push(launch("A"));

        let old = q.lease(W1);
        q.release_session(W1); // 옛 창 사망(넘기기 전이라 되돌아옴)
        let new = q.lease(W2);
        assert_ne!(old.gen, new.gen, "다시 빌려줬으면 세대가 달라야 한다");

        assert_eq!(
            q.ack(old.id, old.gen),
            LeaseAck::Stale,
            "지난 세대의 마감이 먹었다"
        );
        assert_eq!(q.queue.len(), 1, "새 창의 요청이 지워졌다");
        assert_eq!(q.ack(new.id, new.gen), LeaseAck::Ok, "새 창의 마감은 먹어야 한다");
    }

    /// 응답만 유실된 재시도와 남의 요청 구분(Already 와 Stale)
    #[test]
    fn 마감_재시도는_이미_처리됨으로_답한다() {
        let mut q = PendingCompress::default();
        q.push(launch("A"));
        let l = q.lease(W1);
        assert_eq!(q.dispatch(l.id, l.gen), LeaseAck::Ok);
        assert_eq!(q.ack(l.id, l.gen), LeaseAck::Ok);

        // 응답 유실로 인한 재호출 — 이미 처리됨을 알려 진행 유도
        assert_eq!(q.ack(l.id, l.gen), LeaseAck::Already);
        // 넘김 알림도 같다(마감까지 끝난 뒤 재시도)
        assert_eq!(q.dispatch(l.id, l.gen), LeaseAck::Already);
        // 무관한 번호는 여전히 남의 것
        assert_eq!(q.ack(999, 999), LeaseAck::Stale);
    }

    /// 넘김 알림도 멱등
    #[test]
    fn 넘김_알림_재시도는_이미_넘김으로_답한다() {
        let mut q = PendingCompress::default();
        q.push(launch("A"));
        let l = q.lease(W1);
        assert_eq!(q.dispatch(l.id, l.gen), LeaseAck::Ok);
        assert_eq!(q.dispatch(l.id, l.gen), LeaseAck::Already);
        assert_eq!(q.dispatch(l.id, l.gen + 7), LeaseAck::Stale, "세대가 다르면 남의 것이다");
    }

    /// 같은 창의 회수 재시도는 같은 번호, 세대를 받는다
    #[test]
    fn 같은_창의_회수_재시도는_같은_세대다() {
        let mut q = PendingCompress::default();
        q.push(launch("A"));

        let first = q.lease(W1);
        let retry = q.lease(W1);
        assert_eq!((first.id, first.gen), (retry.id, retry.gen));
        assert_eq!(
            retry.launch.expect("같은 값을 다시 줘야 한다").output.as_deref(),
            Some("C:/A.zip")
        );
    }

    /// 살아 있는 다른 창의 요청을 뺏지 않는다
    #[test]
    fn 살아있는_창의_요청은_뺏지_않는다() {
        let mut q = PendingCompress::default();
        q.push(launch("A"));

        let mine = q.lease(W1);
        let other = q.lease(W2);
        assert!(other.launch.is_none(), "남이 들고 있는 것을 가져갔다");

        // 원래 창의 마감은 그대로 수용
        assert_eq!(q.dispatch(mine.id, mine.gen), LeaseAck::Ok);
        assert_eq!(q.ack(mine.id, mine.gen), LeaseAck::Ok);
    }

    /// 창 세대 경합 — W1 빌림 → W1 죽음(되돌아옴) → W2 빌림 → 그제야 W1 의 지연 넘김, 마감 도착
    #[test]
    fn 죽은_창의_지연_호출은_새_창의_요청을_건드리지_못한다() {
        let mut q = PendingCompress::default();
        q.push(launch("A"));

        let w1 = q.lease(W1);
        q.release_session(W1);
        let w2 = q.lease(W2);

        assert_eq!(q.dispatch(w1.id, w1.gen), LeaseAck::Stale, "죽은 창이 넘김을 세웠다");
        assert_eq!(q.ack(w1.id, w1.gen), LeaseAck::Stale, "죽은 창이 마감했다");
        // W2 는 아무 영향 없이 자기 것을 끝낸다
        assert_eq!(q.dispatch(w2.id, w2.gen), LeaseAck::Ok);
        assert_eq!(q.ack(w2.id, w2.gen), LeaseAck::Ok);
        assert!(q.queue.is_empty());
    }

    /// 배치 요청과 일반 요청 미혼합(각각 자기 작업으로 산출)
    #[test]
    fn 배치와_일반_요청은_섞이지_않는다() {
        let mut q = PendingCompress::default();
        q.push(CompressLaunch {
            inputs: Vec::new(),
            format: Some("zip".into()),
            output: None,
            auto_start: true,
            batch: vec![CompressBatchItem {
                input: "C:/x.txt".into(),
                output: "C:/x.zip".into(),
            }],
        });
        q.push(launch("plain"));

        let first = q.lease(W1).launch.expect("배치 요청");
        assert_eq!(first.batch.len(), 1, "배치가 사라졌다");
        assert!(first.inputs.is_empty(), "일반 입력이 배치에 딸려 왔다");
        let l = q.lease(W1);
        assert_eq!(q.ack(l.id, l.gen), LeaseAck::Ok);

        let second = q.lease(W1).launch.expect("일반 요청");
        assert!(second.batch.is_empty(), "일반 요청에 배치가 남았다");
        assert_eq!(second.inputs, vec!["C:/plain.txt"]);
    }

    /// 창 개폐 판정 = 아무도 들고 있지 않은 요청
    #[test]
    fn 재오픈_판단은_들고_있지_않은_요청으로_한다() {
        let mut q = PendingCompress::default();
        assert!(!q.has_unleased(), "빈 큐로 창을 열면 안 된다");

        q.push(launch("A"));
        assert!(q.has_unleased(), "전하지 못한 요청이 있다");

        let l = q.lease(W1);
        assert!(!q.has_unleased(), "살아 있는 창이 들고 있는데 또 열려 한다");

        q.release_session(W1);
        assert!(q.has_unleased(), "죽은 창이 남긴 요청으로는 열어야 한다");
        let _ = l;
    }

    /// 독립 판정(is_standalone) = 프런트 compressPlan.js 와 동일 규칙 필요
    #[test]
    fn 독립_요청_판정이_프런트와_같다() {
        // 배치는 언제나 독립
        let mut l = launch("A");
        l.batch = vec![CompressBatchItem {
            input: "C:/x".into(),
            output: "C:/x.zip".into(),
        }];
        assert!(is_standalone(&l));

        // 자동 시작 + 출력 + 입력이 다 있어야 독립
        let full = launch("A");
        assert!(is_standalone(&full));

        let mut no_auto = launch("A");
        no_auto.auto_start = false;
        assert!(!is_standalone(&no_auto));

        let mut no_out = launch("A");
        no_out.output = None;
        assert!(!is_standalone(&no_out));

        // 빈 문자열 출력은 출력이 아니다(양쪽 같은 규칙)
        let mut empty_out = launch("A");
        empty_out.output = Some(String::new());
        assert!(!is_standalone(&empty_out));

        let mut no_input = launch("A");
        no_input.inputs.clear();
        assert!(!is_standalone(&no_input));
    }

    /// 정상 경로 한 벌이 큐를 비운다(도우미가 실제로 그 순서로 도는지 확인)
    #[test]
    fn 정상_경로는_큐를_비운다() {
        let mut q = PendingCompress::default();
        q.push(launch("A"));
        let t = run(&mut q, W1);
        assert!(t.launch.is_some());
        assert!(q.queue.is_empty());
    }
}
