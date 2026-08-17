//! COM 콜백 구현 — OpenCb, ExtractCb(해제, 테스트, 대상
//! 스트림, 덮어쓰기 정책, 진행률, 취소, opResult), UpdateCb
//!
//! 진행률 = ProgressSink 원시 포인터로 호출측 클로저에 직접 전달
//! 콜백 객체가 Extract/UpdateItems 동기 호출 동안(단일 스레드)만 생존 → 포인터 유효

use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use windows_core::{Interface, BSTR, HRESULT};

use super::com::*;
use super::crc32;
use super::ffi::to_wide_nul;
use super::prop::PropVariant;
use super::streams;
use super::OverwriteMode;
use std::collections::HashMap;
use crate::models::{ScanEntry, TestEntry};

/// 바이러스 검사 콜백 타입 — (내부경로, 파일 바이트) → 상태문자열, 앱이 AMSI 검사 로직을 여기
/// 주입(clean/malware/error)
pub use crate::formats::ScanFn;

/// 진행률 클로저로의 원시 포인터, 작업(Extract/UpdateItems) 동안만 유효하다
#[derive(Clone, Copy)]
pub struct ProgressSink(*mut (dyn FnMut(u8, Option<String>) + 'static));

impl ProgressSink {
    /// &mut dyn FnMut 에서 생성, 반환 값은 그 참조 생존 동안(동기 작업 범위)만 유효
    pub fn new<'a>(f: &mut (dyn FnMut(u8, Option<String>) + 'a)) -> Self {
        let p: *mut (dyn FnMut(u8, Option<String>) + 'a) = f;
        // 라이프타임만 제거(레이아웃 동일) + 동기 작업 범위 내 호출 한정
        ProgressSink(unsafe { std::mem::transmute(p) })
    }
    unsafe fn call(&self, percent: u8, file: Option<String>) {
        (*self.0)(percent, file);
    }
}

// 암호를 BSTR 로 기록(7z 가 SysFreeString 으로 정리), 없으면 널 + S_FALSE
unsafe fn write_password(pw: &Option<Vec<u16>>, out: *mut *const u16) -> HRESULT {
    match pw {
        Some(wide) => {
            let s = String::from_utf16_lossy(&wide[..wide.len().saturating_sub(1)]);
            let b = BSTR::from(s);
            if !out.is_null() {
                *out = b.into_raw();
            }
            S_OK
        }
        None => {
            if !out.is_null() {
                *out = std::ptr::null();
            }
            S_FALSE
        }
    }
}

fn basename(p: &str) -> String {
    p.rsplit(['/', '\\']).next().unwrap_or(p).to_string()
}

// ─────────────────────────── 열기 콜백 ───────────────────────────

#[windows_core::implement(IArchiveOpenCallback, ICryptoGetTextPassword)]
struct OpenCb {
    password: Option<Vec<u16>>,
    crypto_requested: Arc<AtomicBool>,
}

impl IArchiveOpenCallback_Impl for OpenCb_Impl {
    unsafe fn SetTotal(&self, _f: *const u64, _b: *const u64) -> HRESULT {
        S_OK
    }
    unsafe fn SetCompleted(&self, _f: *const u64, _b: *const u64) -> HRESULT {
        S_OK
    }
}

impl ICryptoGetTextPassword_Impl for OpenCb_Impl {
    unsafe fn CryptoGetTextPassword(&self, password: *mut *const u16) -> HRESULT {
        self.crypto_requested.store(true, Ordering::SeqCst);
        write_password(&self.password, password)
    }
}

/// 열기 콜백 생성, 반환된 crypto_requested 로 헤더암호 여부 판정
pub fn make_open_cb(password: Option<&str>) -> (IArchiveOpenCallback, Arc<AtomicBool>) {
    let flag = Arc::new(AtomicBool::new(false));
    let cb: IArchiveOpenCallback = OpenCb {
        password: password.map(to_wide_nul),
        crypto_requested: flag.clone(),
    }
    .into();
    (cb, flag)
}

// ─────────────────────────── 해제/테스트 콜백 ───────────────────────────

/// 작업 후 읽는 공유 상태
#[derive(Clone)]
pub struct ExtractShared {
    pub op_result: Arc<Mutex<i32>>,
    pub crypto_requested: Arc<AtomicBool>,
    pub aborted: Arc<AtomicBool>,
    pub unsafe_paths: Arc<Mutex<Vec<String>>>,
    pub failed_paths: Arc<Mutex<Vec<(String, String)>>>,
    pending: Arc<Mutex<Option<PendingStaged>>>,
}

/// 임시 파일에 쓰는 중인 항목 하나
struct PendingStaged {
    staged: crate::outfile::StagedFile,
    path: String,
    ok: bool,
    expected: Option<u64>,
}

impl ExtractShared {
    /// 대기 항목 마무리 — 성공이면 옮기고 아니면 임시 파일만 지운다
    /// 스트림이 풀린 뒤에 부른다(다음 항목의 GetStream 과 Extract 직후 두 곳)
    pub fn settle_pending(&self) {
        let Some(p) = self.pending.lock().unwrap().take() else {
            return;
        };
        if !p.ok {
            p.staged.abort();
            return;
        }
        // 신고보다 적게 나왔으면 옮기지 않는다(검사는 commit 보다 먼저)
        // != 가 아니라 < — 더 나오는 정상 포맷 존재(D3.14)
        if let Some(want) = p.expected {
            let got = std::fs::metadata(p.staged.path()).map(|m| m.len()).unwrap_or(0);
            if got < want {
                p.staged.abort();
                self.failed_paths.lock().unwrap().push((
                    p.path,
                    format!("목록은 {want}바이트인데 {got}바이트만 나왔습니다"),
                ));
                return;
            }
        }
        if let Err(e) = p.staged.commit() {
            self.failed_paths.lock().unwrap().push((p.path, e.message));
        }
    }
}

/// 무결성 테스트(CRC 보고) 구성, 설정 시 모든 파일을 CRC 계산 싱크로 "해제"(디스크 미기록)하고
/// 파일별 결과 수집(대상 폴더, 덮어쓰기 로직 제외), test_mode 와 타깃 미사용
pub struct CrcReportCfg {
    pub expected: Arc<Vec<Option<u32>>>,
    pub out: Arc<Mutex<Vec<TestEntry>>>,
}

/// 처리 중인 한 항목의 상태(SetOperationResult 에서 CRC, 상태를 확정하려고 기억)
struct CurrentItem {
    path: String,
    is_dir: bool,
    expected_crc: Option<u32>,
    crc: Option<Arc<Mutex<crc32::Crc32>>>,
}

/// 콜백 내부의 CRC 보고 상태
struct CrcReportState {
    expected: Arc<Vec<Option<u32>>>,
    out: Arc<Mutex<Vec<TestEntry>>>,
    current: Mutex<Option<CurrentItem>>,
}

/// 바이러스 검사(AMSI) 구성, 설정 시 각 파일을 메모리로 해제(max_size 미만) 후 검사 콜백에 전달
/// max_size 이상 파일 = 검사 건너뛰기 + skipped 기록
pub struct ScanReportCfg {
    pub max_size: u64,
    pub sizes: Arc<Vec<Option<u64>>>,
    pub scan: ScanFn,
    pub out: Arc<Mutex<Vec<ScanEntry>>>,
    pub seen: Arc<Mutex<std::collections::HashSet<u32>>>,
}

/// 검사 중인 한 파일의 상태(SetOperationResult 에서 검사 실행)
struct ScanCurrent {
    index: u32,
    path: String,
    size: Option<u64>,
    buf: Arc<Mutex<Vec<u8>>>,
}

/// 콜백 내부의 검사 보고 상태
struct ScanReportState {
    max_size: u64,
    sizes: Arc<Vec<Option<u64>>>,
    scan: Mutex<ScanFn>,
    out: Arc<Mutex<Vec<ScanEntry>>>,
    seen: Arc<Mutex<std::collections::HashSet<u32>>>,
    current: Mutex<Option<ScanCurrent>>,
}

/// 해제 콜백 구성
pub struct ExtractCfg<'a> {
    pub entries: Arc<Vec<(String, bool)>>,
    pub sizes: Arc<Vec<Option<u64>>>,
    pub dest: PathBuf,
    pub keep_paths: bool,
    pub overwrite: OverwriteMode,
    pub decisions: HashMap<String, OverwriteMode>,
    pub test_mode: bool,
    pub password: Option<&'a str>,
    pub progress: Option<ProgressSink>,
    pub cancel: Arc<AtomicBool>,
    pub crc_report: Option<CrcReportCfg>,
    pub scan_report: Option<ScanReportCfg>,
    pub mem_target: Option<(u32, Arc<Mutex<Vec<u8>>>)>,
    pub file_target: Option<(u32, PathBuf)>,
    pub writer_target: Option<(u32, Box<dyn std::io::Write + Send>)>,
}

