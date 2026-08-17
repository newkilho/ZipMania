//! 7z.dll 백엔드(in-process COM), dll 경로 = 호출측 주입
//! 하위 모듈: ffi, com, streams, callbacks, prop

#![allow(non_snake_case)]

pub use crate::crc32;

pub mod callbacks;
pub mod com;
pub mod ffi;
pub mod prop;
pub mod streams;

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use windows_core::Interface;

use crate::error::{self, ZipManiaError};
use crate::models::{ArchiveEntry, ScanEntry, TestEntry};

use callbacks::{ExtractCfg, ProgressSink, ScanFn, UpdateCfg, UpdateItem};
use crate::inputs::summarize;
use crate::outfile::reserve_tmp;
use com::*;
use ffi::Dll;

use super::{
    ArchiveBackend, CreateOptions, CreateResult, EditOptions, ExtractOptions, ExtractResult,
    ProgressFn,
};

// 엔진 중립 옵션, 상수 = crate::formats, 기존 경로 호환용 재노출
pub use crate::formats::{CompressFormat, OverwriteMode, READ_EXTS};
use crate::formats::ext_of;

/// 출력 포맷 → 핸들러 CLSID 포맷 id(bit 16-23), CompressFormat 배치 금지
fn clsid_id(format: CompressFormat) -> u8 {
    match format {
        CompressFormat::SevenZip => 0x07,
        CompressFormat::Zip => 0x01,
        CompressFormat::Tar => 0xEE,
    }
}

/// UI 레벨 → 7z 허용 값(0/1/3/5/7/9)
fn normalize_level(level: u8) -> u32 {
    match level {
        0 => 0,
        1..=2 => 1,
        3..=4 => 3,
        5..=6 => 5,
        7..=8 => 7,
        _ => 9,
    }
}

/// 확장자 → 읽기 핸들러 id 목록, 빈 값 = 후보 폴백, 여럿 = 순서대로 시도
/// id = classID 23170F69-40C1-278A-1000-000110XX0000 의 XX, (D3.8)
fn format_ids_for_ext(ext: &str) -> &'static [u8] {
    match ext {
        "7z" | "cb7" => &[0x07],
        // zip 컨테이너 확장자, 정본 밖도 매핑(열기는 되게)
        "zip" | "zipx" | "cbz" | "jar" | "apk" | "docx" | "xlsx" | "pptx" | "epub" | "odt"
        | "ods" | "xpi" | "ipa" | "appx" | "z01" => &[0x01],
        // RAR5 먼저, 실패 시 RAR4
        "rar" | "r00" | "cbr" => &[0xCC, 0x03],
        "arj" => &[0x04],
        "lzh" | "lha" => &[0x06],
        "cab" => &[0x08],
        "tar" | "ova" => &[0xEE],
        "gz" | "gzip" | "tgz" | "tpz" => &[0xEF],
        "bz2" | "bzip2" | "tbz" | "tbz2" => &[0x02],
        "xz" | "txz" => &[0x0C],
        "zst" | "tzst" => &[0x0E],
        "z" | "taz" => &[0x05],
        "lzma" => &[0x0A],
        "lzma86" => &[0x0B],
        "iso" => &[0xE7],
        "udf" => &[0xE0],
        // .img = 여러 핸들러 공유 → 순서대로 시도
        "img" => &[0xE7, 0xE0, 0xDA, 0xD9, 0xC7],
        "wim" | "swm" | "esd" | "ppkg" => &[0xE6],
        "dmg" => &[0xE4],
        "squashfs" => &[0xD2],
        "msi" | "msp" | "msm" => &[0xE5],
        "cpio" => &[0xED],
        "rpm" => &[0xEB],
        "deb" => &[0xEC],
        "xar" | "pkg" | "xip" => &[0xE1],
        "chm" => &[0xE9],
        "nsis" => &[0x09],
        "001" => &[0xEA],
        _ => &[],
    }
}

/// 읽기 핸들러 후보, 확장자 우선 → 7z/zip 폴백
fn candidate_ids(archive: &str) -> Vec<u8> {
    let ext = Path::new(archive)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    let mut ids: Vec<u8> = Vec::new();
    for id in format_ids_for_ext(&ext) {
        if !ids.contains(id) {
            ids.push(*id);
        }
    }
    for f in [0x07u8, 0x01] {
        if !ids.contains(&f) {
            ids.push(f);
        }
    }
    ids
}

fn now_filetime() -> u64 {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    (secs + 11_644_473_600) * 10_000_000
}

// ─────────────────────────── PROPVARIANT 읽기 헬퍼 ───────────────────────────

unsafe fn get_string(arc: &IInArchive, index: u32, prop: u32) -> Option<String> {
    let mut p = prop::PropVariant::empty();
    let _ = arc.GetProperty(index, prop, &mut p);
    let v = p.as_string();
    p.clear();
    v
}
/// 문자열 속성, 조회 실패 = Err, 값 없음 = Ok(None), 형 불일치도 Err
unsafe fn get_string_res(arc: &IInArchive, index: u32, prop: u32) -> Result<Option<String>, ()> {
    let mut p = prop::PropVariant::empty();
    if arc.GetProperty(index, prop, &mut p).0 != 0 {
        p.clear();
        return Err(());
    }
    let v = p.as_string();
    let empty = p.is_empty();
    p.clear();
    match v {
        Some(s) => Ok(Some(s)),
        None if empty => Ok(None),
        None => Err(()),
    }
}
unsafe fn get_u64(arc: &IInArchive, index: u32, prop: u32) -> u64 {
    let mut p = prop::PropVariant::empty();
    let _ = arc.GetProperty(index, prop, &mut p);
    let v = p.as_u64().unwrap_or(0);
    p.clear();
    v
}
/// get_u64 + 읽지 못함(None)과 0 구분
unsafe fn get_u64_opt(arc: &IInArchive, index: u32, prop: u32) -> Option<u64> {
    let mut p = prop::PropVariant::empty();
    if arc.GetProperty(index, prop, &mut p).0 != 0 {
        p.clear();
        return None;
    }
    let v = p.as_u64();
    p.clear();
    v
}
unsafe fn get_bool(arc: &IInArchive, index: u32, prop: u32) -> bool {
    let mut p = prop::PropVariant::empty();
    let _ = arc.GetProperty(index, prop, &mut p);
    let v = p.as_bool().unwrap_or(false);
    p.clear();
    v
}
/// get_bool + 조회 실패, 형 불일치 = Err, 값 없음 = Ok(false)
unsafe fn get_bool_res(arc: &IInArchive, index: u32, prop: u32) -> Result<bool, ()> {
    let mut p = prop::PropVariant::empty();
    if arc.GetProperty(index, prop, &mut p).0 != 0 {
        p.clear();
        return Err(());
    }
    let v = p.as_bool();
    let empty = p.is_empty();
    p.clear();
    match v {
        Some(b) => Ok(b),
        None if empty => Ok(false),
        None => Err(()),
    }
}
unsafe fn get_crc(arc: &IInArchive, index: u32) -> Option<String> {
    let mut p = prop::PropVariant::empty();
    let _ = arc.GetProperty(index, KPID_CRC, &mut p);
    let v = p.as_u32().map(|c| format!("{c:08X}"));
    p.clear();
    v
}
unsafe fn get_crc_u32(arc: &IInArchive, index: u32) -> Option<u32> {
    let mut p = prop::PropVariant::empty();
    let _ = arc.GetProperty(index, KPID_CRC, &mut p);
    let v = p.as_u32();
    p.clear();
    v
}
/// 인덱스별 예상 CRC(kpidCRC)
unsafe fn collect_crcs(arc: &IInArchive, n: usize) -> Vec<Option<u32>> {
    (0..n as u32).map(|i| get_crc_u32(arc, i)).collect()
}
/// 인덱스별 원본 크기(kpidSize), 조회 실패 = None, 0 치환 금지(D3.5)
unsafe fn collect_sizes(arc: &IInArchive, n: usize) -> Vec<Option<u64>> {
    (0..n as u32)
        .map(|i| get_u64_opt(arc, i, KPID_SIZE))
        .collect()
}
unsafe fn get_mtime(arc: &IInArchive, index: u32) -> String {
    let mut p = prop::PropVariant::empty();
    let _ = arc.GetProperty(index, KPID_MTIME, &mut p);
    let v = p.as_filetime().map(prop::filetime_to_string).unwrap_or_default();
    p.clear();
    v
}

/// 항목 수, 조회 실패 = None(0개와 구분)
unsafe fn item_count(arc: &IInArchive) -> Option<u32> {
    let mut n: u32 = 0;
    if arc.GetNumberOfItems(&mut n).0 != 0 {
        return None;
    }
    Some(n)
}

unsafe fn collect_meta(arc: &IInArchive, archive: &str) -> Vec<(String, bool)> {
    match item_count(arc) {
        Some(n) => collect_meta_n(arc, archive, n),
        None => Vec::new(),
    }
}

/// 인덱스 순 (경로, is_dir) 전체, 항목 수는 인자 — 재조회 금지(D3.5)
unsafe fn collect_meta_n(arc: &IInArchive, archive: &str, n: u32) -> Vec<(String, bool)> {
    collect_meta_checked(arc, archive, n).0
}

/// collect_meta_n + 못 읽은 항목 인덱스, 언제나 n 개 채움(밀리면 해제, 검사 어긋남), (D3.5)
unsafe fn collect_meta_checked(
    arc: &IInArchive,
    archive: &str,
    n: u32,
) -> (Vec<(String, bool)>, Vec<u32>) {
    let mut out = Vec::with_capacity(n as usize);
    let mut unreadable = Vec::new();
    for i in 0..n {
        let path_res = get_string_res(arc, i, KPID_PATH);
        let dir_res = get_bool_res(arc, i, KPID_IS_DIR);
        if path_res.is_err() || dir_res.is_err() {
            unreadable.push(i);
        }
        let mut path = path_res.ok().flatten().unwrap_or_default();
        if path.is_empty() {
            path = derived_entry_name(archive);
        }
        out.push((path, dir_res.unwrap_or(false)));
    }
    (out, unreadable)
}

/// 이름 없는 항목(.bz2/.xz/.z) → 아카이브 파일명에서 유도
/// tar 별칭(.tgz/.tbz2/.txz) = .tar 부착, 백업.tgz → 백업.tar, 원본.txt.bz2 → 원본.txt
fn derived_entry_name(archive: &str) -> String {
    let p = Path::new(archive);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
    if stem.is_empty() {
        return "data".to_string();
    }
    // tar 별칭 → .tar 복원
    let tar_alias = matches!(
        ext_of(archive).as_str(),
        "tgz" | "tpz" | "tbz" | "tbz2" | "txz" | "taz" | "tzst"
    );
    if tar_alias {
        return format!("{stem}.tar");
    }
    // 확장자 없음 → 유도 이름 = 파일명 → 원본 덮어씀
    let same_as_archive = p.file_name().and_then(|s| s.to_str()) == Some(stem);
    if same_as_archive {
        format!("{stem}.out")
    } else {
        stem.to_string()
    }
}

// ─────────────────────────── 백엔드 ───────────────────────────

/// 7z.dll COM 백엔드
pub struct SevenZip {
    dll_path: PathBuf,
}

impl SevenZip {
    /// 생성, dll 절대경로 주입
    pub fn new(dll_path: PathBuf) -> Self {
        SevenZip { dll_path }
    }

    /// 보유 dll 경로
    pub fn dll_path(&self) -> &Path {
        &self.dll_path
    }

    /// dll 파일 버전 → 배너 문자열, 예: 7-Zip 26.02 (x64)
    pub fn version(&self) -> Result<String, ZipManiaError> {
        ffi::dll_version_string(&self.dll_path)
            .ok_or_else(|| ZipManiaError::new("parse_error", "7z.dll 버전을 조회하지 못했습니다"))
    }

    fn load(&self) -> Result<Dll, ZipManiaError> {
        Dll::load(&self.dll_path)
    }
}

/// 읽기용 열기, 포맷 후보 순회, 암호 요청된 실패는 즉시 분류
fn open_for_read(dll: &Dll, archive: &str, password: Option<&str>) -> Result<IInArchive, ZipManiaError> {
    let mut last_hr = 0i32;
    for id in candidate_ids(archive) {
        let arc = match dll.create_in_archive(id) {
            Ok(a) => a,
            Err(_) => continue,
        };
        let stream = streams::open_input_file(Path::new(archive))?;
        let (open_cb, crypto) = callbacks::make_open_cb(password);
        let max_check: u64 = 1 << 23;
        let hr = unsafe { arc.Open(stream.as_raw(), &max_check, open_cb.as_raw()) };
        if hr == S_OK {
            return Ok(arc);
        }
        // 암호 요청 = 포맷은 맞음 → 암호 문제로 즉시 분류
        if crypto.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(error::classify_open_failure(
                hr.0,
                true,
                password.is_some(),
            ));
        }
        last_hr = hr.0;
        unsafe {
            let _ = arc.Close();
        }
    }
    Err(error::classify_open_failure(last_hr, false, password.is_some()))
}

/// 목록 조회
fn do_list(
    sz: &SevenZip,
    archive: &str,
    password: Option<&str>,
) -> Result<Vec<ArchiveEntry>, ZipManiaError> {
    let dll = sz.load()?;
    let arc = open_for_read(&dll, archive, password)?;
    let mut n: u32 = 0;
    unsafe {
        if arc.GetNumberOfItems(&mut n).0 != 0 {
            return Err(ZipManiaError::new("corrupt", "아카이브 항목 수를 읽지 못했습니다."));
        }
    }
    let mut entries = Vec::with_capacity(n as usize);
    for i in 0..n {
        unsafe {
            // 이름 없는 항목 → 파일명에서 유도(버리지 않음)
            let mut path = get_string(&arc, i, KPID_PATH).unwrap_or_default();
            if path.is_empty() {
                path = derived_entry_name(archive);
            }
            entries.push(ArchiveEntry {
                path,
                size: get_u64(&arc, i, KPID_SIZE),
                packed_size: get_u64(&arc, i, KPID_PACK_SIZE),
                modified: get_mtime(&arc, i),
                is_dir: get_bool(&arc, i, KPID_IS_DIR),
                crc: get_crc(&arc, i),
            });
        }
    }
    unsafe {
        let _ = arc.Close();
    }
    Ok(entries)
}

