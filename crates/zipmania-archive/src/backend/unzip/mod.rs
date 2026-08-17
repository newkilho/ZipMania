//! ZIP 백엔드, 순수 Rust, 플랫폼 무관, zip 계열 전담, 나머지 40여 포맷은 7z.dll
//! 근거, 측정값 (D3.13)
//!
//! 포맷 1개 = 주인 1개, 분할 금지
//! 미담당분(분할 볼륨 .z01, 손상 복구, 미지원 압축 방식) → Unzip 자기 필드의 폴백으로 위임
//! 판정은 open 한 곳에서만, super::Router 에 폴백 추가 금지
//!
//! 이름 읽기 = entry_name 전용, 크레이트 name() 은 CP949 를 CP437 로 오독

pub mod create;
pub mod extract;
pub mod parallel;

#[cfg(test)]
mod tests;

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use zip::read::ZipFile;
use zip::CompressionMethod;

use crate::crc32::Crc32;
use crate::error::ZipManiaError;
use crate::formats::ScanFn;
use crate::models::{ArchiveEntry, ScanEntry, TestEntry};

use super::{
    ArchiveBackend, CreateOptions, CreateResult, EditOptions, ExtractOptions, ExtractResult,
    ProgressFn,
};

/// 열린 zip 아카이브(디스크 파일 기반)
pub type Archive = zip::ZipArchive<BufReader<File>>;

/// ZIP 직접 처리 백엔드
pub struct Unzip {
    fallback: Option<Box<dyn ArchiveBackend>>,
}

impl Default for Unzip {
    fn default() -> Self {
        Unzip::new()
    }
}

impl Unzip {
    /// 폴백 없이 생성, 분할, 손상 아카이브 = unsupported 거부
    pub fn new() -> Self {
        Unzip { fallback: None }
    }

    /// 폴백 백엔드 주입 생성(Windows = 7z.dll 백엔드)
    pub fn with_fallback(fallback: Box<dyn ArchiveBackend>) -> Self {
        Unzip {
            fallback: Some(fallback),
        }
    }

    /// 폴백 있으면 위임, 없으면 이유를 담은 unsupported 오류
    fn fb(&self, why: &str) -> Result<&dyn ArchiveBackend, ZipManiaError> {
        self.fallback.as_deref().ok_or_else(|| {
            ZipManiaError::new(
                "unsupported",
                format!("이 ZIP 은 처리할 수 없습니다: {why}"),
            )
        })
    }
}

/// 열기 결과, 직접 담당 또는 이유와 함께 위임
pub enum Opened {
    Mine(Box<Archive>),
    Delegate(String),
}

/// 열어서 전담 가능한지 판정, 판정 지점은 여기 한 곳뿐(작업마다 판정하면 목록, 해제 결과가 갈림)
pub fn open(archive: &str) -> Result<Opened, ZipManiaError> {
    let file = File::open(archive)
        .map_err(|e| ZipManiaError::new("io_error", format!("아카이브를 열지 못했습니다: {e}")))?;
    let mut ar = match zip::ZipArchive::new(BufReader::new(file)) {
        Ok(a) => a,
        // 중앙 디렉터리 손상 또는 분할 볼륨 → 7z 가 로컬 헤더 스캔으로 부분 복구
        Err(e) => return Ok(Opened::Delegate(format!("{e}"))),
    };

    // 디코드 불가 방식 1개라도 있으면 전체 위임(절반만 풀지 않음)
    // 미컴파일 방식(zstd) = 크레이트가 Unsupported(id) 반환
    for i in 0..ar.len() {
        let Ok(f) = ar.by_index_raw(i) else {
            return Ok(Opened::Delegate("항목 헤더를 읽지 못했습니다".into()));
        };
        #[allow(deprecated)]
        if let CompressionMethod::Unsupported(id) = f.compression() {
            return Ok(Opened::Delegate(format!("지원하지 않는 압축 방식({id})")));
        }
    }
    Ok(Opened::Mine(Box::new(ar)))
}

