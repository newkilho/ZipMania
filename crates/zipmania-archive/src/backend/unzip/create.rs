//! ZIP 생성, 편집, 산출물 = 임시 파일 생성 → rename(백엔드 공용 crate::outfile)
//! 암호 = ZipCrypto, 이름 = UTF-8(플래그 비트 11), 근거 (D3.13)

use std::collections::HashSet;
use std::fs::File;
use std::io::{BufWriter, Cursor, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::SystemTime;

use zip::unstable::write::FileOptionsExt;
use zip::write::{FileOptions, SimpleFileOptions};
use zip::{CompressionMethod, ZipWriter};

use crate::backend::{CreateOptions, CreateResult, EditOptions, ProgressFn};
use crate::error::ZipManiaError;
use crate::inputs::summarize;
use crate::outfile::reserve_tmp;

use super::{canceled, entry_name, parallel, percent, Archive};

/// 로컬 시간대 오프셋(1회 조회), zip 시각 = 로컬 시간 → UTC 로 적으면 날짜가 시간대만큼 어긋남
/// 조회 불가 환경(멀티스레드 Unix) = UTC
fn local_offset() -> time::UtcOffset {
    static OFF: OnceLock<time::UtcOffset> = OnceLock::new();
    *OFF.get_or_init(|| time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC))
}

/// SystemTime → zip MS-DOS 시각, 표현 불가 시 None
fn dos_datetime(t: SystemTime) -> Option<zip::DateTime> {
    let odt = time::OffsetDateTime::from(t).to_offset(local_offset());
    zip::DateTime::from_date_and_time(
        u16::try_from(odt.year()).ok()?,
        odt.month() as u8,
        odt.day(),
        odt.hour(),
        odt.minute(),
        odt.second(),
    )
    .ok()
}

/// UI 레벨 → 압축 방식 + deflate 레벨, 0 = Store
/// deflate 레벨 0 도 무압축이지만 방식이 Deflate 로 남아 받는 쪽이 해제 코드를 탐(7-Zip -mx0 과 동일)
fn method_and_level(level: u8) -> (CompressionMethod, Option<i64>) {
    if level == 0 {
        (CompressionMethod::Stored, None)
    } else {
        (CompressionMethod::Deflated, Some(level.min(9) as i64))
    }
}

/// 4GiB 이상만 ZIP64. 상시 활성화 금지 — 크레이트가 모든 로컬 헤더 크기를 0xFFFFFFFF 로 적어
/// 7-Zip 이 열지 못함(실측), (D3.13)
const ZIP64_THRESHOLD: u64 = u32::MAX as u64;

/// 암호 제외 쓰기 옵션, 병렬 워커용 → 실패하지 않는 형태
pub(super) fn plain_options(level: u8, mtime: Option<SystemTime>, size: u64) -> SimpleFileOptions {
    let (method, lv) = method_and_level(level);
    let mut opt: SimpleFileOptions = FileOptions::default()
        .compression_method(method)
        .compression_level(lv)
        .large_file(size >= ZIP64_THRESHOLD);
    if let Some(t) = mtime.and_then(dos_datetime) {
        opt = opt.last_modified_time(t);
    }
    opt
}

/// 항목 1개의 쓰기 옵션 생성
fn file_options(
    level: u8,
    password: Option<&str>,
    mtime: Option<SystemTime>,
    size: u64,
) -> Result<SimpleFileOptions, ZipManiaError> {
    let mut opt = plain_options(level, mtime, size);
    // 빈 비밀번호 = 암호 없음, 크레이트는 빈 값 거부 → 새 오류 코드 만들면 프런트에 미번역 코드 증가
    if let Some(pw) = password.filter(|p| !p.is_empty()) {
        opt = opt
            .with_deprecated_encryption(pw.as_bytes())
            .map_err(|e| super::map_err(e, true))?;
    }
    Ok(opt)
}

/// 취소, 오류 중단 시 임시 파일 미잔류
fn abort(tmp: &std::path::Path) {
    let _ = std::fs::remove_file(tmp);
}

/// 호출마다 재할당 금지, 파일당 256KiB → 3000개에서 체감 비용(실측)
const READ_BUF: usize = 256 * 1024;