#[windows_core::implement(IArchiveExtractCallback, ICryptoGetTextPassword)]
struct ExtractCb {
    entries: Arc<Vec<(String, bool)>>,
    sizes: Arc<Vec<Option<u64>>>,
    dest: PathBuf,
    keep_paths: bool,
    overwrite: OverwriteMode,
    decisions: HashMap<String, OverwriteMode>,
    test_mode: bool,
    password: Option<Vec<u16>>,
    total: Mutex<u64>,
    current_file: Mutex<Option<String>>,
    progress: Option<ProgressSink>,
    shared: ExtractShared,
    cancel: Arc<AtomicBool>,
    mem_target: Option<(u32, Arc<Mutex<Vec<u8>>>)>,
    file_target: Option<(u32, PathBuf)>,
    writer_target: Mutex<Option<(u32, Box<dyn std::io::Write + Send>)>>,
    crc_report: Option<CrcReportState>,
    scan_report: Option<ScanReportState>,
}

impl ExtractCb {
    /// 쓰지 못한 항목 기록, 7z 에는 널 스트림(= 건너뛰기)만 반환 가능 → 여기서 남기지
    /// 않으면 그 항목의 누락 사실이 어디에도 미기록
    fn record_failure(&self, path: &str, why: &dyn std::fmt::Display) {
        self.shared
            .failed_paths
            .lock()
            .unwrap()
            .push((path.replace('\\', "/"), why.to_string()));
    }