/// 항목 이름, UTF-8 플래그 없으면 CP949 디코드
/// 플래그 유무 판정 = name() == name_raw()(크레이트가 플래그 미노출)
pub fn entry_name<R: Read>(f: &ZipFile<'_, R>) -> String {
    let raw = f.name_raw();
    let s = if f.name().as_bytes() == raw {
        f.name().replace('\\', "/")
    } else {
        let (text, _, _) = encoding_rs::EUC_KR.decode(raw);
        text.replace('\\', "/")
    };
    // zip = src/, 7z.dll = src, 7z 표기로 통일
    // 선택 해제, 충돌 검사, 중첩 뷰어가 UI 경로와 문자열 비교 → 갈라지면 폴더 선택이 안 풀림
    s.trim_end_matches('/').to_string()
}

/// zip MS-DOS 시각 → 목록 표시용 문자열, 없으면 빈 문자열
fn entry_modified<R: Read>(f: &ZipFile<'_, R>) -> String {
    match f.last_modified() {
        Some(d) => format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            d.year(),
            d.month(),
            d.day(),
            d.hour(),
            d.minute(),
            d.second()
        ),
        None => String::new(),
    }
}

/// 목록 항목 1개 정규화
fn to_entry<R: Read>(f: &ZipFile<'_, R>) -> ArchiveEntry {
    let is_dir = f.is_dir();
    ArchiveEntry {
        path: entry_name(f),
        size: f.size(),
        packed_size: f.compressed_size(),
        modified: entry_modified(f),
        is_dir,
        // 폴더 CRC 0 = 기록만 0, 실제로는 없음
        crc: if is_dir {
            None
        } else {
            Some(format!("{:08X}", f.crc32()))
        },
    }
}

/// 암호 오류 → 우리 코드 체계, 암호 문제 = 항목 단위 아닌 전역 실패(UI 재질의 조건)
pub fn map_err(e: zip::result::ZipError, had_password: bool) -> ZipManiaError {
    use zip::result::ZipError;
    match e {
        ZipError::InvalidPassword => {
            if had_password {
                ZipManiaError::new("wrong_password", "비밀번호가 올바르지 않습니다.")
            } else {
                ZipManiaError::new("password_required", "비밀번호가 필요합니다.")
            }
        }
        ZipError::UnsupportedArchive(msg) if msg == ZipError::PASSWORD_REQUIRED => {
            ZipManiaError::new("password_required", "비밀번호가 필요합니다.")
        }
        ZipError::FileNotFound => {
            ZipManiaError::new("not_found", "아카이브 안에서 해당 항목을 찾지 못했습니다.")
        }
        ZipError::Io(e) => ZipManiaError::new("io_error", format!("입출력 오류: {e}")),
        ZipError::InvalidArchive(msg) => {
            ZipManiaError::new("corrupt", format!("아카이브가 손상되었습니다: {msg}"))
        }
        ZipError::UnsupportedArchive(msg) => {
            ZipManiaError::new("unsupported", format!("지원하지 않는 아카이브입니다: {msg}"))
        }
        ZipError::CompressionMethodNotSupported(id) => ZipManiaError::new(
            "unsupported",
            format!("지원하지 않는 압축 방식입니다({id})."),
        ),
        other => ZipManiaError::new("corrupt", format!("아카이브를 읽지 못했습니다: {other}")),
    }
}

/// 항목 1개 열기, 암호 있으면 복호화 동반
fn open_entry<'a>(
    ar: &'a mut Archive,
    index: usize,
    password: Option<&str>,
) -> Result<ZipFile<'a, BufReader<File>>, ZipManiaError> {
    let r = match password {
        Some(pw) => ar.by_index_decrypt(index, pw.as_bytes()),
        None => ar.by_index(index),
    };
    r.map_err(|e| map_err(e, password.is_some()))
}

/// 항목 1개 → 메모리, limit != 0 이면 실제 출력 바이트에 상한(신고 크기는 공격자가 정함)
fn read_all<R: Read>(f: &mut R, limit: u64) -> Result<Vec<u8>, ZipManiaError> {
    let mut out = Vec::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f
            .read(&mut buf)
            .map_err(|e| ZipManiaError::new("corrupt", format!("압축을 풀지 못했습니다: {e}")))?;
        if n == 0 {
            break;
        }
        if limit != 0 && out.len() as u64 + n as u64 > limit {
            return Err(ZipManiaError::new(
                "too_large",
                "항목이 메모리 처리 상한을 넘었습니다.",
            ));
        }
        out.extend_from_slice(&buf[..n]);
    }
    Ok(out)
}