/// 선택 경로 → 인덱스(폴더 = 하위 전체), 빈 값 = None(전체)
fn selected_indices(meta: &[(String, bool)], selected: &[String]) -> Option<Vec<u32>> {
    if selected.is_empty() {
        return None;
    }
    let sel: Vec<String> = selected.iter().map(|s| s.replace('\\', "/")).collect();
    let mut idx = Vec::new();
    for (i, (path, _)) in meta.iter().enumerate() {
        let p = path.replace('\\', "/");
        let in_scope = sel
            .iter()
            .any(|s| p == *s || p.starts_with(&format!("{s}/")));
        if in_scope {
            idx.push(i as u32);
        }
    }
    Some(idx)
}

/// 해제
#[allow(clippy::too_many_arguments)]
fn do_extract(
    sz: &SevenZip,
    opts: &ExtractOptions,
    on_progress: &mut ProgressFn<'_>,
    cancel: Arc<AtomicBool>,
) -> ExtractResult {
    let dll = match sz.load() {
        Ok(d) => d,
        Err(e) => return ExtractResult::Failed(e),
    };
    let arc = match open_for_read(&dll, &opts.archive, opts.password.as_deref()) {
        Ok(a) => a,
        Err(e) => return ExtractResult::Failed(e),
    };
    let meta = Arc::new(unsafe { collect_meta(&arc, &opts.archive) });
    // 목록 신고 크기, settle_pending 이 실제 바이트와 대조, 조회 실패 = None = 대조 안 함
    let sizes = Arc::new(unsafe { collect_sizes(&arc, meta.len()) });
    let indices = selected_indices(&meta, &opts.selected);

    let sink = ProgressSink::new(&mut *on_progress);
    let (cb, shared) = callbacks::make_extract_cb(ExtractCfg {
        entries: meta.clone(),
        sizes,
        dest: PathBuf::from(&opts.dest),
        keep_paths: opts.keep_paths,
        overwrite: opts.overwrite,
        decisions: opts.decisions.clone(),
        test_mode: false,
        password: opts.password.as_deref(),
        progress: Some(sink),
        cancel: cancel.clone(),
        mem_target: None,
        file_target: None,
        writer_target: None,
        crc_report: None,
        scan_report: None,
    });

    let hr = unsafe {
        match &indices {
            None => arc.Extract(std::ptr::null(), u32::MAX, 0, cb.as_raw()),
            Some(idx) => arc.Extract(idx.as_ptr(), idx.len() as u32, 0, cb.as_raw()),
        }
    };
    unsafe {
        let _ = arc.Close();
    }
    // 마지막 항목 임시 파일 마감, 취소/실패로 빠지기 전에 부를 것
    shared.settle_pending();

    if shared.aborted.load(std::sync::atomic::Ordering::SeqCst) {
        return ExtractResult::Done {
            status: "canceled",
            message: "작업을 취소했습니다. 이미 해제된 일부 파일이 대상 폴더에 남아 있을 수 있습니다."
                .to_string(),
        };
    }

    let op = *shared.op_result.lock().unwrap();
    let crypto = shared
        .crypto_requested
        .load(std::sync::atomic::Ordering::SeqCst);
    if op != 0 {
        return ExtractResult::Failed(error::classify_operation(
            op,
            crypto,
            opts.password.is_some(),
        ));
    }
    if hr != S_OK {
        // opResult 없는 실패: 암호 요청됐으면 암호 오류, 아니면 손상
        if crypto {
            return ExtractResult::Failed(error::classify_open_failure(
                hr.0,
                true,
                opts.password.is_some(),
            ));
        }
        return ExtractResult::Failed(ZipManiaError::new(
            "corrupt",
            "해제 중 오류가 발생했습니다.",
        ));
    }
    // 빠진 항목 있으면 ok 아님 — 앱이 ok 를 보고 [해제 후 원본 삭제] 수행
    let unsafe_paths = shared.unsafe_paths.lock().unwrap();
    let failed_paths = shared.failed_paths.lock().unwrap();
    if !unsafe_paths.is_empty() || !failed_paths.is_empty() {
        let mut parts = Vec::new();
        if !unsafe_paths.is_empty() {
            parts.push(format!(
                "대상 폴더 밖을 가리키는 항목 {}개를 건너뛰었습니다({})",
                unsafe_paths.len(),
                summarize(&unsafe_paths.iter().cloned().collect::<Vec<_>>())
            ));
        }
        if !failed_paths.is_empty() {
            parts.push(format!(
                "파일 {}개를 쓰지 못했습니다({})",
                failed_paths.len(),
                summarize(
                    &failed_paths
                        .iter()
                        .map(|(p, why)| format!("{p}: {why}"))
                        .collect::<Vec<_>>()
                )
            ));
        }
        return ExtractResult::Done {
            status: "warning",
            message: format!("해제를 마쳤지만 일부 항목이 빠졌습니다. {}.", parts.join(" / ")),
        };
    }
    ExtractResult::Done {
        status: "ok",
        message: "해제를 완료했습니다.".to_string(),
    }
}

/// crate::inputs 결과 → 7z 항목, 시각 → FILETIME, keep_index = None(신규)
fn collect_items(inputs: &[String]) -> (Vec<UpdateItem>, Vec<String>) {
    let (found, skipped) = crate::inputs::collect(inputs);
    let items = found
        .into_iter()
        .map(|i| UpdateItem {
            name: i.name,
            source: i.source,
            size: i.size,
            is_dir: i.is_dir,
            keep_index: None,
            mtime: i.mtime.map(to_filetime).unwrap_or_else(now_filetime),
        })
        .collect();
    (items, skipped)
}

/// SystemTime → FILETIME(100ns, 1601 기준)
fn to_filetime(t: SystemTime) -> u64 {
    let secs = t.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    (secs + 11_644_473_600) * 10_000_000
}

/// 압축 생성
fn do_create(
    sz: &SevenZip,
    opts: &CreateOptions,
    on_progress: &mut ProgressFn<'_>,
    cancel: Arc<AtomicBool>,
) -> CreateResult {
    if opts.inputs.is_empty() {
        return CreateResult::Failed(ZipManiaError::new("no_input", "압축할 파일이 없습니다."));
    }
    let (items, skipped) = collect_items(&opts.inputs);
    if items.is_empty() {
        return CreateResult::Failed(ZipManiaError::new("no_input", "압축할 파일이 없습니다."));
    }

    // 기존 파일 선삭제 금지, 임시 파일 완성 후 이동
    let out_path = PathBuf::from(&opts.output);
    // 독점 생성으로 자리 선점, 이름만 생성 금지
    let mut tmp_path = match reserve_tmp(&out_path) {
        Ok(p) => p,
        Err(e) => return CreateResult::Failed(e),
    };

    let dll = match sz.load() {
        Ok(d) => d,
        Err(e) => return CreateResult::Failed(e),
    };
    let out_arc = match dll.create_out_archive(clsid_id(opts.format)) {
        Ok(a) => a,
        Err(e) => return CreateResult::Failed(e),
    };

    // 암호/헤더암호 확정
    let use_password = opts.format.supports_password()
        && opts
            .password
            .as_deref()
            .map(|p| !p.is_empty())
            .unwrap_or(false);
    let header_enc = use_password && opts.encrypt_names && opts.format.supports_header_encryption();

    // 옵션 설정: 압축 레벨 "x", 헤더암호 "he"
    if opts.format.has_level() || header_enc {
        let setp: ISetProperties = match out_arc.cast() {
            Ok(s) => s,
            Err(e) => {
                return CreateResult::Failed(ZipManiaError::new(
                    "engine_error",
                    format!("옵션 설정 인터페이스를 얻지 못했습니다: {e:?}"),
                ))
            }
        };
        let mut names: Vec<Vec<u16>> = Vec::new();
        let mut values: Vec<prop::PropVariant> = Vec::new();
        if opts.format.has_level() {
            names.push(ffi::to_wide_nul("x"));
            let mut pv = prop::PropVariant::empty();
            pv.set_u32(normalize_level(opts.level));
            values.push(pv);
        }
        if header_enc {
            names.push(ffi::to_wide_nul("he"));
            let mut pv = prop::PropVariant::empty();
            pv.set_bool(true);
            values.push(pv);
        }
        let name_ptrs: Vec<*const u16> = names.iter().map(|n| n.as_ptr()).collect();
        let hr = unsafe {
            setp.SetProperties(name_ptrs.as_ptr(), values.as_ptr(), values.len() as u32)
        };
        if hr != S_OK {
            return CreateResult::Failed(ZipManiaError::new(
                "engine_error",
                format!("압축 옵션 설정 실패(hr=0x{:08X}).", hr.0),
            ));
        }
    }

    // 경로로 재개방 금지, 만든 핸들 그대로 전달
    let Some(tmp_file) = tmp_path.take_file() else {
        return CreateResult::Failed(ZipManiaError::new(
            "output_error",
            "임시 파일 핸들을 얻지 못했습니다.",
        ));
    };
    let out_stream = match streams::output_seekable_file(tmp_file) {
        Ok(s) => s,
        Err(e) => return CreateResult::Failed(e),
    };

    let num = items.len() as u32;
    let sink = ProgressSink::new(&mut *on_progress);
    let (cb, shared) = callbacks::make_update_cb(UpdateCfg {
        items: Arc::new(items),
        password: if use_password {
            opts.password.as_deref()
        } else {
            None
        },
        progress: Some(sink),
        cancel: cancel.clone(),
    });

    let hr = unsafe { out_arc.UpdateItems(out_stream.as_raw(), num, cb.as_raw()) };
    // 출력 스트림, 아카이브 드롭 → 파일 핸들 닫힘
    drop(out_stream);
    drop(cb);
    drop(out_arc);

    if shared.aborted.load(std::sync::atomic::Ordering::SeqCst) {
        let _ = std::fs::remove_file(&tmp_path);
        return CreateResult::Done {
            status: "canceled",
            message: "압축을 취소했습니다. 생성 중이던 아카이브 파일은 삭제했습니다.".to_string(),
        };
    }

    let op = *shared.op_result.lock().unwrap();
    if hr != S_OK || op != 0 {
        let _ = std::fs::remove_file(&tmp_path);
        if op != 0 {
            return CreateResult::Failed(error::classify_operation(
                op,
                use_password,
                opts.password.is_some(),
            ));
        }
        return CreateResult::Failed(ZipManiaError::new(
            "engine_error",
            format!("압축에 실패했습니다(hr=0x{:08X}).", hr.0),
        ));
    }
    // TmpPath::commit 필수, commit_replace 직접 호출 금지
    if let Err(e) = tmp_path.commit() {
        return CreateResult::Failed(e);
    }
    // 담지 못한 항목 있으면 ok 아님
    if !skipped.is_empty() {
        return CreateResult::Done {
            status: "warning",
            message: format!(
                "압축을 마쳤지만 {}개 항목을 담지 못했습니다: {}",
                skipped.len(),
                summarize(&skipped)
            ),
        };
    }
    CreateResult::Done {
        status: "ok",
        message: "압축을 완료했습니다.".to_string(),
    }
}