/// 파일 1개 → writer 스트리밍, 취소는 청크마다 확인, 인자 = 전부 호출측 재사용 상태
#[allow(clippy::too_many_arguments)]
fn stream_file<W: Write + std::io::Seek>(
    zw: &mut ZipWriter<W>,
    source: &std::path::Path,
    on_progress: &mut ProgressFn<'_>,
    name: &str,
    done: &mut u64,
    total: u64,
    cancel: &Arc<AtomicBool>,
    buf: &mut [u8],
) -> Result<bool, ZipManiaError> {
    let mut f = File::open(source)
        .map_err(|e| ZipManiaError::new("io_error", format!("원본을 열지 못했습니다: {e}")))?;
    let mut reported = false;
    loop {
        if canceled(cancel) {
            return Ok(false);
        }
        let n = f
            .read(buf)
            .map_err(|e| ZipManiaError::new("io_error", format!("원본을 읽지 못했습니다: {e}")))?;
        if n == 0 {
            break;
        }
        zw.write_all(&buf[..n])
            .map_err(|e| ZipManiaError::new("output_error", format!("압축 쓰기 실패: {e}")))?;
        *done += n as u64;
        // 이름은 1회만 전송, 청크마다 String 생성 시 수천 번 할당
        if reported {
            on_progress(percent(*done, total), None);
        } else {
            on_progress(percent(*done, total), Some(name.to_string()));
            reported = true;
        }
    }
    Ok(true)
}