    fn abort_if_canceled(&self) -> bool {
        if self.cancel.load(Ordering::SeqCst) {
            self.shared.aborted.store(true, Ordering::SeqCst);
            true
        } else {
            false
        }
    }
}

impl IArchiveExtractCallback_Impl for ExtractCb_Impl {
    unsafe fn SetTotal(&self, total: u64) -> HRESULT {
        *self.total.lock().unwrap() = total;
        S_OK
    }

    unsafe fn SetCompleted(&self, complete: *const u64) -> HRESULT {
        if self.abort_if_canceled() {
            return E_ABORT;
        }
        if let Some(sink) = &self.progress {
            let total = *self.total.lock().unwrap();
            let c = if complete.is_null() { 0 } else { *complete };
            let percent = if total > 0 {
                ((c.saturating_mul(100)) / total).min(100) as u8
            } else {
                0
            };
            let file = self.current_file.lock().unwrap().clone();
            sink.call(percent, file);
        }
        S_OK
    }

    unsafe fn GetStream(
        &self,
        index: u32,
        out_stream: *mut *mut c_void,
        ask_mode: i32,
    ) -> HRESULT {
        if !out_stream.is_null() {
            *out_stream = std::ptr::null_mut();
        }
        // 앞 항목의 임시 파일 마무리 지점, 7z 이 그 스트림을 이미 놓은 시점 (다음
        // 항목 요청 = 앞 항목 종료의 뜻)
        self.shared.settle_pending();
        if self.abort_if_canceled() {
            return E_ABORT;
        }
        let (path, is_dir) = match self.entries.get(index as usize) {
            Some(x) => (x.0.as_str(), x.1),
            None => return S_OK,
        };
        *self.current_file.lock().unwrap() = Some(path.replace('\\', "/"));

        // 무결성 테스트(CRC 보고) 모드: 모든 파일을 CRC 계산 싱크로 해제(디스크 미기록)
        if let Some(rep) = &self.crc_report {
            let expected = rep.expected.get(index as usize).copied().flatten();
            if ask_mode == ASK_EXTRACT && !is_dir {
                let crc = Arc::new(Mutex::new(crc32::Crc32::new()));
                *rep.current.lock().unwrap() = Some(CurrentItem {
                    path: path.replace('\\', "/"),
                    is_dir,
                    expected_crc: expected,
                    crc: Some(crc.clone()),
                });
                let s = streams::output_writer(Box::new(crc32::CrcWriter::new(crc)));
                if !out_stream.is_null() {
                    *out_stream = s.into_raw();
                }
            } else {
                // 폴더/건너뛰기: 스트림 없이 현재 항목만 기록
                *rep.current.lock().unwrap() = Some(CurrentItem {
                    path: path.replace('\\', "/"),
                    is_dir,
                    expected_crc: expected,
                    crc: None,
                });
            }
            return S_OK;
        }

        // 바이러스 검사 모드: 각 파일(크기 미만)을 메모리로 풀어 검사 대상 버퍼에 담는다
        if let Some(rep) = &self.scan_report {
            if is_dir {
                return S_OK; // 폴더는 검사 대상 아님(표에서도 제외)
            }
            let size = rep.sizes.get(index as usize).copied().flatten();
            // 신고 크기가 작다는 말만 믿지 않는다, 그 값은 아카이브가 정하는 것이라 "1KB"라고 적어
            // 놓고 수 GB 를 흘려보낼 수 있고, kpidSize 를 읽지 못한 항목은 크기를 아예 모른다
            // 모르는 것은 읽어 보되 같은 한도로 자른다 — 넘으면 스트림이 실패해 그 항목이
            // error(검사하지 못함)로 기록
            let within = scan_admits(size, rep.max_size);
            if ask_mode == ASK_EXTRACT && within {
                let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
                *rep.current.lock().unwrap() = Some(ScanCurrent {
                    index,
                    path: path.replace('\\', "/"),
                    size,
                    buf: buf.clone(),
                });
                let s = streams::mem_out_capped(buf, rep.max_size);
                if !out_stream.is_null() {
                    *out_stream = s.into_raw();
                }
            } else {
                // 크기 초과 → 검사 건너뜀, 즉시 기록하고 스트림은 넘기지 않는다(해제 생략)
                rep.out.lock().unwrap().push(ScanEntry {
                    path: path.replace('\\', "/"),
                    is_dir: false,
                    size: size.unwrap_or(0),
                    status: "skipped".to_string(),
                });
                rep.seen.lock().unwrap().insert(index);
                *rep.current.lock().unwrap() = None;
            }
            return S_OK;
        }

        // 메모리 추출 모드: 대상 인덱스만 메모리 버퍼로, 나머지는 널
        if let Some((target, buf)) = &self.mem_target {
            if ask_mode == ASK_EXTRACT && index == *target && !is_dir {
                let s = streams::mem_out_from(buf.clone());
                if !out_stream.is_null() {
                    *out_stream = s.into_raw();
                }
            }
            return S_OK;
        }

        // 파일 스트리밍 추출: 대상 인덱스만 지정 파일로, 나머지는 널
        // 여기도 대상을 자르지 않는다 — 전체 해제와 같은 StagedFile, settle_pending 경로다
        if let Some((target, path)) = &self.file_target {
            if ask_mode == ASK_EXTRACT && index == *target && !is_dir {
                match crate::outfile::StagedFile::create(path) {
                    Ok((f, staged)) => {
                        *self.shared.pending.lock().unwrap() = Some(PendingStaged {
                            staged,
                            path: path.to_string_lossy().into_owned(),
                            ok: false,
                            expected: self.sizes.get(index as usize).copied().flatten(),
                        });
                        let s = streams::output_file_from(f);
                        if !out_stream.is_null() {
                            *out_stream = s.into_raw();
                        }
                    }
                    // 출력 파일 생성 실패 → E_FAIL 로 중단
                    Err(_) => return HRESULT(0x8000_4005u32 as i32),
                }
            }
            return S_OK;
        }

        // writer 스트리밍 추출 모드 = 대상 인덱스에 writer 1회 전달
        {
            let mut wt = self.writer_target.lock().unwrap();
            if let Some((target, _)) = wt.as_ref() {
                let target = *target;
                if ask_mode == ASK_EXTRACT && index == target && !is_dir {
                    if let Some((_, w)) = wt.take() {
                        let s = streams::output_writer(w);
                        if !out_stream.is_null() {
                            *out_stream = s.into_raw();
                        }
                    }
                }
                return S_OK;
            }
        }

        // 테스트/건너뛰기(kSkip): 출력 스트림 없이 진행
        if self.test_mode || ask_mode == ASK_SKIP {
            return S_OK;
        }

        // 대상 경로 계산, 아카이브에 저장된 이름은 공격자가 정하는 값이므로 그대로 join 하지 않는다
        // — 7z.dll 은 경로를 검증하지 않는다(자세한 이유는 crate::paths), 안전하지 않은 항목은 널
        // 스트림으로 건너뛴다, 여기서 실패를 올리면 악성 항목 하나가 해제 전체를 죽이므로, 기록만
        // 남기고 나머지는 계속 푼다
        let name = if self.keep_paths {
            path.to_string()
        } else {
            basename(path)
        };
        let target = match crate::paths::sanitize(&name)
            .and_then(|rel| crate::paths::resolve_under(&self.dest, &rel))
        {
            Ok(t) => t,
            Err(_) => {
                self.shared
                    .unsafe_paths
                    .lock()
                    .unwrap()
                    .push(path.replace('\\', "/"));
                return S_OK;
            }
        };

        if is_dir {
            if let Err(e) = std::fs::create_dir_all(&target) {
                self.record_failure(path, &e);
            }
            return S_OK;
        }

        // 충돌 처리 = 파일별 개별 정책 우선, 없으면 기본 정책
        let target = if target.exists() {
            let key = path.replace('\\', "/");
            match self.decisions.get(&key).copied().unwrap_or(self.overwrite) {
                // 건너뛰기: 널 스트림을 돌려 이 항목을 쓰지 않는다
                OverwriteMode::Skip => return S_OK,
                // 이름 변경 = 겹치지 않는 새 이름으로 저장 + 기존 파일 보존
                OverwriteMode::Rename => crate::formats::unique_path(&target),
                OverwriteMode::Overwrite => target,
            }
        } else {
            target
        };
        if let Some(parent) = target.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                self.record_failure(path, &e);
                return S_OK;
            }
        }
        // 기존 파일을 여기서 자르지 않는다, 대상이 이미 있으면 옆의 임시 파일에 쓰고, 그 항목이
        // 성공 시에만 제자리로 이동(settle_pending), 즉시 열어 자르면 손상
        // 아카이브의 CRC 오류 하나로 멀쩡하던 기존 파일이 사라진다
        match crate::outfile::StagedFile::create(&target) {
            Ok((f, staged)) => {
                *self.shared.pending.lock().unwrap() = Some(PendingStaged {
                    staged,
                    path: path.replace('\\', "/"),
                    ok: false,
                    expected: self.sizes.get(index as usize).copied().flatten(),
                });
                let s = streams::output_file_from(f);
                if !out_stream.is_null() {
                    *out_stream = s.into_raw();
                }
                S_OK
            }
            // 생성 실패 = 널 스트림으로 건너뛰기 + 기록 필수, 조용히 넘기면 파일 누락
            // "해제 성공" 이 되고, [해제 후 원본 삭제] 와 만나면 복구가 불가능하다
            Err(e) => {
                self.record_failure(path, &e);
                S_OK
            }
        }
    }

    unsafe fn PrepareOperation(&self, _ask_mode: i32) -> HRESULT {
        S_OK
    }

    unsafe fn SetOperationResult(&self, op_result: i32) -> HRESULT {
        if op_result != 0 {
            *self.shared.op_result.lock().unwrap() = op_result;
        }
        // 직전 항목의 성공 여부를 임시 파일 쪽에 통지, 이동은 스트림 해제 뒤
        if let Some(p) = self.shared.pending.lock().unwrap().as_mut() {
            p.ok = op_result == 0;
        }
        // 테스트(CRC 보고) 모드: 방금 처리한 항목의 결과를 확정해 수집
        if let Some(rep) = &self.crc_report {
            if let Some(cur) = rep.current.lock().unwrap().take() {
                let actual = cur.crc.map(|c| c.lock().unwrap().finalize());
                rep.out.lock().unwrap().push(TestEntry {
                    path: cur.path,
                    is_dir: cur.is_dir,
                    expected_crc: cur.expected_crc,
                    actual_crc: actual,
                    ok: op_result == 0,
                });
            }
        }

        // 바이러스 검사 모드: 방금 메모리로 푼 파일을 검사 콜백에 넘겨 상태를 확정
        if let Some(rep) = &self.scan_report {
            if let Some(cur) = rep.current.lock().unwrap().take() {
                // 신고 크기를 모르는 항목은 실제로 나온 바이트를 크기로 적는다(0 이 아니다)
                let actual = cur.buf.lock().unwrap().len() as u64;
                let status = if op_result != 0 {
                    // 해제 실패(손상/암호/한도 초과) → 검사 불가
                    "error".to_string()
                } else {
                    let data = cur.buf.lock().unwrap();
                    let mut scan = rep.scan.lock().unwrap();
                    (scan)(&cur.path, &data)
                };
                rep.seen.lock().unwrap().insert(cur.index);
                rep.out.lock().unwrap().push(ScanEntry {
                    path: cur.path,
                    is_dir: false,
                    size: cur.size.unwrap_or(actual),
                    status,
                });
            }
        }
        S_OK
    }
}