/// 아카이브 편집, IInArchive → 같은 핸들러 IOutArchive 캐스팅 → UpdateItems
/// 유지 항목 = keep_index(new_data=0, 재압축 없음), 출력 = 임시 파일 → 성공 시 교체
fn do_edit(
    sz: &SevenZip,
    opts: &EditOptions,
    on_progress: &mut ProgressFn<'_>,
    cancel: Arc<AtomicBool>,
) -> CreateResult {
    // 편집 가능 포맷 = 7z/zip/tar, 나머지 미지원
    let ext = Path::new(&opts.archive)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    if !matches!(ext.as_str(), "7z" | "zip" | "tar") {
        return CreateResult::Failed(ZipManiaError::new(
            "unsupported",
            "이 형식은 편집을 지원하지 않습니다.",
        ));
    }
    if opts.add.is_empty() && opts.remove.is_empty() {
        return CreateResult::Failed(ZipManiaError::new("no_input", "편집할 내용이 없습니다."));
    }

    let dll = match sz.load() {
        Ok(d) => d,
        Err(e) => return CreateResult::Failed(e),
    };
    let password = opts.password.as_deref();
    let in_arc = match open_for_read(&dll, &opts.archive, password) {
        Ok(a) => a,
        Err(e) => return CreateResult::Failed(e),
    };

    // 기존 항목 (경로, is_dir) 인덱스 순 수집
    let meta = unsafe { collect_meta(&in_arc, &opts.archive) };

    // 삭제 대상: remove 경로 자신 또는 그 하위(폴더 서브트리), / 정규화 비교
    let removes: Vec<String> = opts.remove.iter().map(|s| s.replace('\\', "/")).collect();
    // 신규 항목(디스크), 이름 겹치면 기존 것 교체
    let (new_items, skipped) = collect_items(&opts.add);
    let new_names: std::collections::HashSet<String> =
        new_items.iter().map(|i| i.name.replace('\\', "/")).collect();

    let is_removed = |path_norm: &str| -> bool {
        removes
            .iter()
            .any(|r| path_norm == r || path_norm.starts_with(&format!("{r}/")))
            || new_names.contains(path_norm)
    };

    // 출력 항목: 유지분(keep_index)을 먼저, 신규분을 뒤에
    let mut items: Vec<UpdateItem> = Vec::with_capacity(meta.len() + new_items.len());
    for (i, (path, is_dir)) in meta.iter().enumerate() {
        let path_norm = path.replace('\\', "/");
        if is_removed(&path_norm) {
            continue;
        }
        items.push(UpdateItem {
            name: path.clone(),
            source: None,
            size: 0,
            is_dir: *is_dir,
            mtime: 0,
            keep_index: Some(i as u32),
        });
    }
    items.extend(new_items);

    // 같은 핸들러를 IOutArchive 로 캐스팅(편집 미지원 핸들러면 실패)
    let out_arc: IOutArchive = match in_arc.cast() {
        Ok(o) => o,
        Err(_) => {
            return CreateResult::Failed(ZipManiaError::new(
                "unsupported",
                "이 형식은 편집을 지원하지 않습니다.",
            ))
        }
    };

    // 암호 = 신규 항목에만 적용, 유지 항목은 원본 블록 복사라 무관
    let use_password = password.map(|p| !p.is_empty()).unwrap_or(false);

    // 임시 출력 파일 → 성공 시 원본 교체
    let arc_path = PathBuf::from(&opts.archive);
    let mut tmp_path = match reserve_tmp(&arc_path) {
        Ok(p) => p,
        Err(e) => return CreateResult::Failed(e),
    };
    // 경로로 재개방 금지, 만든 핸들 그대로 전달
    let Some(tmp_file) = tmp_path.take_file() else {
        return CreateResult::Failed(ZipManiaError::new(
            "output_error",
            "임시 파일 핸들을 얻지 못했습니다.",
        ));
    };
    let out_stream = match streams::output_seekable_file(tmp_file) {
        Ok(s) => s,
        Err(e) => return CreateResult::Failed(e),
    };

    let num = items.len() as u32;
    let sink = ProgressSink::new(&mut *on_progress);
    let (cb, shared) = callbacks::make_update_cb(UpdateCfg {
        items: Arc::new(items),
        password: if use_password { password } else { None },
        progress: Some(sink),
        cancel: cancel.clone(),
    });

    let hr = unsafe { out_arc.UpdateItems(out_stream.as_raw(), num, cb.as_raw()) };
    // 핸들 정리: 출력 스트림, 콜백, 아카이브(입출력 동일 객체) 드롭 → 원본 핸들 해제
    drop(out_stream);
    drop(cb);
    drop(out_arc);
    drop(in_arc);

    if shared.aborted.load(std::sync::atomic::Ordering::SeqCst) {
        let _ = std::fs::remove_file(&tmp_path);
        return CreateResult::Done {
            status: "canceled",
            message: "편집을 취소했습니다.".to_string(),
        };
    }

    let op = *shared.op_result.lock().unwrap();
    if hr != S_OK || op != 0 {
        let _ = std::fs::remove_file(&tmp_path);
        if op != 0 {
            return CreateResult::Failed(error::classify_operation(
                op,
                use_password,
                password.is_some(),
            ));
        }
        return CreateResult::Failed(ZipManiaError::new(
            "engine_error",
            format!("편집에 실패했습니다(hr=0x{:08X}).", hr.0),
        ));
    }

    // 삭제 없이 덮어쓰기, 삭제 후 rename 금지
    if let Err(e) = tmp_path.commit() {
        return CreateResult::Failed(e);
    }

    if !skipped.is_empty() {
        return CreateResult::Done {
            status: "warning",
            message: format!(
                "편집을 마쳤지만 {}개 항목을 담지 못했습니다: {}",
                skipped.len(),
                summarize(&skipped)
            ),
        };
    }
    CreateResult::Done {
        status: "ok",
        message: "편집을 완료했습니다.".to_string(),
    }
}

/// 무결성 테스트
fn do_test(sz: &SevenZip, archive: &str, password: Option<&str>) -> Result<(), ZipManiaError> {
    let dll = sz.load()?;
    let arc = open_for_read(&dll, archive, password)?;
    let meta = Arc::new(unsafe { collect_meta(&arc, archive) });
    let cancel = Arc::new(AtomicBool::new(false));
    let (cb, shared) = callbacks::make_extract_cb(ExtractCfg {
        entries: meta,
        // 파일 안 쓰는 경로 → 크기 대조 없음
        sizes: Arc::new(Vec::new()),
        dest: PathBuf::new(),
        keep_paths: true,
        overwrite: OverwriteMode::Overwrite,
        decisions: Default::default(),
        test_mode: true,
        password,
        progress: None,
        cancel,
        mem_target: None,
        file_target: None,
        writer_target: None,
        crc_report: None,
        scan_report: None,
    });
    let hr = unsafe { arc.Extract(std::ptr::null(), u32::MAX, 1, cb.as_raw()) };
    unsafe {
        let _ = arc.Close();
    }
    let op = *shared.op_result.lock().unwrap();
    if op != 0 {
        let crypto = shared
            .crypto_requested
            .load(std::sync::atomic::Ordering::SeqCst);
        return Err(error::classify_operation(op, crypto, password.is_some()));
    }
    if hr != S_OK {
        return Err(ZipManiaError::new("corrupt", "무결성 테스트에 실패했습니다."));
    }
    Ok(())
}

/// 무결성 테스트(상세), 각 파일 → CRC 싱크(디스크 미기록)
/// 암호 문제 = 전역 실패, 개별 CRC, 데이터 오류 = 항목 ok=false, (D3.5)
fn do_test_report(
    sz: &SevenZip,
    archive: &str,
    password: Option<&str>,
    on_progress: &mut ProgressFn<'_>,
    cancel: Arc<AtomicBool>,
) -> Result<Vec<TestEntry>, ZipManiaError> {
    let dll = sz.load()?;
    let arc = open_for_read(&dll, archive, password)?;
    let meta = Arc::new(unsafe { collect_meta(&arc, archive) });
    let expected = Arc::new(unsafe { collect_crcs(&arc, meta.len()) });
    let out = Arc::new(Mutex::new(Vec::<TestEntry>::new()));

    let sink = ProgressSink::new(&mut *on_progress);
    let (cb, shared) = callbacks::make_extract_cb(ExtractCfg {
        entries: meta.clone(),
        // 파일 안 쓰는 경로 → 크기 대조 없음
        sizes: Arc::new(Vec::new()),
        dest: PathBuf::new(),
        keep_paths: true,
        overwrite: OverwriteMode::Overwrite,
        decisions: Default::default(),
        test_mode: false, // kExtract 로 실제 복호화해 CRC 싱크에 흘려보낸다
        password,
        progress: Some(sink),
        cancel: cancel.clone(),
        mem_target: None,
        file_target: None,
        writer_target: None,
        crc_report: Some(callbacks::CrcReportCfg {
            expected: expected.clone(),
            out: out.clone(),
        }),
        scan_report: None,
    });

    // 전체 해제(인덱스 null), 데이터 → CRC 싱크
    let hr = unsafe { arc.Extract(std::ptr::null(), u32::MAX, 0, cb.as_raw()) };
    unsafe {
        let _ = arc.Close();
    }

    if shared.aborted.load(std::sync::atomic::Ordering::SeqCst) {
        return Err(ZipManiaError::new("canceled", "테스트를 취소했습니다."));
    }

    let op = *shared.op_result.lock().unwrap();
    let crypto = shared
        .crypto_requested
        .load(std::sync::atomic::Ordering::SeqCst);
    // 암호 요청 + 실패 → 암호 문제(전역 실패)
    if crypto && op != 0 {
        return Err(error::classify_operation(op, crypto, password.is_some()));
    }
    // hr 실패 + opResult 없음 + 암호 요청 → 암호 오류
    if hr != S_OK && crypto {
        return Err(error::classify_open_failure(hr.0, crypto, password.is_some()));
    }

    let report = out.lock().unwrap().clone();
    finish_test_report(hr == S_OK, meta.len(), report)
}

/// hr + 결과 항목 수 → 보고서 확정, 부분 결과 = 성공 아님(D3.5)
fn finish_test_report(
    completed: bool,
    expected_items: usize,
    report: Vec<TestEntry>,
) -> Result<Vec<TestEntry>, ZipManiaError> {
    if !completed {
        return Err(ZipManiaError::new(
            "corrupt",
            "무결성 테스트를 끝까지 마치지 못했습니다.",
        ));
    }
    if report.len() < expected_items {
        return Err(ZipManiaError::new(
            "corrupt",
            format!(
                "일부 항목을 검사하지 못했습니다({}/{} 항목).",
                report.len(),
                expected_items
            ),
        ));
    }
    Ok(report)
}

#[cfg(test)]
mod test_report_tests {
    use super::finish_test_report;
    use crate::models::TestEntry;

    fn entry(ok: bool) -> TestEntry {
        TestEntry {
            path: "a.txt".into(),
            is_dir: false,
            expected_crc: Some(1),
            actual_crc: Some(if ok { 1 } else { 2 }),
            ok,
        }
    }

    /// 미완료 → 부분 결과를 성공으로 주지 않음
    #[test]
    fn 마치지_못한_검사는_성공이_아니다() {
        let e = finish_test_report(false, 1, vec![entry(true)]).expect_err("부분 결과를 성공으로 줬다");
        assert_eq!(e.code, "corrupt");
        // 결과 0개도 같다
        assert!(finish_test_report(false, 0, Vec::new()).is_err());
    }

    /// 운영 코드가 판정 함수를 거치는지 소스로 확인(D3.5)
    #[test]
    fn 상세_테스트는_판정_함수를_거친다() {
        let src = include_str!("mod.rs");
        assert!(
            src.contains("finish_test_report(hr == S_OK, meta.len(), report)"),
            "do_test_report 가 판정 함수를 거치지 않는다(부분 결과가 성공으로 샌다)"
        );
    }

    /// 결과 없는 항목 있으면 성공 아님
    #[test]
    fn 결과가_빠진_항목이_있으면_성공이_아니다() {
        let e = finish_test_report(true, 3, vec![entry(true), entry(true)])
            .expect_err("두 개만 검사하고 성공으로 줬다");
        assert_eq!(e.code, "corrupt");
        assert!(e.message.contains("2/3"), "몇 개가 빠졌는지 알려 주지 않는다: {}", e.message);
        // 목록 수만큼 도착 시 통과
        assert!(finish_test_report(true, 2, vec![entry(true), entry(true)]).is_ok());
    }

    /// 개별 CRC 오류 = 오류 아닌 결과, 마쳤으면 그대로 반환
    #[test]
    fn 마친_검사는_항목_실패를_그대로_돌려준다() {
        let r = finish_test_report(true, 2, vec![entry(true), entry(false)]).expect("마친 검사다");
        assert_eq!(r.len(), 2);
        assert!(!r[1].ok, "깨진 항목 표시가 사라졌다");
    }
}

/// 바이러스 검사, 각 파일 → 메모리(max_size 미만) → scan 콜백 → (경로, 크기, 상태)
/// max_size 이상 = skipped, AMSI 로직은 호출측 주입
fn do_scan_report(
    sz: &SevenZip,
    archive: &str,
    password: Option<&str>,
    max_size: u64,
    scan: ScanFn,
    on_progress: &mut ProgressFn<'_>,
    cancel: Arc<AtomicBool>,
) -> Result<Vec<ScanEntry>, ZipManiaError> {
    let dll = sz.load()?;
    let arc = open_for_read(&dll, archive, password)?;
    // 목록 못 얻음 vs 빈 아카이브 구분 필수
    let Some(count) = (unsafe { item_count(&arc) }) else {
        return Err(ZipManiaError::new(
            "corrupt",
            "아카이브 목록을 읽지 못해 검사하지 못했습니다.",
        ));
    };
    // 항목 수는 1회만 조회, 속성 못 읽은 항목은 인덱스로 수신(D3.5)
    let (meta, unreadable) = unsafe { collect_meta_checked(&arc, archive, count) };
    let meta = Arc::new(meta);
    let sizes = Arc::new(unsafe { collect_sizes(&arc, meta.len()) });
    let out = Arc::new(Mutex::new(Vec::<ScanEntry>::new()));
    let seen = Arc::new(Mutex::new(std::collections::HashSet::<u32>::new()));

    let sink = ProgressSink::new(&mut *on_progress);
    let (cb, shared) = callbacks::make_extract_cb(ExtractCfg {
        entries: meta.clone(),
        // 파일 안 쓰는 경로 → 크기 대조 없음
        sizes: Arc::new(Vec::new()),
        dest: PathBuf::new(),
        keep_paths: true,
        overwrite: OverwriteMode::Overwrite,
        decisions: Default::default(),
        test_mode: false,
        password,
        progress: Some(sink),
        cancel: cancel.clone(),
        mem_target: None,
        file_target: None,
        writer_target: None,
        crc_report: None,
        scan_report: Some(callbacks::ScanReportCfg {
            max_size,
            sizes: sizes.clone(),
            scan,
            out: out.clone(),
            seen: seen.clone(),
        }),
    });

    let hr = unsafe { arc.Extract(std::ptr::null(), u32::MAX, 0, cb.as_raw()) };
    unsafe {
        let _ = arc.Close();
    }

    if shared.aborted.load(std::sync::atomic::Ordering::SeqCst) {
        return Err(ZipManiaError::new("canceled", "검사를 취소했습니다."));
    }

    let op = *shared.op_result.lock().unwrap();
    let crypto = shared
        .crypto_requested
        .load(std::sync::atomic::Ordering::SeqCst);
    // 암호 문제는 전역 실패로(프론트가 암호 재입력)
    if crypto && op != 0 {
        return Err(error::classify_operation(op, crypto, password.is_some()));
    }
    if hr != S_OK && crypto {
        return Err(error::classify_open_failure(hr.0, crypto, password.is_some()));
    }

    // 결과 없는 항목 → error 채움(incomplete 로 올라감), 셈은 인덱스로(D3.5)
    let mut report = out.lock().unwrap().clone();
    let scanned = seen.lock().unwrap();
    for (i, (path, is_dir)) in meta.iter().enumerate() {
        if *is_dir || scanned.contains(&(i as u32)) {
            continue;
        }
        report.push(ScanEntry {
            path: path.replace('\\', "/"),
            is_dir: false,
            size: sizes.get(i).copied().flatten().unwrap_or(0),
            status: "error".to_string(),
        });
    }

    // 전역 오류(hr, op)가 있었으면 항목이 전부 결과를 냈더라도 clean 이 아니다
    if hr != S_OK || op != 0 {
        if report.is_empty() {
            return Err(ZipManiaError::new(
                "corrupt",
                "아카이브를 읽지 못해 검사하지 못했습니다.",
            ));
        }
        report.push(ScanEntry {
            path: format!("({archive} — 아카이브 오류로 끝까지 검사하지 못함)"),
            is_dir: false,
            size: 0,
            status: "error".to_string(),
        });
    }

    // 목록 못 읽은 항목 = 검사하지 못함
    if !unreadable.is_empty() {
        report.push(ScanEntry {
            path: format!(
                "({archive} — 항목 {}개의 정보를 읽지 못함)",
                unreadable.len()
            ),
            is_dir: false,
            size: 0,
            status: "error".to_string(),
        });
    }
    Ok(report)
}