/// 압축 생성
pub fn do_create(
    opts: &CreateOptions,
    on_progress: &mut ProgressFn<'_>,
    cancel: Arc<AtomicBool>,
) -> CreateResult {
    if opts.inputs.is_empty() {
        return CreateResult::Failed(ZipManiaError::new("no_input", "압축할 파일이 없습니다."));
    }
    let (items, skipped) = crate::inputs::collect(&opts.inputs);
    if items.is_empty() {
        return CreateResult::Failed(ZipManiaError::new("no_input", "압축할 파일이 없습니다."));
    }

    let out_path = PathBuf::from(&opts.output);
    // 자리를 create_new 로 선점, 이름만 만들고 File::create 하면 그 사이에 놓인 것을 truncate
    // 선점한 빈 파일이므로 아래에서 truncate 안 함
    let mut tmp_path = match reserve_tmp(&out_path) {
        Ok(p) => p,
        Err(e) => return CreateResult::Failed(e),
    };
    let total: u64 = items.iter().map(|i| i.size).sum();

    // 경로 재개방 금지 — 그 사이 경로 삭제나 링크 전환 시 엉뚱한 대상 포착
    let Some(file) = tmp_path.take_file() else {
        return CreateResult::Failed(ZipManiaError::new(
            "output_error",
            "임시 파일 핸들을 얻지 못했습니다.",
        ));
    };
    let mut zw = ZipWriter::new(BufWriter::with_capacity(READ_BUF, file));
    let mut done = 0u64;
    let mut buf = vec![0u8; READ_BUF];

    // 병렬 압축, deflate 는 단일 스트림 병렬화 불가 → 파일 단위 분산(7-Zip 과 동일)
    // 암호 시 사용 금지 — raw_copy_file 이 암호 정보를 못 옮겨 암호화 바이트가 평문으로 표시됨
    let pw = opts.password.as_deref().filter(|p| !p.is_empty());
    let workers = parallel::worker_count();
    let elig: Vec<usize> = if pw.is_some() {
        Vec::new()
    } else {
        parallel::eligible(&items, workers)
    };
    let elig_set: HashSet<usize> = elig.iter().copied().collect();
    let lv = opts.level;
    let mut pipe = if elig.is_empty() {
        None
    } else {
        Some(parallel::Pipeline::start(
            &items,
            &elig,
            workers,
            |it| plain_options(lv, it.mtime, it.size),
            cancel.clone(),
        ))
    };

    /// 중단 시 워커 정지 필수, 미정지 → 예산 대기로 멈춤
    macro_rules! bail {
        ($zw:expr, $pipe:expr, $ret:expr) => {{
            let _ = $zw.finish();
            if let Some(p) = $pipe.take() {
                p.stop();
            }
            abort(&tmp_path);
            return $ret;
        }};
    }
    let canceled_result = || CreateResult::Done {
        status: "canceled",
        message: "사용자가 취소했습니다.".to_string(),
    };

    for (idx, item) in items.iter().enumerate() {
        if canceled(&cancel) {
            bail!(zw, pipe, canceled_result());
        }
        let name = item.name.replace('\\', "/");

        if item.is_dir {
            let opt = plain_options(opts.level, item.mtime, 0);
            if let Err(e) = zw.add_directory(&name, opt) {
                bail!(zw, pipe, CreateResult::Failed(super::map_err(e, false)));
            }
            continue;
        }
        let Some(source) = item.source.as_deref() else {
            continue;
        };

        // 병렬 항목 = 이미 압축됨 → 재압축 없이 이동
        // TooBig 는 실패 아님, 수집 후 파일이 커진 것 → 순차 스트리밍으로 처리하면 결과 동일
        let mut ready: Option<Vec<u8>> = None;
        let mut regrew = false;
        if elig_set.contains(&idx) {
            let Some(p) = pipe.as_ref() else {
                unreachable!("병렬 목록이 있는데 파이프라인이 없다")
            };
            match p.take(idx, &cancel) {
                None => bail!(zw, pipe, canceled_result()),
                Some(Err(e)) => bail!(zw, pipe, CreateResult::Failed(e)),
                Some(Ok(parallel::Piece::Zip(b))) => ready = Some(b),
                Some(Ok(parallel::Piece::TooBig)) => regrew = true,
            }
        }
        if let Some(bytes) = ready {
            let mut one = match zip::ZipArchive::new(Cursor::new(bytes)) {
                Ok(a) => a,
                Err(e) => bail!(zw, pipe, CreateResult::Failed(super::map_err(e, false))),
            };
            let entry = match one.by_index_raw(0) {
                Ok(e) => e,
                Err(e) => bail!(zw, pipe, CreateResult::Failed(super::map_err(e, false))),
            };
            if let Err(e) = zw.raw_copy_file_rename(entry, &name) {
                bail!(zw, pipe, CreateResult::Failed(super::map_err(e, false)));
            }
            done += item.size;
            on_progress(percent(done, total), Some(name));
            continue;
        }

        // 나머지(폴더, 큰 파일, 암호) = 스트리밍 압축, 증가 확인 항목만 크기를 다시
        // 읽는다 — ZIP64 판정이 이 값으로 선다(전부 stat 하지는 않는다, D3.13)
        let size_for_opt = if regrew {
            std::fs::metadata(source)
                .map(|m| m.len())
                .unwrap_or(item.size)
                .max(item.size)
        } else {
            item.size
        };
        let opt = match file_options(opts.level, pw, item.mtime, size_for_opt) {
            Ok(o) => o,
            Err(e) => bail!(zw, pipe, CreateResult::Failed(e)),
        };
        if let Err(e) = zw.start_file(&name, opt) {
            bail!(zw, pipe, CreateResult::Failed(super::map_err(e, false)));
        }
        match stream_file(
            &mut zw,
            source,
            on_progress,
            &name,
            &mut done,
            total,
            &cancel,
            &mut buf,
        ) {
            Ok(true) => {}
            Ok(false) => bail!(zw, pipe, canceled_result()),
            Err(e) => bail!(zw, pipe, CreateResult::Failed(e)),
        }
    }

    if let Some(p) = pipe.take() {
        p.stop();
    }

    match zw.finish() {
        Ok(mut w) => {
            if let Err(e) = w.flush() {
                abort(&tmp_path);
                return CreateResult::Failed(ZipManiaError::new(
                    "output_error",
                    format!("압축을 마무리하지 못했습니다: {e}"),
                ));
            }
        }
        Err(e) => {
            abort(&tmp_path);
            return CreateResult::Failed(super::map_err(e, false));
        }
    }

    // TmpPath::commit 사용, commit_replace 직접 호출 시 committed 미설정 → Drop 이 재삭제 시도
    if let Err(e) = tmp_path.commit() {
        return CreateResult::Failed(e);
    }
    on_progress(100, None);
    finish_message(skipped)
}