impl ICryptoGetTextPassword_Impl for ExtractCb_Impl {
    unsafe fn CryptoGetTextPassword(&self, password: *mut *const u16) -> HRESULT {
        self.shared.crypto_requested.store(true, Ordering::SeqCst);
        write_password(&self.password, password)
    }
}

/// 해제/테스트 콜백 생성
pub fn make_extract_cb(cfg: ExtractCfg<'_>) -> (IArchiveExtractCallback, ExtractShared) {
    let shared = ExtractShared {
        op_result: Arc::new(Mutex::new(0)),
        crypto_requested: Arc::new(AtomicBool::new(false)),
        aborted: Arc::new(AtomicBool::new(false)),
        unsafe_paths: Arc::new(Mutex::new(Vec::new())),
        failed_paths: Arc::new(Mutex::new(Vec::new())),
        pending: Arc::new(Mutex::new(None)),
    };
    let cb: IArchiveExtractCallback = ExtractCb {
        entries: cfg.entries,
        sizes: cfg.sizes,
        dest: cfg.dest,
        keep_paths: cfg.keep_paths,
        overwrite: cfg.overwrite,
        decisions: cfg.decisions,
        test_mode: cfg.test_mode,
        password: cfg.password.map(to_wide_nul),
        total: Mutex::new(0),
        current_file: Mutex::new(None),
        progress: cfg.progress,
        shared: shared.clone(),
        cancel: cfg.cancel,
        mem_target: cfg.mem_target,
        file_target: cfg.file_target,
        writer_target: Mutex::new(cfg.writer_target),
        crc_report: cfg.crc_report.map(|c| CrcReportState {
            expected: c.expected,
            out: c.out,
            current: Mutex::new(None),
        }),
        scan_report: cfg.scan_report.map(|c| ScanReportState {
            max_size: c.max_size,
            sizes: c.sizes,
            scan: Mutex::new(c.scan),
            out: c.out,
            seen: c.seen,
            current: Mutex::new(None),
        }),
    }
    .into();
    (cb, shared)
}