/// 내부 경로 → 인덱스, 구분자, 대소문자 그대로 비교
fn index_of(ar: &mut Archive, inner_path: &str) -> Result<usize, ZipManiaError> {
    let want = inner_path.replace('\\', "/");
    for i in 0..ar.len() {
        let Ok(f) = ar.by_index_raw(i) else { continue };
        if !f.is_dir() && entry_name(&f) == want {
            return Ok(i);
        }
    }
    Err(ZipManiaError::new(
        "not_found",
        "아카이브 안에서 해당 항목을 찾지 못했습니다.",
    ))
}

fn canceled(cancel: &Arc<AtomicBool>) -> bool {
    cancel.load(Ordering::Relaxed)
}

fn percent(done: u64, total: u64) -> u8 {
    if total == 0 {
        return 0;
    }
    ((done.saturating_mul(100) / total).min(100)) as u8
}

impl ArchiveBackend for Unzip {
    fn id(&self) -> &'static str {
        "unzip"
    }

    fn read_exts(&self) -> &'static [&'static str] {
        crate::formats::UNZIP_EXTS
    }

    fn write_exts(&self) -> &'static [&'static str] {
        crate::formats::UNZIP_EXTS
    }

    /// 확장자 미상 파일 미담당, 내용 탐지는 7z.dll 이 우수
    fn accepts_unknown(&self) -> bool {
        false
    }

    fn list(&self, archive: &str, password: Option<&str>) -> Result<Vec<ArchiveEntry>, ZipManiaError> {
        let mut ar = match open(archive)? {
            Opened::Mine(a) => a,
            Opened::Delegate(why) => return self.fb(&why)?.list(archive, password),
        };
        let mut out = Vec::with_capacity(ar.len());
        for i in 0..ar.len() {
            // 목록은 해제 없이 읽힘 — 이름, 크기는 헤더에 있음
            let f = ar.by_index_raw(i).map_err(|e| map_err(e, false))?;
            out.push(to_entry(&f));
        }
        Ok(out)
    }

    fn extract(
        &self,
        opts: &ExtractOptions,
        on_progress: &mut ProgressFn<'_>,
        cancel: Arc<AtomicBool>,
    ) -> ExtractResult {
        let ar = match open(&opts.archive) {
            Ok(Opened::Mine(a)) => a,
            Ok(Opened::Delegate(why)) => {
                return match self.fb(&why) {
                    Ok(b) => b.extract(opts, on_progress, cancel),
                    Err(e) => ExtractResult::Failed(e),
                }
            }
            Err(e) => return ExtractResult::Failed(e),
        };
        extract::extract_all(*ar, opts, on_progress, cancel)
    }

    fn create(
        &self,
        opts: &CreateOptions,
        on_progress: &mut ProgressFn<'_>,
        cancel: Arc<AtomicBool>,
    ) -> CreateResult {
        create::do_create(opts, on_progress, cancel)
    }

    fn edit(
        &self,
        opts: &EditOptions,
        on_progress: &mut ProgressFn<'_>,
        cancel: Arc<AtomicBool>,
    ) -> CreateResult {
        let ar = match open(&opts.archive) {
            Ok(Opened::Mine(a)) => a,
            Ok(Opened::Delegate(why)) => {
                return match self.fb(&why) {
                    Ok(b) => b.edit(opts, on_progress, cancel),
                    Err(e) => CreateResult::Failed(e),
                }
            }
            Err(e) => return CreateResult::Failed(e),
        };
        create::do_edit(*ar, opts, on_progress, cancel)
    }

    fn test(&self, archive: &str, password: Option<&str>) -> Result<(), ZipManiaError> {
        let report = self.test_report(
            archive,
            password,
            &mut |_, _| {},
            Arc::new(AtomicBool::new(false)),
        )?;
        if report.iter().any(|e| !e.ok) {
            return Err(ZipManiaError::new(
                "corrupt",
                "무결성 검사에서 손상된 항목을 찾았습니다.",
            ));
        }
        Ok(())
    }

    /// 파일별 CRC 검증, 크레이트도 끝에서 검증하나 우리도 실제 출력 바이트로 직접 계산
    /// (실패 시 표에 실제값을 채우려면 필요)
    fn test_report(
        &self,
        archive: &str,
        password: Option<&str>,
        on_progress: &mut ProgressFn<'_>,
        cancel: Arc<AtomicBool>,
    ) -> Result<Vec<TestEntry>, ZipManiaError> {
        let mut ar = match open(archive)? {
            Opened::Mine(a) => a,
            Opened::Delegate(why) => {
                return self.fb(&why)?.test_report(archive, password, on_progress, cancel)
            }
        };

        let mut total = 0u64;
        for i in 0..ar.len() {
            if let Ok(f) = ar.by_index_raw(i) {
                if !f.is_dir() {
                    total += f.size();
                }
            }
        }
        let mut done = 0u64;
        let mut out = Vec::with_capacity(ar.len());

        for i in 0..ar.len() {
            if canceled(&cancel) {
                return Err(ZipManiaError::new("canceled", "검사를 취소했습니다."));
            }
            let (name, is_dir, expected, encrypted) = {
                let f = ar.by_index_raw(i).map_err(|e| map_err(e, false))?;
                (entry_name(&f), f.is_dir(), f.crc32(), f.encrypted())
            };
            // AES(AE-2) CRC 0 = 없음, 값 0 아님, 그대로 비교하면 정상 AES-256 zip 이 전부 손상 판정(실측)
            // 암호화 항목에만 적용 — 평문의 0 은 진짜 CRC 일 수 있음
            let has_crc = !(encrypted && expected == 0);
            if is_dir {
                out.push(TestEntry {
                    path: name,
                    is_dir: true,
                    expected_crc: None,
                    actual_crc: None,
                    ok: true,
                });
                continue;
            }

            on_progress(percent(done, total), Some(name.clone()));

            let mut crc = Crc32::new();
            let mut size = 0u64;
            let mut ok = true;
            {
                let mut f = match open_entry(&mut ar, i, password) {
                    Ok(f) => f,
                    // 암호 = 전역 실패, 항목 손상으로 보고하면 UI 재질의 불가
                    Err(e) if e.code == "password_required" || e.code == "wrong_password" => {
                        return Err(e)
                    }
                    Err(_) => {
                        out.push(TestEntry {
                            path: name,
                            is_dir: false,
                            expected_crc: has_crc.then_some(expected),
                            actual_crc: None,
                            ok: false,
                        });
                        continue;
                    }
                };
                let mut buf = [0u8; 64 * 1024];
                loop {
                    match f.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            crc.update(&buf[..n]);
                            size += n as u64;
                        }
                        // 끝에서 CRC 불일치 시 크레이트가 여기서 오류
                        Err(_) => {
                            ok = false;
                            break;
                        }
                    }
                }
            }
            let actual = crc.finalize();
            // 기록 없으면 읽어 낸 것 자체가 검증, AES 항목은 복호 + HMAC 통과해야 도달
            if has_crc && actual != expected {
                ok = false;
            }
            done += size;
            out.push(TestEntry {
                path: name,
                is_dir: false,
                expected_crc: has_crc.then_some(expected),
                actual_crc: Some(actual),
                ok,
            });
        }
        on_progress(100, None);
        Ok(out)
    }

    fn scan_report(
        &self,
        archive: &str,
        password: Option<&str>,
        max_size: u64,
        mut scan: ScanFn,
        on_progress: &mut ProgressFn<'_>,
        cancel: Arc<AtomicBool>,
    ) -> Result<Vec<ScanEntry>, ZipManiaError> {
        let mut ar = match open(archive)? {
            Opened::Mine(a) => a,
            Opened::Delegate(why) => {
                return self
                    .fb(&why)?
                    .scan_report(archive, password, max_size, scan, on_progress, cancel)
            }
        };

        let count = ar.len();
        let mut out = Vec::with_capacity(count);
        for i in 0..count {
            if canceled(&cancel) {
                return Err(ZipManiaError::new("canceled", "검사를 취소했습니다."));
            }
            let (name, is_dir, size) = {
                let f = ar.by_index_raw(i).map_err(|e| map_err(e, false))?;
                (entry_name(&f), f.is_dir(), f.size())
            };
            if is_dir {
                continue;
            }
            on_progress(percent(i as u64, count as u64), Some(name.clone()));

            // 상한 초과 항목 = 미검사로 보고, 안전에 포함 금지
            if max_size != 0 && size >= max_size {
                out.push(ScanEntry {
                    path: name,
                    is_dir: false,
                    size,
                    status: "skipped".into(),
                });
                continue;
            }

            let bytes = {
                let mut f = match open_entry(&mut ar, i, password) {
                    Ok(f) => f,
                    Err(e) if e.code == "password_required" || e.code == "wrong_password" => {
                        return Err(e)
                    }
                    Err(_) => {
                        out.push(ScanEntry {
                            path: name,
                            is_dir: false,
                            size,
                            status: "error".into(),
                        });
                        continue;
                    }
                };
                match read_all(&mut f, max_size.max(1)) {
                    Ok(b) => b,
                    Err(_) => {
                        out.push(ScanEntry {
                            path: name,
                            is_dir: false,
                            size,
                            status: "error".into(),
                        });
                        continue;
                    }
                }
            };

            let status = scan(&name, &bytes);
            out.push(ScanEntry {
                path: name,
                is_dir: false,
                size,
                status,
            });
        }
        on_progress(100, None);
        Ok(out)
    }

    fn read_entry_to_memory(
        &self,
        archive: &str,
        inner_path: &str,
        password: Option<&str>,
    ) -> Result<Vec<u8>, ZipManiaError> {
        let mut ar = match open(archive)? {
            Opened::Mine(a) => a,
            Opened::Delegate(why) => {
                return self.fb(&why)?.read_entry_to_memory(archive, inner_path, password)
            }
        };
        let idx = index_of(&mut ar, inner_path)?;
        let mut f = open_entry(&mut ar, idx, password)?;
        read_all(&mut f, crate::formats::MAX_MEMORY_ENTRY_BYTES)
    }

    fn extract_entry_to_file(
        &self,
        archive: &str,
        inner_path: &str,
        dest_file: &Path,
        password: Option<&str>,
    ) -> Result<(), ZipManiaError> {
        let mut ar = match open(archive)? {
            Opened::Mine(a) => a,
            Opened::Delegate(why) => {
                return self
                    .fb(&why)?
                    .extract_entry_to_file(archive, inner_path, dest_file, password)
            }
        };
        let idx = index_of(&mut ar, inner_path)?;
        let mut f = open_entry(&mut ar, idx, password)?;
        // 대상 truncate 금지, File::create 사용 시 기존 파일이 즉시 잘림
        // 전체 해제와 같은 crate::outfile::StagedFile 로 옆에 쓰고 완료 후 이동
        let (out, staged) = crate::outfile::StagedFile::create(dest_file).map_err(|e| {
            ZipManiaError::new("io_error", format!("파일을 만들지 못했습니다: {e}"))
        })?;
        let mut out = std::io::BufWriter::new(out);
        std::io::copy(&mut f, &mut out)
            .map_err(|e| ZipManiaError::new("corrupt", format!("압축을 풀지 못했습니다: {e}")))?;
        // flush 필수, BufWriter 는 최대 8KiB 보유 후 drop 때 기록 → 그 실패가 조용히 버려짐
        std::io::Write::flush(&mut out)
            .map_err(|e| ZipManiaError::new("io_error", format!("파일을 쓰지 못했습니다: {e}")))?;
        // 핸들 닫고 이동, Windows 는 열린 파일 rename 불가
        drop(out);
        staged.commit()
    }

    fn extract_entry_to_writer(
        &self,
        archive: &str,
        inner_path: &str,
        writer: Box<dyn std::io::Write + Send>,
        password: Option<&str>,
    ) -> Result<(), ZipManiaError> {
        let mut ar = match open(archive)? {
            Opened::Mine(a) => a,
            Opened::Delegate(why) => {
                return self
                    .fb(&why)?
                    .extract_entry_to_writer(archive, inner_path, writer, password)
            }
        };
        let idx = index_of(&mut ar, inner_path)?;
        let mut f = open_entry(&mut ar, idx, password)?;
        let mut writer = writer;
        std::io::copy(&mut f, &mut writer)
            .map_err(|e| ZipManiaError::new("corrupt", format!("압축을 풀지 못했습니다: {e}")))?;
        // 전달받은 writer 가 버퍼 보유 가능(드래그 지연 렌더링 채널 writer 등)
        // flush 없으면 마지막 조각의 실패가 drop 안에서 소실
        writer
            .flush()
            .map_err(|e| ZipManiaError::new("io_error", format!("출력을 마치지 못했습니다: {e}")))?;
        Ok(())
    }
}