/// 아카이브 편집, 기존 항목 = 재압축 없이 복사, 추가분만 새로 압축
pub fn do_edit(
    mut ar: Archive,
    opts: &EditOptions,
    on_progress: &mut ProgressFn<'_>,
    cancel: Arc<AtomicBool>,
) -> CreateResult {
    let arc_path = PathBuf::from(&opts.archive);
    let mut tmp_path = match reserve_tmp(&arc_path) {
        Ok(p) => p,
        Err(e) => return CreateResult::Failed(e),
    };

    // 삭제 대상 = 지정 경로 자신 + 하위 전체
    let removes: Vec<String> = opts.remove.iter().map(|s| s.replace('\\', "/")).collect();
    let is_removed = |name: &str| {
        removes
            .iter()
            .any(|r| name == r || name.starts_with(&format!("{r}/")))
    };

    let (new_items, skipped) = crate::inputs::collect(&opts.add);
    let total: u64 = new_items.iter().map(|i| i.size).sum();

    // 경로 재개방 금지 — 그 사이 경로 삭제나 링크 전환 시 엉뚱한 대상 포착
    let Some(file) = tmp_path.take_file() else {
        return CreateResult::Failed(ZipManiaError::new(
            "output_error",
            "임시 파일 핸들을 얻지 못했습니다.",
        ));
    };
    let mut zw = ZipWriter::new(BufWriter::with_capacity(READ_BUF, file));
    let mut buf = vec![0u8; READ_BUF];

    // 새 이름이 기존 것을 밀어냄(같은 이름 중복 방지)
    let incoming: Vec<String> = new_items.iter().map(|i| i.name.replace('\\', "/")).collect();

    for i in 0..ar.len() {
        if canceled(&cancel) {
            let _ = zw.finish();
            abort(&tmp_path);
            return CreateResult::Done {
                status: "canceled",
                message: "사용자가 취소했습니다.".into(),
            };
        }
        let f = match ar.by_index_raw(i) {
            Ok(f) => f,
            Err(e) => {
                let _ = zw.finish();
                abort(&tmp_path);
                return CreateResult::Failed(super::map_err(e, false));
            }
        };
        let name = entry_name(&f);
        if is_removed(&name) || incoming.contains(&name) {
            continue;
        }
        if let Err(e) = zw.raw_copy_file(f) {
            let _ = zw.finish();
            abort(&tmp_path);
            return CreateResult::Failed(super::map_err(e, false));
        }
    }

    let mut done = 0u64;
    for item in &new_items {
        if canceled(&cancel) {
            let _ = zw.finish();
            abort(&tmp_path);
            return CreateResult::Done {
                status: "canceled",
                message: "사용자가 취소했습니다.".into(),
            };
        }
        let name = item.name.replace('\\', "/");
        // 암호 = 아카이브에 걸린 것 그대로, 평문 삽입 시 한 아카이브에 암호, 평문 혼재
        let opt = match file_options(5, opts.password.as_deref(), item.mtime, item.size) {
            Ok(o) => o,
            Err(e) => {
                let _ = zw.finish();
                abort(&tmp_path);
                return CreateResult::Failed(e);
            }
        };
        if item.is_dir {
            if let Err(e) = zw.add_directory(&name, opt) {
                let _ = zw.finish();
                abort(&tmp_path);
                return CreateResult::Failed(super::map_err(e, false));
            }
            continue;
        }
        if let Err(e) = zw.start_file(&name, opt) {
            let _ = zw.finish();
            abort(&tmp_path);
            return CreateResult::Failed(super::map_err(e, false));
        }
        let Some(source) = item.source.as_deref() else {
            continue;
        };
        match stream_file(&mut zw, source, on_progress, &name, &mut done, total, &cancel, &mut buf) {
            Ok(true) => {}
            Ok(false) => {
                let _ = zw.finish();
                abort(&tmp_path);
                return CreateResult::Done {
                    status: "canceled",
                    message: "사용자가 취소했습니다.".into(),
                };
            }
            Err(e) => {
                let _ = zw.finish();
                abort(&tmp_path);
                return CreateResult::Failed(e);
            }
        }
    }

    match zw.finish() {
        Ok(mut w) => {
            if let Err(e) = w.flush() {
                abort(&tmp_path);
                return CreateResult::Failed(ZipManiaError::new(
                    "output_error",
                    format!("압축을 마무리하지 못했습니다: {e}"),
                ));
            }
        }
        Err(e) => {
            abort(&tmp_path);
            return CreateResult::Failed(super::map_err(e, false));
        }
    }

    // Windows 덮어쓰기 조건: 원본 핸들 해제
    drop(ar);
    if let Err(e) = tmp_path.commit() {
        return CreateResult::Failed(e);
    }
    on_progress(100, None);
    finish_message(skipped)
}

/// 누락분 있으면 warning, 조용히 빠지면 압축했는데 안에 없는 상태
fn finish_message(skipped: Vec<String>) -> CreateResult {
    if skipped.is_empty() {
        CreateResult::Done {
            status: "ok",
            message: "압축을 완료했습니다.".into(),
        }
    } else {
        CreateResult::Done {
            status: "warning",
            message: format!(
                "압축을 마쳤지만 {}개 항목을 담지 못했습니다({}).",
                skipped.len(),
                summarize(&skipped)
            ),
        }
    }
}