// ─────────────────────────── 생성 콜백 ───────────────────────────

/// 생성/편집할 항목 하나
#[derive(Clone)]
pub struct UpdateItem {
    pub name: String,
    pub source: Option<PathBuf>,
    pub size: u64,
    pub is_dir: bool,
    pub mtime: u64,
    pub keep_index: Option<u32>,
}

/// 작업 후 읽는 공유 상태
#[derive(Clone)]
pub struct UpdateShared {
    pub op_result: Arc<Mutex<i32>>,
    pub aborted: Arc<AtomicBool>,
}

/// 생성 콜백 구성
pub struct UpdateCfg<'a> {
    pub items: Arc<Vec<UpdateItem>>,
    pub password: Option<&'a str>,
    pub progress: Option<ProgressSink>,
    pub cancel: Arc<AtomicBool>,
}

#[windows_core::implement(IArchiveUpdateCallback, ICryptoGetTextPassword2)]
struct UpdateCb {
    items: Arc<Vec<UpdateItem>>,
    password: Option<Vec<u16>>,
    total: Mutex<u64>,
    current_file: Mutex<Option<String>>,
    progress: Option<ProgressSink>,
    shared: UpdateShared,
    cancel: Arc<AtomicBool>,
}

impl IArchiveUpdateCallback_Impl for UpdateCb_Impl {
    unsafe fn SetTotal(&self, total: u64) -> HRESULT {
        *self.total.lock().unwrap() = total;
        S_OK
    }