/// 뷰어용, 내부 파일 1개 → 메모리(임시 해제 없음)
fn do_read_entry(
    sz: &SevenZip,
    archive: &str,
    inner_path: &str,
    password: Option<&str>,
) -> Result<Vec<u8>, ZipManiaError> {
    let dll = sz.load()?;
    let arc = open_for_read(&dll, archive, password)?;
    let meta = Arc::new(unsafe { collect_meta(&arc, archive) });

    // 내부 경로(구분자 무관)로 대상 인덱스를 찾는다
    let want = inner_path.replace('\\', "/");
    let target = meta
        .iter()
        .position(|(p, is_dir)| !is_dir && p.replace('\\', "/") == want)
        .map(|i| i as u32);
    let target = match target {
        Some(t) => t,
        None => {
            unsafe {
                let _ = arc.Close();
            }
            return Err(ZipManiaError::new(
                "not_found",
                "아카이브 안에서 해당 항목을 찾지 못했습니다.",
            ));
        }
    };

    let buf = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let cancel = Arc::new(AtomicBool::new(false));
    let (cb, shared) = callbacks::make_extract_cb(ExtractCfg {
        entries: meta,
        // 파일 안 쓰는 경로 → 크기 대조 없음
        sizes: Arc::new(Vec::new()),
        dest: PathBuf::new(),
        keep_paths: false,
        overwrite: OverwriteMode::Overwrite,
        decisions: Default::default(),
        test_mode: false,
        password,
        progress: None,
        cancel,
        mem_target: Some((target, buf.clone())),
        file_target: None,
        writer_target: None,
        crc_report: None,
        scan_report: None,
    });
    let indices = [target];
    let hr = unsafe { arc.Extract(indices.as_ptr(), 1, 0, cb.as_raw()) };
    unsafe {
        let _ = arc.Close();
    }

    let op = *shared.op_result.lock().unwrap();
    if op != 0 {
        let crypto = shared
            .crypto_requested
            .load(std::sync::atomic::Ordering::SeqCst);
        return Err(error::classify_operation(op, crypto, password.is_some()));
    }
    if hr != S_OK {
        return Err(ZipManiaError::new("read_error", "항목을 읽지 못했습니다."));
    }
    let out = buf.lock().unwrap().clone();
    Ok(out)
}

/// 단일 항목 → 지정 파일 스트리밍 추출, Shell DnD 지연 렌더링용
fn do_extract_entry_to_file(
    sz: &SevenZip,
    archive: &str,
    inner_path: &str,
    dest_file: &Path,
    password: Option<&str>,
) -> Result<(), ZipManiaError> {
    let dll = sz.load()?;
    let arc = open_for_read(&dll, archive, password)?;
    let meta = Arc::new(unsafe { collect_meta(&arc, archive) });
    // 파일 쓰는 경로 → 크기 대조(settle_pending)
    let sizes = Arc::new(unsafe { collect_sizes(&arc, meta.len()) });

    let want = inner_path.replace('\\', "/");
    let target = meta
        .iter()
        .position(|(p, is_dir)| !is_dir && p.replace('\\', "/") == want)
        .map(|i| i as u32);
    let target = match target {
        Some(t) => t,
        None => {
            unsafe {
                let _ = arc.Close();
            }
            return Err(ZipManiaError::new(
                "not_found",
                "아카이브 안에서 해당 항목을 찾지 못했습니다.",
            ));
        }
    };

    // 출력 파일의 부모 폴더 보장
    if let Some(parent) = dest_file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let cancel = Arc::new(AtomicBool::new(false));
    let (cb, shared) = callbacks::make_extract_cb(ExtractCfg {
        entries: meta,
        sizes,
        dest: PathBuf::new(),
        keep_paths: false,
        overwrite: OverwriteMode::Overwrite,
        decisions: Default::default(),
        test_mode: false,
        password,
        progress: None,
        cancel,
        mem_target: None,
        file_target: Some((target, dest_file.to_path_buf())),
        writer_target: None,
        crc_report: None,
        scan_report: None,
    });
    let indices = [target];
    let hr = unsafe { arc.Extract(indices.as_ptr(), 1, 0, cb.as_raw()) };
    unsafe {
        let _ = arc.Close();
    }

    let op = *shared.op_result.lock().unwrap();
    if op != 0 {
        let crypto = shared
            .crypto_requested
            .load(std::sync::atomic::Ordering::SeqCst);
        return Err(error::classify_operation(op, crypto, password.is_some()));
    }
    if hr != S_OK {
        return Err(ZipManiaError::new("read_error", "항목을 읽지 못했습니다."));
    }
    // 성공 → 임시 파일 이동, 오류 경로에서는 미호출 → Drop 이 정리
    shared.settle_pending();
    // 옮기지 못한 것 = 실패, failed_paths 확인 필수
    if let Some((path, why)) = shared.failed_paths.lock().unwrap().first() {
        return Err(ZipManiaError::new(
            "output_error",
            format!("항목을 제자리에 놓지 못했습니다({path}): {why}"),
        ));
    }
    Ok(())
}

/// 단일 항목 → 임의 writer 순차 스트리밍(임시 파일, 전체 메모리 없음), Shell DnD 스트리밍용
fn do_extract_entry_to_writer(
    sz: &SevenZip,
    archive: &str,
    inner_path: &str,
    writer: Box<dyn std::io::Write + Send>,
    password: Option<&str>,
) -> Result<(), ZipManiaError> {
    let dll = sz.load()?;
    let arc = open_for_read(&dll, archive, password)?;
    let meta = Arc::new(unsafe { collect_meta(&arc, archive) });

    let want = inner_path.replace('\\', "/");
    let target = meta
        .iter()
        .position(|(p, is_dir)| !is_dir && p.replace('\\', "/") == want)
        .map(|i| i as u32);
    let target = match target {
        Some(t) => t,
        None => {
            unsafe {
                let _ = arc.Close();
            }
            return Err(ZipManiaError::new(
                "not_found",
                "아카이브 안에서 해당 항목을 찾지 못했습니다.",
            ));
        }
    };

    let cancel = Arc::new(AtomicBool::new(false));
    let (cb, shared) = callbacks::make_extract_cb(ExtractCfg {
        entries: meta,
        // 파일 안 쓰는 경로 → 크기 대조 없음
        sizes: Arc::new(Vec::new()),
        dest: PathBuf::new(),
        keep_paths: false,
        overwrite: OverwriteMode::Overwrite,
        decisions: Default::default(),
        test_mode: false,
        password,
        progress: None,
        cancel,
        mem_target: None,
        file_target: None,
        writer_target: Some((target, writer)),
        crc_report: None,
        scan_report: None,
    });
    let indices = [target];
    let hr = unsafe { arc.Extract(indices.as_ptr(), 1, 0, cb.as_raw()) };
    unsafe {
        let _ = arc.Close();
    }

    let op = *shared.op_result.lock().unwrap();
    if op != 0 {
        let crypto = shared
            .crypto_requested
            .load(std::sync::atomic::Ordering::SeqCst);
        return Err(error::classify_operation(op, crypto, password.is_some()));
    }
    if hr != S_OK {
        return Err(ZipManiaError::new("read_error", "항목을 읽지 못했습니다."));
    }
    Ok(())
}

// ─────────────────────────── 트레이트 구현 ───────────────────────────

impl ArchiveBackend for SevenZip {
    fn id(&self) -> &'static str {
        "sevenzip"
    }

    fn read_exts(&self) -> &'static [&'static str] {
        // 7z.dll 담당분만 선언, egg/alz = unegg 소관
        crate::formats::SEVENZIP_EXTS
    }

    /// 후보 핸들러 순회 → 확장자 없거나 모르는 파일도 담당
    fn accepts_unknown(&self) -> bool {
        true
    }

    fn engine_version(&self) -> Result<String, ZipManiaError> {
        SevenZip::version(self)
    }

    fn write_exts(&self) -> &'static [&'static str] {
        &["7z", "zip", "tar"]
    }

    fn list(&self, archive: &str, password: Option<&str>) -> Result<Vec<ArchiveEntry>, ZipManiaError> {
        do_list(self, archive, password)
    }

    fn extract(
        &self,
        opts: &ExtractOptions,
        on_progress: &mut ProgressFn<'_>,
        cancel: Arc<AtomicBool>,
    ) -> ExtractResult {
        do_extract(self, opts, on_progress, cancel)
    }

    fn create(
        &self,
        opts: &CreateOptions,
        on_progress: &mut ProgressFn<'_>,
        cancel: Arc<AtomicBool>,
    ) -> CreateResult {
        do_create(self, opts, on_progress, cancel)
    }

    fn edit(
        &self,
        opts: &EditOptions,
        on_progress: &mut ProgressFn<'_>,
        cancel: Arc<AtomicBool>,
    ) -> CreateResult {
        do_edit(self, opts, on_progress, cancel)
    }

    fn test(&self, archive: &str, password: Option<&str>) -> Result<(), ZipManiaError> {
        do_test(self, archive, password)
    }

    fn test_report(
        &self,
        archive: &str,
        password: Option<&str>,
        on_progress: &mut ProgressFn<'_>,
        cancel: Arc<AtomicBool>,
    ) -> Result<Vec<TestEntry>, ZipManiaError> {
        do_test_report(self, archive, password, on_progress, cancel)
    }

    fn scan_report(
        &self,
        archive: &str,
        password: Option<&str>,
        max_size: u64,
        scan: ScanFn,
        on_progress: &mut ProgressFn<'_>,
        cancel: Arc<AtomicBool>,
    ) -> Result<Vec<ScanEntry>, ZipManiaError> {
        do_scan_report(self, archive, password, max_size, scan, on_progress, cancel)
    }

    fn read_entry_to_memory(
        &self,
        archive: &str,
        inner_path: &str,
        password: Option<&str>,
    ) -> Result<Vec<u8>, ZipManiaError> {
        do_read_entry(self, archive, inner_path, password)
    }

    fn extract_entry_to_file(
        &self,
        archive: &str,
        inner_path: &str,
        dest_file: &Path,
        password: Option<&str>,
    ) -> Result<(), ZipManiaError> {
        do_extract_entry_to_file(self, archive, inner_path, dest_file, password)
    }

    fn extract_entry_to_writer(
        &self,
        archive: &str,
        inner_path: &str,
        writer: Box<dyn std::io::Write + Send>,
        password: Option<&str>,
    ) -> Result<(), ZipManiaError> {
        do_extract_entry_to_writer(self, archive, inner_path, writer, password)
    }
}

// ─────────────────────── 확장자 → 핸들러 매핑 테스트 ───────────────────────
// 정본, 사본 대조 = crate::formats ext_tests, 여기서는 핸들러 id 매핑 유무만

#[cfg(test)]
mod ext_tests {
    use super::*;

    #[test]
    fn 정본의_모든_확장자에_핸들러_매핑이_있다() {
            // 매핑 없으면 7z/zip 폴백만 시도하다 실패
        let unmapped: Vec<&&str> = crate::formats::SEVENZIP_EXTS
            .iter()
            .filter(|e| format_ids_for_ext(e).is_empty())
            .collect();
        assert!(
            unmapped.is_empty(),
            "핸들러 id 매핑이 없는 확장자: {unmapped:?} → format_ids_for_ext 에 추가하십시오."
        );
    }

    #[test]
    fn rar_는_rar5_를_먼저_시도한다() {
        // RAR5(0xCC) 우선 필요
        assert_eq!(format_ids_for_ext("rar"), &[0xCC, 0x03]);
    }
}

// ─────────────────────────── 옵션 enum 단위 테스트 ───────────────────────────

#[cfg(test)]
mod option_tests {
    use super::*;

    #[test]
    fn 덮어쓰기_문자열_해석() {
        assert_eq!(OverwriteMode::from_str("overwrite"), OverwriteMode::Overwrite);
        assert_eq!(OverwriteMode::from_str("skip"), OverwriteMode::Skip);
        assert_eq!(OverwriteMode::from_str("묻기"), OverwriteMode::Skip);
    }

    #[test]
    fn 압축_포맷_능력_판정() {
        assert!(CompressFormat::SevenZip.supports_password());
        assert!(CompressFormat::SevenZip.supports_header_encryption());
        assert!(CompressFormat::Zip.supports_password());
        assert!(!CompressFormat::Zip.supports_header_encryption());
        assert!(!CompressFormat::Tar.supports_password());
        assert!(!CompressFormat::Tar.has_level());
        assert_eq!(CompressFormat::from_str("zip"), CompressFormat::Zip);
        assert_eq!(CompressFormat::from_str("tar"), CompressFormat::Tar);
        assert_eq!(CompressFormat::from_str("기타"), CompressFormat::SevenZip);
    }