    unsafe fn SetCompleted(&self, complete: *const u64) -> HRESULT {
        if self.cancel.load(Ordering::SeqCst) {
            self.shared.aborted.store(true, Ordering::SeqCst);
            return E_ABORT;
        }
        if let Some(sink) = &self.progress {
            let total = *self.total.lock().unwrap();
            let c = if complete.is_null() { 0 } else { *complete };
            let percent = if total > 0 {
                ((c.saturating_mul(100)) / total).min(100) as u8
            } else {
                0
            };
            let file = self.current_file.lock().unwrap().clone();
            sink.call(percent, file);
        }
        S_OK
    }

    unsafe fn GetUpdateItemInfo(
        &self,
        index: u32,
        new_data: *mut i32,
        new_props: *mut i32,
        index_in_archive: *mut u32,
    ) -> HRESULT {
        // keep_index 가 있으면 기존 아카이브 항목을 그대로 복사(재압축 없음), 없으면 신규
        let keep = self.items.get(index as usize).and_then(|i| i.keep_index);
        match keep {
            Some(orig) => {
                if !new_data.is_null() {
                    *new_data = 0;
                }
                if !new_props.is_null() {
                    *new_props = 0;
                }
                if !index_in_archive.is_null() {
                    *index_in_archive = orig;
                }
            }
            None => {
                if !new_data.is_null() {
                    *new_data = 1;
                }
                if !new_props.is_null() {
                    *new_props = 1;
                }
                if !index_in_archive.is_null() {
                    *index_in_archive = u32::MAX;
                }
            }
        }
        S_OK
    }