    #[test]
    fn 포맷_clsid_id() {
        assert_eq!(clsid_id(CompressFormat::SevenZip), 0x07);
        assert_eq!(clsid_id(CompressFormat::Zip), 0x01);
        assert_eq!(clsid_id(CompressFormat::Tar), 0xEE);
    }

    #[test]
    fn 레벨_정규화() {
        assert_eq!(normalize_level(0), 0);
        assert_eq!(normalize_level(1), 1);
        assert_eq!(normalize_level(4), 3);
        assert_eq!(normalize_level(5), 5);
        assert_eq!(normalize_level(100), 9);
    }

    #[test]
    fn 확장자_포맷_매핑() {
        assert_eq!(format_ids_for_ext("7z"), &[0x07]);
        assert_eq!(format_ids_for_ext("zip"), &[0x01]);
        assert!(format_ids_for_ext("모름").is_empty());
        // 후보는 확장자 우선 + 7z/zip 폴백
        assert_eq!(candidate_ids("a.7z"), vec![0x07, 0x01]);
        assert_eq!(candidate_ids("a.zip"), vec![0x01, 0x07]);
        assert_eq!(candidate_ids("a.unknown"), vec![0x07, 0x01]);
        // 핸들러 여럿 = 순서대로 + 폴백
        assert_eq!(candidate_ids("a.rar"), vec![0xCC, 0x03, 0x07, 0x01]);
    }
}

// ─────────────────────────── 통합 테스트(실제 7z.dll) ───────────────────────────

#[cfg(test)]
mod integration_tests {
    use super::*;
    use std::fs;
    use std::process::Command;
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    /// 번들 7z.dll 경로
    fn dll() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("src-tauri")
            .join("binaries")
            .join("7z.dll")
    }
    /// 픽스처 생성, 교차검증용 7z.exe(배포 미포함)
    fn exe() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("src-tauri")
            .join("binaries")
            .join("7z.exe")
    }
    fn backend() -> SevenZip {
        SevenZip::new(dll())
    }

    struct TempDir {
        path: PathBuf,
    }
    impl TempDir {
        fn new(tag: &str) -> Self {
            let mut path = std::env::temp_dir();
            path.push(format!("zipmania_dll_{}_{}", std::process::id(), tag));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("임시 폴더 생성 실패");
            TempDir { path }
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    /// 7z.exe 로 픽스처 아카이브 생성
    fn mk_fixture(dir: &Path, extra: &[&str], out: &str, inputs: &[&str]) {
        let mut cmd = Command::new(exe());
        cmd.current_dir(dir);
        cmd.args(["a", "-sccUTF-8", "-y"]);
        cmd.args(extra);
        cmd.arg(out);
        cmd.args(inputs);
        let o = cmd.output().expect("7z.exe 픽스처 생성 실행 실패");
        assert!(o.status.success(), "픽스처 생성 실패: {}", String::from_utf8_lossy(&o.stderr));
    }

    /// 7z.exe t 교차검증
    fn exe_test(archive: &Path, password: Option<&str>) -> bool {
        let mut cmd = Command::new(exe());
        cmd.args(["t", "-y"]);
        if let Some(p) = password {
            cmd.arg(format!("-p{p}"));
        }
        cmd.arg(archive.to_str().unwrap());
        cmd.output().map(|o| o.status.success()).unwrap_or(false)
    }

    /// 단일 항목 추출도 대상을 먼저 자르지 않는다(7z)
    #[test]
    fn 단일_항목_추출은_대상을_먼저_자르지_않는다_7z() {
        let td = TempDir::new("entry_file_7z");
        let body = b"0123456789abcdef0123456789abcdef";
        fs::write(td.path.join("a.txt"), body).unwrap();
        mk_fixture(&td.path, &["-t7z", "-mx0"], "a.7z", &["a.txt"]);
        let arc = td.path.join("a.7z");

        // ── 1. 정상 추출 ──
        let out = td.path.join("뽑은것.txt");
        backend()
            .extract_entry_to_file(&arc.to_string_lossy(), "a.txt", &out, None)
            .expect("정상 항목 추출이 실패했다");
        assert_eq!(fs::read(&out).unwrap(), body);

        // ── 2. 손상 아카이브 — 기존 파일 보존 확인 ──
        // 무압축(-mx0) → 원본 바이트 그대로, 1바이트 뒤집어 CRC 파괴
        let mut raw = fs::read(&arc).unwrap();
        let at = raw
            .windows(body.len())
            .position(|w| w == body)
            .expect("무압축 데이터를 찾지 못했다");
        raw[at] ^= 0xFF;
        fs::write(&arc, &raw).unwrap();

        let keep = "기존 내용".as_bytes();
        fs::write(&out, keep).unwrap();
        let r = backend().extract_entry_to_file(&arc.to_string_lossy(), "a.txt", &out, None);
        assert!(r.is_err(), "CRC 가 깨졌는데 성공했다");
        assert_eq!(fs::read(&out).unwrap(), keep, "실패했는데 기존 파일이 잘렸다");

        // ── 3. 임시 파일이 남지 않는다 ──
        let left: Vec<String> = fs::read_dir(&td.path)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".zmtmp-"))
            .collect();
        assert!(left.is_empty(), "임시 파일이 남았다: {left:?}");
    }

    fn make_inputs(dir: &Path) {
        fs::write(dir.join("한글문서.txt"), "안녕하세요 ZipMania").unwrap();
        fs::write(dir.join("빈파일.dat"), b"").unwrap();
        fs::create_dir_all(dir.join("사진").join("2026")).unwrap();
        fs::write(dir.join("사진").join("가을.jpg"), b"jpeg-bytes").unwrap();
        fs::write(dir.join("사진").join("2026").join("겨울.png"), b"png").unwrap();
    }

    fn extract_opts(archive: &Path, dest: &Path, keep: bool, ow: OverwriteMode) -> ExtractOptions {
        ExtractOptions {
            archive: archive.to_str().unwrap().to_string(),
            dest: dest.to_str().unwrap().to_string(),
            keep_paths: keep,
            overwrite: ow,
            password: None,
            selected: vec![],
            decisions: Default::default(),
        }
    }

    fn extract_all(archive: &Path, dest: &Path, keep: bool, ow: OverwriteMode) -> ExtractResult {
        let cancel = Arc::new(AtomicBool::new(false));
        backend().extract(&extract_opts(archive, dest, keep, ow), &mut |_p, _f| {}, cancel)
    }

    // ── (a) DLL 읽기 경로를 7z.exe 산출물로 검증 ──

    #[test]
    fn dll_읽기_7z솔리드_한글_빈파일_보존() {
        let td = TempDir::new("read7z");
        make_inputs(&td.path);
        mk_fixture(&td.path, &["-t7z"], "test.7z", &["한글문서.txt", "빈파일.dat", "사진"]);
        let archive = td.path.join("test.7z");

        let entries = backend().list(archive.to_str().unwrap(), None).expect("목록 실패");
        assert_eq!(entries.len(), 6, "엔트리 수: {entries:?}");
        assert!(entries.iter().any(|e| e.path == "한글문서.txt" && !e.is_dir));
        assert!(entries.iter().any(|e| e.path == "빈파일.dat" && e.size == 0));
        assert!(entries.iter().any(|e| e.path == "사진" && e.is_dir));
        assert!(entries.iter().any(|e| e.path.replace('\\', "/") == "사진/2026/겨울.png"));
    }

    #[test]
    fn dll_읽기_zip_한글_폴더판정() {
        let td = TempDir::new("readzip");
        make_inputs(&td.path);
        mk_fixture(&td.path, &["-tzip"], "test.zip", &["한글문서.txt", "사진"]);
        let archive = td.path.join("test.zip");

        let entries = backend().list(archive.to_str().unwrap(), None).expect("zip 목록 실패");
        assert!(entries.iter().any(|e| e.path == "한글문서.txt" && !e.is_dir));
        assert!(entries.iter().any(|e| e.is_dir));
    }

    #[test]
    fn dll_읽기_헤더암호_암호흐름() {
        let td = TempDir::new("mhe");
        fs::write(td.path.join("비밀.txt"), "top secret").unwrap();
        mk_fixture(&td.path, &["-t7z", "-mhe=on", "-pzipmania비밀"], "enc.7z", &["비밀.txt"]);
        let archive = td.path.join("enc.7z");
        let a = archive.to_str().unwrap();

        // 암호 없이 → password_required
        let e = backend().list(a, None).expect_err("암호없이 열림");
        assert_eq!(e.code, "password_required", "err={e:?}");
        // 틀린 암호 → wrong_password
        let e = backend().list(a, Some("틀린암호")).expect_err("틀린암호로 열림");
        assert_eq!(e.code, "wrong_password", "err={e:?}");
        // 올바른 암호 → 목록 성공
        let entries = backend().list(a, Some("zipmania비밀")).expect("올바른 암호 목록 실패");
        assert!(entries.iter().any(|e| e.path == "비밀.txt"));
    }

    // ── 해제 ──

    #[test]
    fn dll_전체해제_내용_한글_경로_보존() {
        let td = TempDir::new("ex_all");
        make_inputs(&td.path);
        mk_fixture(&td.path, &["-t7z"], "test.7z", &["한글문서.txt", "빈파일.dat", "사진"]);
        let archive = td.path.join("test.7z");
        let out = td.path.join("out");

        match extract_all(&archive, &out, true, OverwriteMode::Overwrite) {
            ExtractResult::Done { status, .. } => assert_eq!(status, "ok"),
            ExtractResult::Failed(e) => panic!("해제 실패: {e:?}"),
        }
        assert_eq!(fs::read_to_string(out.join("한글문서.txt")).unwrap(), "안녕하세요 ZipMania");
        assert_eq!(fs::metadata(out.join("빈파일.dat")).unwrap().len(), 0);
        assert!(out.join("사진").join("가을.jpg").exists());
        assert_eq!(fs::read(out.join("사진").join("2026").join("겨울.png")).unwrap(), b"png");
    }

    #[test]
    fn dll_선택항목_폴더만_재귀_해제() {
        let td = TempDir::new("ex_sel");
        make_inputs(&td.path);
        mk_fixture(&td.path, &["-t7z"], "test.7z", &["한글문서.txt", "빈파일.dat", "사진"]);
        let archive = td.path.join("test.7z");
        let out = td.path.join("out");

        let mut opts = extract_opts(&archive, &out, true, OverwriteMode::Overwrite);
        opts.selected = vec!["사진".to_string()];
        let cancel = Arc::new(AtomicBool::new(false));
        let r = backend().extract(&opts, &mut |_p, _f| {}, cancel);
        assert!(matches!(r, ExtractResult::Done { status: "ok", .. }));

        assert!(out.join("사진").join("가을.jpg").exists());
        assert!(out.join("사진").join("2026").join("겨울.png").exists());
        assert!(!out.join("한글문서.txt").exists());
        assert!(!out.join("빈파일.dat").exists());
    }

    #[test]
    fn dll_덮어쓰기_정책_skip보존_overwrite교체() {
        let td = TempDir::new("ex_pol");
        make_inputs(&td.path);
        mk_fixture(&td.path, &["-t7z"], "test.7z", &["한글문서.txt"]);
        let archive = td.path.join("test.7z");
        let out = td.path.join("out");
        fs::create_dir_all(&out).unwrap();
        fs::write(out.join("한글문서.txt"), "기존내용").unwrap();

        assert!(matches!(
            extract_all(&archive, &out, true, OverwriteMode::Skip),
            ExtractResult::Done { .. }
        ));
        assert_eq!(fs::read_to_string(out.join("한글문서.txt")).unwrap(), "기존내용", "skip인데 덮어써짐");

        assert!(matches!(
            extract_all(&archive, &out, true, OverwriteMode::Overwrite),
            ExtractResult::Done { .. }
        ));
        assert_eq!(fs::read_to_string(out.join("한글문서.txt")).unwrap(), "안녕하세요 ZipMania", "overwrite인데 안바뀜");
    }

    #[test]
    fn dll_암호zip_해제_틀린암호_무암호_분류() {
        let td = TempDir::new("ex_pw");
        fs::write(td.path.join("secret.txt"), "top secret").unwrap();
        mk_fixture(&td.path, &["-tzip", "-pzipmaniaKey1"], "enc.zip", &["secret.txt"]);
        let archive = td.path.join("enc.zip");

        // 틀린 암호
        let mut opts = extract_opts(&archive, &td.path.join("w"), true, OverwriteMode::Overwrite);
        opts.password = Some("틀린암호".to_string());
        match backend().extract(&opts, &mut |_p, _f| {}, Arc::new(AtomicBool::new(false))) {
            ExtractResult::Failed(e) => assert_eq!(e.code, "wrong_password", "err={e:?}"),
            ExtractResult::Done { status, .. } => panic!("틀린암호인데 완료: {status}"),
        }
        // 무암호
        let opts_n = extract_opts(&archive, &td.path.join("n"), true, OverwriteMode::Overwrite);
        match backend().extract(&opts_n, &mut |_p, _f| {}, Arc::new(AtomicBool::new(false))) {
            ExtractResult::Failed(e) => assert_eq!(e.code, "password_required", "err={e:?}"),
            ExtractResult::Done { status, .. } => panic!("무암호인데 완료: {status}"),
        }
        // 올바른 암호
        let mut opts_ok = extract_opts(&archive, &td.path.join("ok"), true, OverwriteMode::Overwrite);
        opts_ok.password = Some("zipmaniaKey1".to_string());
        assert!(matches!(
            backend().extract(&opts_ok, &mut |_p, _f| {}, Arc::new(AtomicBool::new(false))),
            ExtractResult::Done { .. }
        ));
        assert_eq!(fs::read_to_string(td.path.join("ok").join("secret.txt")).unwrap(), "top secret");
    }

    #[test]
    fn dll_충돌검사_기존파일만_반환() {
        let td = TempDir::new("conf");
        make_inputs(&td.path);
        mk_fixture(&td.path, &["-t7z"], "test.7z", &["한글문서.txt", "빈파일.dat", "사진"]);
        let archive = td.path.join("test.7z");
        let out = td.path.join("out");
        fs::create_dir_all(&out).unwrap();
        fs::write(out.join("한글문서.txt"), "기존").unwrap();

        let conflicts = backend()
            .find_conflicts(archive.to_str().unwrap(), out.to_str().unwrap(), true, &[], None)
            .expect("충돌 검사 실패");
        assert_eq!(conflicts, vec!["한글문서.txt".to_string()], "conflicts={conflicts:?}");
    }

    #[test]
    fn dll_대용량_해제_중_취소() {
        let td = TempDir::new("cancel");
        let src = td.path.join("src");
        fs::create_dir_all(&src).unwrap();
        let blob = vec![0u8; 4 * 1024 * 1024];
        for i in 0..40 {
            fs::write(src.join(format!("big_{i}.bin")), &blob).unwrap();
        }
        mk_fixture(&td.path, &["-t7z", "-mx=0"], "big.7z", &["src"]);
        let archive = td.path.join("big.7z");
        let out = td.path.join("out");

        let cancel = Arc::new(AtomicBool::new(false));
        let started = Arc::new(AtomicBool::new(false));
        let cancel_run = Arc::clone(&cancel);
        let started_cb = Arc::clone(&started);
        let a = archive.to_str().unwrap().to_string();
        let o = out.to_str().unwrap().to_string();

        let handle = std::thread::spawn(move || {
            let opts = ExtractOptions {
                archive: a,
                dest: o,
                keep_paths: true,
                overwrite: OverwriteMode::Overwrite,
        decisions: Default::default(),
                password: None,
                selected: vec![],
            };
            backend().extract(&opts, &mut move |_p, _f| {
                started_cb.store(true, Ordering::SeqCst);
            }, cancel_run)
        });

        let deadline = Instant::now() + Duration::from_secs(20);
        while !started.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        cancel.store(true, Ordering::SeqCst);

        match handle.join().expect("해제 스레드 조인 실패") {
            ExtractResult::Done { status, .. } => assert_eq!(status, "canceled"),
            ExtractResult::Failed(e) => panic!("취소가 오류로 처리됨: {e:?}"),
        }
    }

    // ── 생성(DLL create → DLL read 왕복 + 7z.exe 교차검증) ──

    fn create_opts(out: &Path, inputs: &[String], fmt: CompressFormat, level: u8, pw: Option<&str>, enc: bool) -> CreateOptions {
        CreateOptions {
            output: out.to_str().unwrap().to_string(),
            inputs: inputs.to_vec(),
            format: fmt,
            level,
            password: pw.map(|s| s.to_string()),
            encrypt_names: enc,
        }
    }
    fn create(out: &Path, inputs: &[String], fmt: CompressFormat, level: u8, pw: Option<&str>, enc: bool) -> CreateResult {
        let cancel = Arc::new(AtomicBool::new(false));
        backend().create(&create_opts(out, inputs, fmt, level, pw, enc), &mut |_p, _f| {}, cancel)
    }

    #[test]
    fn dll_생성_절대경로입력_basename저장_한글보존_왕복() {
        let td = TempDir::new("cr_7z");
        make_inputs(&td.path);
        let inputs = vec![
            td.path.join("한글문서.txt").to_str().unwrap().to_string(),
            td.path.join("빈파일.dat").to_str().unwrap().to_string(),
            td.path.join("사진").to_str().unwrap().to_string(),
        ];
        let out = td.path.join("out.7z");
        match create(&out, &inputs, CompressFormat::SevenZip, 5, None, false) {
            CreateResult::Done { status, .. } => assert_eq!(status, "ok"),
            CreateResult::Failed(e) => panic!("압축 실패: {e:?}"),
        }
        assert!(out.exists());

        // DLL 로 다시 읽어 왕복 검증
        let entries = backend().list(out.to_str().unwrap(), None).expect("생성물 목록 실패");
        assert!(entries.iter().any(|e| e.path == "한글문서.txt" && !e.is_dir));
        assert!(entries.iter().any(|e| e.path == "빈파일.dat"));
        assert!(entries.iter().any(|e| e.path == "사진" && e.is_dir));
        assert!(entries.iter().any(|e| e.path.replace('\\', "/") == "사진/2026/겨울.png"));
        assert!(!entries.iter().any(|e| e.path.contains(':')));

        // 7z.exe 교차검증
        assert!(exe_test(&out, None), "7z.exe t 가 생성물을 거부");

        // 내용 왕복(메모리 읽기)
        let bytes = backend().read_entry_to_memory(out.to_str().unwrap(), "한글문서.txt", None).unwrap();
        assert_eq!(bytes, "안녕하세요 ZipMania".as_bytes());
    }

    #[test]
    fn dll_생성_zip_왕복_교차검증() {
        let td = TempDir::new("cr_zip");
        make_inputs(&td.path);
        let inputs = vec![td.path.join("한글문서.txt").to_str().unwrap().to_string()];
        let out = td.path.join("out.zip");
        assert!(matches!(
            create(&out, &inputs, CompressFormat::Zip, 5, None, false),
            CreateResult::Done { .. }
        ));
        let entries = backend().list(out.to_str().unwrap(), None).unwrap();
        assert!(entries.iter().any(|e| e.path == "한글문서.txt"));
        assert!(exe_test(&out, None), "7z.exe t 가 zip 생성물을 거부");
    }

    #[test]
    fn dll_압축_레벨_저장보다_최고가_작다() {
        let td = TempDir::new("cr_lv");
        let data = "ZipMania 압축 테스트 ".repeat(20000);
        fs::write(td.path.join("big.txt"), &data).unwrap();
        let input = vec![td.path.join("big.txt").to_str().unwrap().to_string()];
        let store = td.path.join("store.7z");
        let ultra = td.path.join("ultra.7z");
        assert!(matches!(create(&store, &input, CompressFormat::SevenZip, 0, None, false), CreateResult::Done { .. }));
        assert!(matches!(create(&ultra, &input, CompressFormat::SevenZip, 9, None, false), CreateResult::Done { .. }));
        let s0 = fs::metadata(&store).unwrap().len();
        let s9 = fs::metadata(&ultra).unwrap().len();
        assert!(s9 < s0, "최고({s9})가 저장({s0})보다 작아야 함");
    }

    #[test]
    fn dll_압축_암호zip_생성_후_해제_흐름() {
        let td = TempDir::new("cr_pw");
        fs::write(td.path.join("비밀.txt"), "top secret 내용").unwrap();
        let input = vec![td.path.join("비밀.txt").to_str().unwrap().to_string()];
        let out = td.path.join("enc.zip");
        assert!(matches!(
            create(&out, &input, CompressFormat::Zip, 5, Some("zipmaniaKey1"), false),
            CreateResult::Done { .. }
        ));
        assert!(exe_test(&out, Some("zipmaniaKey1")), "7z.exe t -p 거부");

        // 틀린 암호 해제 → wrong_password
        let mut w = extract_opts(&out, &td.path.join("w"), true, OverwriteMode::Overwrite);
        w.password = Some("틀린암호".to_string());
        match backend().extract(&w, &mut |_p, _f| {}, Arc::new(AtomicBool::new(false))) {
            ExtractResult::Failed(e) => assert_eq!(e.code, "wrong_password", "err={e:?}"),
            ExtractResult::Done { status, .. } => panic!("틀린암호인데 완료: {status}"),
        }
        // 올바른 암호 해제 → 내용 보존
        let mut ok = extract_opts(&out, &td.path.join("ok"), true, OverwriteMode::Overwrite);
        ok.password = Some("zipmaniaKey1".to_string());
        assert!(matches!(
            backend().extract(&ok, &mut |_p, _f| {}, Arc::new(AtomicBool::new(false))),
            ExtractResult::Done { .. }
        ));
        assert_eq!(fs::read_to_string(td.path.join("ok").join("비밀.txt")).unwrap(), "top secret 내용");
    }

    #[test]
    fn dll_압축_7z_헤더암호_생성() {
        let td = TempDir::new("cr_mhe");
        fs::write(td.path.join("secret.txt"), "hidden").unwrap();
        let input = vec![td.path.join("secret.txt").to_str().unwrap().to_string()];
        let out = td.path.join("mhe.7z");
        assert!(matches!(
            create(&out, &input, CompressFormat::SevenZip, 5, Some("암호키"), true),
            CreateResult::Done { .. }
        ));
        // 헤더암호 → 무암호 목록은 password_required
        let e = backend().list(out.to_str().unwrap(), None).expect_err("무암호로 목록됨");
        assert_eq!(e.code, "password_required", "err={e:?}");
        // 올바른 암호 목록 성공
        let entries = backend().list(out.to_str().unwrap(), Some("암호키")).expect("올바른 암호 목록 실패");
        assert!(entries.iter().any(|e| e.path == "secret.txt"));
        // 7z.exe 교차검증
        assert!(exe_test(&out, Some("암호키")), "7z.exe t -p 가 헤더암호 생성물 거부");
    }

    #[test]
    fn dll_압축_기존출력_교체() {
        let td = TempDir::new("cr_repl");
        fs::write(td.path.join("a.txt"), "AAA").unwrap();
        fs::write(td.path.join("b.txt"), "BBB").unwrap();
        let out = td.path.join("arc.7z");
        assert!(matches!(
            create(&out, &vec![td.path.join("a.txt").to_str().unwrap().to_string()], CompressFormat::SevenZip, 5, None, false),
            CreateResult::Done { .. }
        ));
        assert!(matches!(
            create(&out, &vec![td.path.join("b.txt").to_str().unwrap().to_string()], CompressFormat::SevenZip, 5, None, false),
            CreateResult::Done { .. }
        ));
        let entries = backend().list(out.to_str().unwrap(), None).unwrap();
        assert!(entries.iter().any(|e| e.path == "b.txt"));
        assert!(!entries.iter().any(|e| e.path == "a.txt"), "교체인데 이전 a.txt 잔존");
    }

    #[test]
    fn dll_압축_입력없으면_오류() {
        let td = TempDir::new("cr_empty");
        let out = td.path.join("x.7z");
        match create(&out, &[], CompressFormat::SevenZip, 5, None, false) {
            CreateResult::Failed(e) => assert_eq!(e.code, "no_input"),
            CreateResult::Done { .. } => panic!("입력이 없는데 완료됨"),
        }
    }

    #[test]
    fn dll_압축_중_취소_산출물_삭제() {
        let td = TempDir::new("cr_cancel");
        let src = td.path.join("src");
        fs::create_dir_all(&src).unwrap();
        let mut blob = vec![0u8; 4 * 1024 * 1024];
        for (i, b) in blob.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        for i in 0..40 {
            fs::write(src.join(format!("big_{i}.bin")), &blob).unwrap();
        }
        let out = td.path.join("big.7z");
        let input = vec![src.to_str().unwrap().to_string()];

        let cancel = Arc::new(AtomicBool::new(false));
        let started = Arc::new(AtomicBool::new(false));
        let cancel_run = Arc::clone(&cancel);
        let started_cb = Arc::clone(&started);
        let out_s = out.to_str().unwrap().to_string();

        let handle = std::thread::spawn(move || {
            let opts = CreateOptions {
                output: out_s,
                inputs: input,
                format: CompressFormat::SevenZip,
                level: 9,
                password: None,
                encrypt_names: false,
            };
            backend().create(&opts, &mut move |_p, _f| {
                started_cb.store(true, Ordering::SeqCst);
            }, cancel_run)
        });

        let deadline = Instant::now() + Duration::from_secs(20);
        while !started.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        cancel.store(true, Ordering::SeqCst);

        match handle.join().expect("압축 스레드 조인 실패") {
            CreateResult::Done { status, .. } => assert_eq!(status, "canceled"),
            CreateResult::Failed(e) => panic!("취소가 오류로 처리됨: {e:?}"),
        }
        assert!(!out.exists(), "취소 후 불완전 아카이브 잔존");
    }

    /// 압축 실패 → 기존 아카이브 보존
    #[test]
    fn 압축_실패해도_기존_파일이_남는다() {
        let td = TempDir::new("cr_keep");
        make_inputs(&td.path);
        let out = td.path.join("기존.7z");
        fs::write(&out, "소중한 기존 아카이브".as_bytes()).unwrap();

        let opts = CreateOptions {
            output: out.to_str().unwrap().to_string(),
            inputs: vec![td.path.join("한글문서.txt").to_str().unwrap().to_string()],
            format: CompressFormat::SevenZip,
            level: 5,
            password: None,
            encrypt_names: false,
        };
        // 없는 DLL → sz.load() 실패
        let broken = SevenZip::new(td.path.join("없는7z.dll"));
        let mut prog = |_p: u8, _f: Option<String>| {};
        match broken.create(&opts, &mut prog, Arc::new(AtomicBool::new(false))) {
            CreateResult::Failed(_) => {}
            CreateResult::Done { status, .. } => panic!("실패해야 하는데 {status}"),
        }
        assert_eq!(
            fs::read(&out).unwrap(),
            "소중한 기존 아카이브".as_bytes(),
            "압축 실패로 기존 파일이 손상되었다"
        );
        // 임시 파일도 남기지 않는다
        let leftovers: Vec<String> = fs::read_dir(&td.path)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.contains(".zmtmp-"))
            .collect();
        assert!(leftovers.is_empty(), "임시 파일 잔존: {leftovers:?}");
    }

    /// 압축 취소 → 기존 아카이브 보존
    #[test]
    fn 압축_취소해도_기존_파일이_남는다() {
        let td = TempDir::new("cr_keep_cancel");
        let src = td.path.join("src");
        fs::create_dir_all(&src).unwrap();
        let mut blob = vec![0u8; 4 * 1024 * 1024];
        for (i, b) in blob.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        for i in 0..40 {
            fs::write(src.join(format!("big_{i}.bin")), &blob).unwrap();
        }
        let out = td.path.join("기존.7z");
        fs::write(&out, "소중한 기존 아카이브".as_bytes()).unwrap();

        let cancel = Arc::new(AtomicBool::new(false));
        let started = Arc::new(AtomicBool::new(false));
        let cancel_run = Arc::clone(&cancel);
        let started_cb = Arc::clone(&started);
        let out_s = out.to_str().unwrap().to_string();
        let input = vec![src.to_str().unwrap().to_string()];

        let handle = std::thread::spawn(move || {
            let opts = CreateOptions {
                output: out_s,
                inputs: input,
                format: CompressFormat::SevenZip,
                level: 9,
                password: None,
                encrypt_names: false,
            };
            backend().create(
                &opts,
                &mut move |_p, _f| {
                    started_cb.store(true, Ordering::SeqCst);
                },
                cancel_run,
            )
        });

        let deadline = Instant::now() + Duration::from_secs(20);
        while !started.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        cancel.store(true, Ordering::SeqCst);
        let _ = handle.join().expect("압축 스레드 조인 실패");

        assert_eq!(
            fs::read(&out).unwrap(),
            "소중한 기존 아카이브".as_bytes(),
            "취소로 기존 파일이 사라졌다"
        );
    }

    /// 해제 취소 → 기존 파일 보존(7z = settle_pending 경로)
    #[test]
    fn 해제_취소해도_기존_파일이_남는다() {
        let td = TempDir::new("ex_keep_cancel");
        let src = td.path.join("src");
        fs::create_dir_all(&src).unwrap();
        // 항목 1개 고정, 여러 개면 취소 시점이 실행마다 달라진다
        fs::write(src.join("big.bin"), vec![b'q'; 64 * 1024 * 1024]).unwrap();
        mk_fixture(&td.path, &["-mx=0"], "a.7z", &["src"]);

        let dest = td.path.join("out");
        let victim = dest.join("src").join("big.bin");
        fs::create_dir_all(victim.parent().unwrap()).unwrap();
        let keep = "취소했으니 그대로여야 한다".as_bytes();
        fs::write(&victim, keep).unwrap();

        // 첫 보고 말고 몇 번 뒤에 취소, 첫 SetCompleted 는 GetStream 보다 선행 가능
        let cancel = Arc::new(AtomicBool::new(false));
        let flip = Arc::clone(&cancel);
        let seen = std::cell::Cell::new(0u32);
        let mut prog = move |_p: u8, _f: Option<String>| {
            seen.set(seen.get() + 1);
            if seen.get() >= 3 {
                flip.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        };
        let r = backend().extract(
            &extract_opts(&td.path.join("a.7z"), &dest, true, OverwriteMode::Overwrite),
            &mut prog,
            cancel,
        );
        match r {
            ExtractResult::Done { status, .. } => assert_eq!(status, "canceled"),
            ExtractResult::Failed(e) => panic!("해제 실패: {}", e.message),
        }
        assert_eq!(
            fs::read(&victim).unwrap(),
            keep,
            "취소가 기존 파일을 날렸다"
        );

        // 임시 파일도 남기지 않는다(마지막 항목은 settle_pending 이 치운다)
        fn leftovers(dir: &Path, out: &mut Vec<String>) {
            let Ok(rd) = fs::read_dir(dir) else { return };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    leftovers(&p, out);
                } else {
                    let n = e.file_name().to_string_lossy().to_string();
                    if n.contains(".zmtmp-") {
                        out.push(n);
                    }
                }
            }
        }
        let mut left = Vec::new();
        leftovers(&dest, &mut left);
        assert!(left.is_empty(), "임시 파일이 남았다: {left:?}");
    }

    /// 압축은 옆에 있는 파일을 건드리지 않는다(.zmtmp-* 도 지우지 않는다)
    #[test]
    fn 압축은_옆_파일을_건드리지_않는다() {
        let td = TempDir::new("tmp_sweep");
        make_inputs(&td.path);
        let out = td.path.join("결과.7z");

        // 임시 파일과 이름 모양이 같은 것들, 전부 보존 필요
        let looks_stale = td.path.join("결과.7z.zmtmp-999999-0");
        let mine = td.path.join(format!("결과.7z.zmtmp-{}-99", std::process::id()));
        let other = td.path.join("다른것.7z.zmtmp-999999-0");
        let from_archive = td.path.join("notes.zmtmp-123-456");
        for p in [&looks_stale, &mine, &other, &from_archive] {
            fs::write(p, b"x").unwrap();
        }

        let opts = CreateOptions {
            output: out.to_str().unwrap().to_string(),
            inputs: vec![td.path.join("한글문서.txt").to_str().unwrap().to_string()],
            format: CompressFormat::SevenZip,
            level: 1,
            password: None,
            encrypt_names: false,
        };
        let mut prog = |_p: u8, _f: Option<String>| {};
        match backend().create(&opts, &mut prog, Arc::new(AtomicBool::new(false))) {
            CreateResult::Done { status, .. } => assert_eq!(status, "ok"),
            CreateResult::Failed(e) => panic!("압축 실패: {}", e.message),
        }

        for p in [&looks_stale, &mine, &other, &from_archive] {
            assert!(p.exists(), "옆에 있던 파일을 지웠다: {}", p.display());
        }
        assert!(out.is_file(), "산출물이 없다");
    }

    /// 압축 입력 재귀는 링크를 따라가지 않는다
    #[test]
    fn 입력_재귀가_링크를_따라가지_않는다() {
        let td = TempDir::new("junction");
        let src = td.path.join("src");
        let outside = td.path.join("outside");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(src.join("안.txt"), "in").unwrap();
        fs::write(outside.join("밖.txt"), "out").unwrap();

        // 밖을 가리키는 정션 + 자기 조상을 가리키는 정션(순환)
        let ok1 = mklink_junction(&src.join("링크"), &outside);
        let ok2 = mklink_junction(&src.join("순환"), &td.path);
        if !ok1 && !ok2 {
            eprintln!("정션을 만들 수 없는 환경 — 건너뜀");
            return;
        }

        let out = td.path.join("결과.7z");
        let opts = CreateOptions {
            output: out.to_str().unwrap().to_string(),
            inputs: vec![src.to_str().unwrap().to_string()],
            format: CompressFormat::SevenZip,
            level: 1,
            password: None,
            encrypt_names: false,
        };
        let mut prog = |_p: u8, _f: Option<String>| {};
        // 링크 추적 시 미종료 또는 외부 파일 유입
        let r = backend().create(&opts, &mut prog, Arc::new(AtomicBool::new(false)));
        match r {
            CreateResult::Done { status, message } => {
                assert_eq!(status, "warning", "건너뛴 링크를 알리지 않는다: {message}");
                assert!(message.contains("링크"), "{message}");
            }
            CreateResult::Failed(e) => panic!("압축 실패: {}", e.message),
        }

        let entries = backend().list(out.to_str().unwrap(), None).expect("목록 실패");
        let names: Vec<String> = entries.iter().map(|e| e.path.replace('\\', "/")).collect();
        assert!(names.iter().any(|n| n.ends_with("안.txt")), "정상 파일이 빠졌다: {names:?}");
        assert!(
            !names.iter().any(|n| n.ends_with("밖.txt")),
            "링크를 따라가 바깥 파일이 들어갔다: {names:?}"
        );
    }

    /// 디렉터리 정션 생성(권한 불필요), 실패 = false
    fn mklink_junction(link: &Path, target: &Path) -> bool {
        std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// 편집도 임시 파일을 제자리로 이동만(원본 선삭제 없음)
    #[test]
    fn 편집_실패해도_원본이_남는다() {
        let td = TempDir::new("ed_keep");
        make_inputs(&td.path);
        mk_fixture(&td.path, &["-t7z"], "test.7z", &["한글문서.txt"]);
        let archive = td.path.join("test.7z");
        let before = fs::read(&archive).unwrap();

        // 추가할 원본 부재 → 편집 실패
        let opts = EditOptions {
            archive: archive.to_str().unwrap().to_string(),
            add: vec![td.path.join("없는파일.txt").to_str().unwrap().to_string()],
            remove: Vec::new(),
            password: None,
        };
        let mut prog = |_p: u8, _f: Option<String>| {};
        let _ = backend().edit(&opts, &mut prog, Arc::new(AtomicBool::new(false)));

        assert_eq!(fs::read(&archive).unwrap(), before, "편집 실패로 원본이 바뀌었다");
    }

    // ── 무결성 테스트 / 메모리 읽기 ──

    #[test]
    fn dll_무결성_테스트_정상_및_손상() {
        let td = TempDir::new("test_ok");
        make_inputs(&td.path);
        mk_fixture(&td.path, &["-t7z"], "test.7z", &["한글문서.txt", "사진"]);
        let archive = td.path.join("test.7z");
        backend().test(archive.to_str().unwrap(), None).expect("정상 아카이브 테스트 실패");

        // 손상: 바이트 일부를 뒤엎는다
        let mut bytes = fs::read(&archive).unwrap();
        let n = bytes.len();
        for b in bytes.iter_mut().skip(n / 2).take(64) {
            *b ^= 0xFF;
        }
        let broken = td.path.join("broken.7z");
        fs::write(&broken, &bytes).unwrap();
        assert!(backend().test(broken.to_str().unwrap(), None).is_err(), "손상인데 통과");
    }

    #[test]
    fn dll_메모리읽기_내부항목_바이트일치() {
        let td = TempDir::new("read_mem");
        let payload = "메모리 읽기 테스트 내용 ZipMania".as_bytes().to_vec();
        fs::write(td.path.join("문서.txt"), &payload).unwrap();
        fs::write(td.path.join("사이드.bin"), b"other").unwrap();
        mk_fixture(&td.path, &["-t7z"], "mem.7z", &["문서.txt", "사이드.bin"]);
        let archive = td.path.join("mem.7z");

        let bytes = backend()
            .read_entry_to_memory(archive.to_str().unwrap(), "문서.txt", None)
            .expect("메모리 읽기 실패");
        assert_eq!(bytes, payload, "읽은 바이트가 원본과 다름");
    }
}