    unsafe fn GetProperty(&self, index: u32, prop_id: u32, value: *mut PropVariant) -> HRESULT {
        let it = match self.items.get(index as usize) {
            Some(i) => i,
            None => {
                (*value).set_empty();
                return S_OK;
            }
        };
        match prop_id {
            KPID_PATH => (*value).set_bstr(&it.name),
            KPID_IS_DIR => (*value).set_bool(it.is_dir),
            KPID_SIZE => (*value).set_u64(it.size),
            KPID_ATTRIB => (*value).set_u32(if it.is_dir { 0x10 } else { 0x20 }),
            KPID_MTIME => (*value).set_filetime(it.mtime),
            _ => (*value).set_empty(),
        }
        S_OK
    }

    unsafe fn GetStream(&self, index: u32, in_stream: *mut *mut c_void) -> HRESULT {
        if !in_stream.is_null() {
            *in_stream = std::ptr::null_mut();
        }
        if self.cancel.load(Ordering::SeqCst) {
            self.shared.aborted.store(true, Ordering::SeqCst);
            return E_ABORT;
        }
        let it = match self.items.get(index as usize) {
            Some(i) => i,
            None => return S_OK,
        };
        if it.is_dir {
            return S_OK;
        }
        *self.current_file.lock().unwrap() = Some(it.name.replace('\\', "/"));
        if let Some(path) = &it.source {
            match streams::source_file(path) {
                Ok(s) => {
                    if !in_stream.is_null() {
                        *in_stream = s.into_raw();
                    }
                }
                Err(_) => {
                    *self.shared.op_result.lock().unwrap() = 2;
                    return HRESULT(0x8007_0002u32 as i32); // 파일 없음 → 실패
                }
            }
        }
        S_OK
    }

    unsafe fn SetOperationResult(&self, op_result: i32) -> HRESULT {
        if op_result != 0 {
            *self.shared.op_result.lock().unwrap() = op_result;
        }
        S_OK
    }
}

impl ICryptoGetTextPassword2_Impl for UpdateCb_Impl {
    unsafe fn CryptoGetTextPassword2(
        &self,
        password_is_defined: *mut i32,
        password: *mut *const u16,
    ) -> HRESULT {
        match &self.password {
            Some(wide) => {
                if !password_is_defined.is_null() {
                    *password_is_defined = 1;
                }
                let s = String::from_utf16_lossy(&wide[..wide.len().saturating_sub(1)]);
                let b = BSTR::from(s);
                if !password.is_null() {
                    *password = b.into_raw();
                }
            }
            None => {
                if !password_is_defined.is_null() {
                    *password_is_defined = 0;
                }
                if !password.is_null() {
                    *password = std::ptr::null();
                }
            }
        }
        S_OK
    }
}

/// 생성 콜백 생성
pub fn make_update_cb(cfg: UpdateCfg<'_>) -> (IArchiveUpdateCallback, UpdateShared) {
    let shared = UpdateShared {
        op_result: Arc::new(Mutex::new(0)),
        aborted: Arc::new(AtomicBool::new(false)),
    };
    let cb: IArchiveUpdateCallback = UpdateCb {
        items: cfg.items,
        password: cfg.password.map(to_wide_nul),
        total: Mutex::new(0),
        current_file: Mutex::new(None),
        progress: cfg.progress,
        shared: shared.clone(),
        cancel: cfg.cancel,
    }
    .into();
    (cb, shared)
}

/// 이 항목을 검사 대상으로 읽을 것인가 — 크기를 모르면(None) 읽어 보되 streams::mem_out_capped
/// 가 같은 한도로 절단, 초과 시 error 로 기록(D3.5)
pub(crate) fn scan_admits(size: Option<u64>, max: u64) -> bool {
    size.is_none_or(|s| s < max)
}

#[cfg(test)]
mod scan_rule_tests {
    use super::*;