// ─────────────────── 단일 스트림 포맷(gz/bz2/xz) 회귀 테스트 ───────────────────
// 항목 1개 + 이름은 7z 가 아카이브 파일명에서 유도(원본.txt.gz → 원본.txt)

#[cfg(test)]
mod single_stream_tests {
    use super::*;
    use std::fs;
    use std::process::Command;

    fn bin(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("src-tauri")
            .join("binaries")
            .join(name)
    }

    fn extract_to(arc: &Path, dest: &Path) -> ExtractResult {
        let opts = ExtractOptions {
            archive: arc.to_str().unwrap().to_string(),
            dest: dest.to_str().unwrap().to_string(),
            keep_paths: true,
            overwrite: OverwriteMode::Overwrite,
            password: None,
            selected: Vec::new(),
            decisions: Default::default(),
        };
        let mut prog = |_p: u8, _f: Option<String>| {};
        SevenZip::new(bin("7z.dll")).extract(&opts, &mut prog, Arc::new(AtomicBool::new(false)))
    }

    fn case(tag: &str, sw: &str, arc_name: &str) {
        let dir = std::env::temp_dir().join(format!("zipmania_ss_{}_{}", std::process::id(), tag));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("원본.txt"), "단일 스트림 내용").unwrap();

        let o = Command::new(bin("7z.exe"))
            .current_dir(&dir)
            .args(["a", "-y", sw, arc_name, "원본.txt"])
            .output()
            .expect("7z.exe 실행 실패");
        assert!(o.status.success(), "픽스처 생성 실패: {}", String::from_utf8_lossy(&o.stdout));

        let arc = dir.join(arc_name);
        let sz = SevenZip::new(bin("7z.dll"));
        let entries = sz.list(arc.to_str().unwrap(), None).expect("목록 실패");
        eprintln!("[{tag}] 목록 = {entries:?}");
        assert!(!entries.is_empty(), "[{tag}] 목록이 비었다");

        let dest = dir.join("out");
        fs::create_dir_all(&dest).unwrap();
        match extract_to(&arc, &dest) {
            ExtractResult::Done { status, message } => eprintln!("[{tag}] 해제 = {status} / {message}"),
            ExtractResult::Failed(e) => panic!("[{tag}] 해제 실패: {} {}", e.code, e.message),
        }
        let produced: Vec<String> = fs::read_dir(&dest)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        eprintln!("[{tag}] 산출물 = {produced:?}");
        assert!(!produced.is_empty(), "[{tag}] 해제 후 대상 폴더가 비었다");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn dll_gz_목록과_해제() {
        case("gz", "-tgzip", "원본.txt.gz");
    }

    #[test]
    fn dll_bz2_목록과_해제() {
        case("bz2", "-tbzip2", "원본.txt.bz2");
    }

    #[test]
    fn dll_xz_목록과_해제() {
        case("xz", "-txz", "원본.txt.xz");
    }

    #[test]
    fn dll_tgz_는_tar_로_풀린다() {
        // tar 별칭 = 안쪽이 tar, .tar 복원해야 2차 해제 가능
        let dir = std::env::temp_dir().join(format!("zipmania_ss_{}_tgz", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("원본.txt"), "tgz 내용").unwrap();
        let o = Command::new(bin("7z.exe"))
            .current_dir(&dir)
            .args(["a", "-y", "-ttar", "백업.tar", "원본.txt"])
            .output()
            .expect("tar 생성 실패");
        assert!(o.status.success());
        let o = Command::new(bin("7z.exe"))
            .current_dir(&dir)
            .args(["a", "-y", "-tgzip", "백업.tgz", "백업.tar"])
            .output()
            .expect("tgz 생성 실패");
        assert!(o.status.success());
        let _ = fs::remove_file(dir.join("백업.tar"));

        let arc = dir.join("백업.tgz");
        let dest = dir.join("out");
        fs::create_dir_all(&dest).unwrap();
        match extract_to(&arc, &dest) {
            ExtractResult::Done { .. } => {}
            ExtractResult::Failed(e) => panic!("tgz 해제 실패: {} {}", e.code, e.message),
        }
        let produced: Vec<String> = fs::read_dir(&dest)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        eprintln!("[tgz] 산출물 = {produced:?}");
        assert!(produced.iter().any(|f| f.ends_with(".tar")), "tar 로 풀리지 않았다: {produced:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn 유도_이름_규칙() {
        assert_eq!(derived_entry_name(r"D:\원본.txt.bz2"), "원본.txt");
        assert_eq!(derived_entry_name("a.xz"), "a");
        assert_eq!(derived_entry_name("백업.tgz"), "백업.tar");
        assert_eq!(derived_entry_name("백업.tbz2"), "백업.tar");
        // 확장자 부재 시 원본 아카이브 덮어쓰기 방지용 이름 변경
        assert_eq!(derived_entry_name("확장자없음"), "확장자없음.out");
    }
}

// ─────────────────── 충돌 정책(덮어쓰기/건너뛰기/이름변경) 테스트 ───────────────────

#[cfg(test)]
mod conflict_tests {
    use super::*;
    use std::fs;
    use std::process::Command;

    fn bin(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("src-tauri")
            .join("binaries")
            .join(name)
    }

    /// a.txt("새 내용") zip 생성 + 대상 폴더에 a.txt("옛 내용") 배치
    fn setup(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("zipmania_cf_{}_{}", std::process::id(), tag));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.txt"), "새 내용").unwrap();
        let o = Command::new(bin("7z.exe"))
            .current_dir(&dir)
            .args(["a", "-y", "-tzip", "arc.zip", "a.txt"])
            .output()
            .expect("zip 생성 실패");
        assert!(o.status.success());

        let dest = dir.join("out");
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("a.txt"), "옛 내용").unwrap();
        (dir.clone(), dir.join("arc.zip"), dest)
    }

    fn run(arc: &Path, dest: &Path, mode: OverwriteMode, decisions: &[(&str, OverwriteMode)]) {
        let opts = ExtractOptions {
            archive: arc.to_str().unwrap().to_string(),
            dest: dest.to_str().unwrap().to_string(),
            keep_paths: true,
            overwrite: mode,
            password: None,
            selected: Vec::new(),
            decisions: decisions
                .iter()
                .map(|(k, v)| (k.to_string(), *v))
                .collect(),
        };
        let mut prog = |_p: u8, _f: Option<String>| {};
        match SevenZip::new(bin("7z.dll")).extract(&opts, &mut prog, Arc::new(AtomicBool::new(false))) {
            ExtractResult::Done { .. } => {}
            ExtractResult::Failed(e) => panic!("해제 실패: {} {}", e.code, e.message),
        }
    }

    #[test]
    fn 덮어쓰기는_기존_파일을_바꾼다() {
        let (dir, arc, dest) = setup("ow");
        run(&arc, &dest, OverwriteMode::Overwrite, &[]);
        assert_eq!(fs::read_to_string(dest.join("a.txt")).unwrap(), "새 내용");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn 건너뛰기는_기존_파일을_보존한다() {
        let (dir, arc, dest) = setup("skip");
        run(&arc, &dest, OverwriteMode::Skip, &[]);
        assert_eq!(fs::read_to_string(dest.join("a.txt")).unwrap(), "옛 내용");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn 이름변경은_기존을_두고_새_이름으로_저장한다() {
        let (dir, arc, dest) = setup("rename");
        run(&arc, &dest, OverwriteMode::Rename, &[]);
        // 기존 파일 보존 확인
        assert_eq!(fs::read_to_string(dest.join("a.txt")).unwrap(), "옛 내용");
        // 새 파일 = a (2).txt(확장자 보존)
        let renamed = dest.join("a (2).txt");
        assert!(renamed.exists(), "이름 변경 파일이 없다: {:?}", fs::read_dir(&dest).unwrap().flatten().map(|e| e.file_name()).collect::<Vec<_>>());
        assert_eq!(fs::read_to_string(&renamed).unwrap(), "새 내용");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn 파일별_선택이_기본정책보다_우선한다() {
        // 기본 = 덮어쓰기, a.txt 만 건너뛰기 지정 시 기존 파일 보존
        let (dir, arc, dest) = setup("per");
        run(&arc, &dest, OverwriteMode::Overwrite, &[("a.txt", OverwriteMode::Skip)]);
        assert_eq!(fs::read_to_string(dest.join("a.txt")).unwrap(), "옛 내용");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn 유일한_이름_생성_규칙() {
        let dir = std::env::temp_dir().join(format!("zipmania_uq_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // 없는 경로는 그대로
        assert_eq!(crate::formats::unique_path(&dir.join("x.txt")), dir.join("x.txt"));
        fs::write(dir.join("x.txt"), "1").unwrap();
        assert_eq!(crate::formats::unique_path(&dir.join("x.txt")), dir.join("x (2).txt"));
        fs::write(dir.join("x (2).txt"), "2").unwrap();
        assert_eq!(crate::formats::unique_path(&dir.join("x.txt")), dir.join("x (3).txt"));
        // 확장자 없는 파일
        fs::write(dir.join("y"), "1").unwrap();
        assert_eq!(crate::formats::unique_path(&dir.join("y")), dir.join("y (2)"));
        let _ = fs::remove_dir_all(&dir);
    }
}


// ─────────────────── Zip Slip(경로 탈출) 방어 회귀 테스트 ───────────────────
// 악성 이름 = 정상 도구로 못 만듦 → zip 을 바이트로 직접 생성(저장 방식)
// 확인 = 해제 폴더 밖에 아무것도 생기지 않음

#[cfg(test)]
mod zip_slip_tests {
    use super::*;
    use std::fs;

    fn dll() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("src-tauri")
            .join("binaries")
            .join("7z.dll")
    }

    /// 범용 플래그 비트 11 = 이름 UTF-8. 없으면 7z 가 CP949 로 읽는다
    const UTF8_FLAG: u16 = 0x0800;

    /// 저장(무압축) zip 바이트 생성
    fn make_store_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();

        for (name, data) in entries {
            let mut crc = crate::crc32::Crc32::new();
            crc.update(data);
            let crc = crc.finalize();
            let n = name.as_bytes();
            let offset = out.len() as u32;

            // 로컬 파일 헤더
            out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            out.extend_from_slice(&10u16.to_le_bytes()); // 필요 버전
            out.extend_from_slice(&UTF8_FLAG.to_le_bytes()); // 플래그: 이름은 UTF-8
            out.extend_from_slice(&0u16.to_le_bytes()); // 방식: 저장
            out.extend_from_slice(&0u16.to_le_bytes()); // 시각
            out.extend_from_slice(&0u16.to_le_bytes()); // 날짜
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(n.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes()); // 추가 필드 길이
            out.extend_from_slice(n);
            out.extend_from_slice(data);

            // 중앙 디렉터리 항목
            central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes()); // 만든 버전
            central.extend_from_slice(&10u16.to_le_bytes());
            central.extend_from_slice(&UTF8_FLAG.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(n.len() as u16).to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes()); // 추가 필드
            central.extend_from_slice(&0u16.to_le_bytes()); // 주석
            central.extend_from_slice(&0u16.to_le_bytes()); // 디스크 번호
            central.extend_from_slice(&0u16.to_le_bytes()); // 내부 속성
            central.extend_from_slice(&0u32.to_le_bytes()); // 외부 속성
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(n);
        }

        let cd_offset = out.len() as u32;
        let cd_size = central.len() as u32;
        out.extend_from_slice(&central);

        // 중앙 디렉터리 끝 레코드
        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&cd_size.to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    /// 악성 이름 zip 해제 시 out/ 밖 산출물 없음
    #[test]
    fn 대상_폴더_밖으로_쓰지_않는다() {
        let dir = std::env::temp_dir().join(format!("zipmania_slip_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let out = dir.join("out");
        let sentinel = dir.join("탈출.txt"); // out 의 형제 — 여기 생기면 실패
        fs::create_dir_all(&out).unwrap();

        // .. 탈출, 깊은 .., 절대경로, 드라이브 접두사, 그리고 정상 항목 하나
        let evil = make_store_zip(&[
            ("../탈출.txt", b"evil" as &[u8]),
            ("a/b/../../../탈출.txt", b"evil"),
            (r"..\탈출.txt", b"evil"),
            ("/탈출.txt", b"evil"),
            ("C:/탈출.txt", b"evil"),
            (r"C:\탈출.txt", b"evil"),
            ("정상.txt", b"ok"),
        ]);
        let arc = dir.join("evil.zip");
        fs::write(&arc, &evil).unwrap();

        let opts = ExtractOptions {
            archive: arc.to_str().unwrap().to_string(),
            dest: out.to_str().unwrap().to_string(),
            keep_paths: true,
            overwrite: OverwriteMode::Overwrite,
            password: None,
            selected: Vec::new(),
            decisions: Default::default(),
        };
        let mut prog = |_p: u8, _f: Option<String>| {};
        let r = SevenZip::new(dll()).extract(&opts, &mut prog, Arc::new(AtomicBool::new(false)));
        assert!(!sentinel.exists(), "해제 폴더 밖에 파일이 생겼다: {}", sentinel.display());
        assert!(!Path::new(r"C:\탈출.txt").exists(), "드라이브 루트에 파일이 생겼다");
        // 정상 항목은 그대로 해제(방어가 전체를 죽이면 안 됨)
        assert_eq!(
            fs::read_to_string(out.join("정상.txt")).unwrap(),
            "ok",
            "정상 항목이 풀리지 않았다"
        );
        // 절대경로, 드라이브 접두사 = 거부 아닌 상대화, 대상 폴더 안이면 안전
        assert!(out.join("탈출.txt").is_file(), "절대경로 항목이 대상 폴더 안에 풀리지 않았다");
        // .. 항목 3개 건너뜀 → 빠진 항목 있음 → warning
        match r {
            ExtractResult::Done { status, message } => {
                assert_eq!(status, "warning", "빠진 항목이 있는데 ok 로 보고한다: {message}");
                assert!(
                    message.contains("항목 3개"),
                    "건너뛴 항목을 알리지 않는다: {message}"
                );
            }
            ExtractResult::Failed(e) => panic!("해제가 실패했다: {}", e.message),
        }

        let _ = fs::remove_dir_all(&dir);
    }

    /// 경로 무시(평면) 해제도 파일명만 남겨 폴더 밖 이탈 없음
    #[test]
    fn 평면_해제도_안전하다() {
        let dir = std::env::temp_dir().join(format!("zipmania_slipflat_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let out = dir.join("out");
        fs::create_dir_all(&out).unwrap();

        let evil = make_store_zip(&[("../../탈출.txt", b"evil" as &[u8]), ("..", b"evil")]);
        let arc = dir.join("evil.zip");
        fs::write(&arc, &evil).unwrap();

        let opts = ExtractOptions {
            archive: arc.to_str().unwrap().to_string(),
            dest: out.to_str().unwrap().to_string(),
            keep_paths: false,
            overwrite: OverwriteMode::Overwrite,
            password: None,
            selected: Vec::new(),
            decisions: Default::default(),
        };
        let mut prog = |_p: u8, _f: Option<String>| {};
        let _ = SevenZip::new(dll()).extract(&opts, &mut prog, Arc::new(AtomicBool::new(false)));

        assert!(!dir.join("탈출.txt").exists(), "해제 폴더 밖에 파일이 생겼다");
        assert!(out.join("탈출.txt").is_file(), "평면 해제 결과가 없다");

        let _ = fs::remove_dir_all(&dir);
    }
}


// ─────────────────── 압축 폭탄 — 메모리 상한 ───────────────────
// 신고 크기가 아니라 실제 출력 바이트 기준 상한 확인

#[cfg(test)]
mod bomb_tests {
    use super::*;
    use std::fs;
    use std::process::Command;

    fn bin(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("src-tauri")
            .join("binaries")
            .join(name)
    }

    /// 상한 초과 항목을 메모리로 읽으면 오류
    #[test]
    fn 메모리_읽기는_상한을_넘지_않는다() {
        let cap = crate::formats::MAX_MEMORY_ENTRY_BYTES;
        let dir = std::env::temp_dir().join(format!("zipmania_bomb_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // 0 으로 채운 큰 파일 = 극단적 압축률(디스크에는 몇 KB)
        let big = dir.join("bomb.bin");
        let zero = vec![0u8; 8 * 1024 * 1024];
        {
            use std::io::Write;
            let mut f = fs::File::create(&big).unwrap();
            for _ in 0..(cap / (8 * 1024 * 1024) + 4) {
                f.write_all(&zero).unwrap();
            }
        }
        let arc = dir.join("bomb.zip");
        let o = Command::new(bin("7z.exe"))
            .current_dir(&dir)
            .args(["a", "-y", "-tzip", "-mx=9", "bomb.zip", "bomb.bin"])
            .output()
            .expect("7z.exe 실행 실패");
        assert!(o.status.success(), "픽스처 생성 실패");
        let _ = fs::remove_file(&big); // 원본 불필요(용량 회수)

        let sz = SevenZip::new(bin("7z.dll"));
        let r = sz.read_entry_to_memory(arc.to_str().unwrap(), "bomb.bin", None);
        assert!(
            r.is_err(),
            "상한({cap} 바이트)을 넘는데 메모리로 다 읽었다: {:?} 바이트",
            r.map(|v| v.len())
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