    /// 낮춰 잡은 한도와 "크기 미상"이 어떻게 갈리나
    #[test]
    fn 크기를_모르면_읽어_보되_한도는_그대로다() {
        // 한도보다 작으면 읽는다
        assert!(scan_admits(Some(0), 10));
        assert!(scan_admits(Some(9), 10));
        // 한도 이상 미판독(skipped 기록)
        assert!(!scan_admits(Some(10), 10));
        assert!(!scan_admits(Some(u64::MAX), 10));
        // 모르는 것은 건너뛰기 금지 — 건너뛰면 미검사 상태로 위협 없음 판정
        assert!(scan_admits(None, 10));
        // 한도가 0 이면 아는 항목은 전부 건너뛴다
        assert!(!scan_admits(Some(0), 0));
        // 모르는 항목도 판독 시도 — 스트림 상한(0)이 첫 바이트에서 절단하므로 결과는 error
        // 건너뛰기(skipped)와 달리 검사하지 못함으로 상신
        assert!(scan_admits(None, 0));
    }
}


#[cfg(test)]
mod settle_tests {
    use super::*;

    fn shared() -> ExtractShared {
        ExtractShared {
            op_result: Arc::new(Mutex::new(0)),
            crypto_requested: Arc::new(AtomicBool::new(false)),
            aborted: Arc::new(AtomicBool::new(false)),
            unsafe_paths: Arc::new(Mutex::new(Vec::new())),
            failed_paths: Arc::new(Mutex::new(Vec::new())),
            pending: Arc::new(Mutex::new(None)),
        }
    }

    /// 임시 파일에 bytes 를 쓴 대기 항목 생성(expected = 목록 신고 크기)
    fn pend(sh: &ExtractShared, target: &std::path::Path, bytes: &[u8], expected: Option<u64>) {
        let (mut f, staged) = crate::outfile::StagedFile::create(target).unwrap();
        std::io::Write::write_all(&mut f, bytes).unwrap();
        drop(f);
        *sh.pending.lock().unwrap() = Some(PendingStaged {
            staged,
            path: "a.txt".into(),
            ok: true,
            expected,
        });
    }

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("zm_settle_{}_{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// 신고한 것보다 적게 나온 항목은 옮기지 않는다(더 나오는 것은 막지 않는다 — D3.14)
    #[test]
    fn 신고보다_적게_나오면_옮기지_않는다() {
        let d = tmpdir("short");
        let target = d.join("a.txt");
        std::fs::write(&target, "기존 내용").unwrap();

        let sh = shared();
        pend(&sh, &target, b"0123456789", Some(999));
        sh.settle_pending();

        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "기존 내용",
            "짧게 나온 항목이 기존 파일을 덮었다"
        );
        let failed = sh.failed_paths.lock().unwrap();
        assert_eq!(failed.len(), 1, "빠진 항목으로 기록되지 않았다");
        assert!(failed[0].1.contains("999"), "무엇이 어긋났는지 알려 주지 않는다: {}", failed[0].1);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// 더 나온 것은 미차단, 정상 아카이브인데 목록보다 많이 나오는 포맷 존재 — 실측으로
    /// bzip2 는 목록 크기를 0 으로 주고(핸들러가 원본 크기를 모른다), 다중 멤버 gzip 은 마지막
    /// 멤버 크기만 신고(777 로 적고 1554 산출), != 로 막으면 멀쩡한 .bz2 해제가 전부
    /// 경고
    #[test]
    fn 더_나온_것과_모르는_크기는_막지_않는다() {
        let d = tmpdir("more");

        // bzip2 처럼 목록이 0 을 준 경우
        let t1 = d.join("b.txt");
        let sh = shared();
        pend(&sh, &t1, b"0123456789", Some(0));
        sh.settle_pending();
        assert_eq!(std::fs::read(&t1).unwrap(), b"0123456789", "0 신고를 막았다");

        // 다중 멤버 gzip 처럼 실제가 더 큰 경우
        let t2 = d.join("c.txt");
        let sh = shared();
        pend(&sh, &t2, b"01234567890123456789", Some(10));
        sh.settle_pending();
        assert_eq!(std::fs::read(&t2).unwrap().len(), 20, "더 나온 것을 막았다");

        // 크기 미상 항목(조회 실패)도 그대로 이동
        let t3 = d.join("d.txt");
        let sh = shared();
        pend(&sh, &t3, b"abc", None);
        sh.settle_pending();
        assert_eq!(std::fs::read(&t3).unwrap(), b"abc", "모르는 크기를 막았다");

        assert!(sh.failed_paths.lock().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }
}
