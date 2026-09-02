//! 큰 단일 파일의 청크 병렬 deflate, 청크 = 독립 압축 후 이어 붙이기
//! 산출 = 항목 1개짜리 zip 임시 파일, 호출측이 raw_copy_file_rename 으로 최종본 연결 (D3.17)
//!
//! 필수 4가지
//! 1. 청크 = Z_SYNC_FLUSH 마감(바이트 정렬 + 빈 스토어 블록), 전체 끝에 최종 빈 블록 END_BLOCK
//! 2. 암호, Store 레벨, MAX_SIZE 이상은 이 경로 제외
//! 3. 중단 시 job 송신부 드롭 필수, 미드롭 시 워커가 수신 대기로 잔류
//! 4. 수집 시점 크기 불신 — 어긋나면 Outcome::TooBig 로 순차 경로에 양보
//! 5. 기록 대기 청크 수 상한 Slots 필수 — 앞 청크가 늦으면 뒤 결과가 무한히 쌓임

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, sync_channel, Receiver};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use std::time::SystemTime;

use flate2::{Compress, Compression, FlushCompress};

use crate::backend::{Progress, ProgressFn};
use crate::error::ZipManiaError;
use crate::inputs::InputItem;
use crate::outfile::{reserve_tmp, TmpPath};

/// 이하 = parallel 의 파일 단위 분산이 담당
pub const MIN_SIZE: u64 = super::parallel::MAX_ENTRY;

/// ZIP64 미사용 상한, 이상은 순차 스트리밍
pub const MAX_SIZE: u64 = u32::MAX as u64;

/// 청크 크기, 작을수록 병렬 입도가 곱고 압축률 손실 증가
const CHUNK: usize = 1024 * 1024;

/// deflate 최종 빈 블록(BFINAL=1, 고정 허프만, end-of-block), 바이트 정렬 상태 전제
const END_BLOCK: [u8; 2] = [0x03, 0x00];

/// 워커 1개가 동시에 들 수 있는 청크 수, 입력 버퍼 + 압축 결과 + 재정렬 대기 합산
const SLOTS_PER_WORKER: usize = 3;

/// 중간 컨테이너의 항목 이름, 호출측이 raw_copy_file_rename 으로 교체
const STUB: u8 = 0x61;

/// 로컬 헤더(30) + 이름(1)
const LOCAL_LEN: u64 = 31;

/// 아직 기록되지 않은 청크 수의 상한, 읽기 측이 잡고 수집 측이 놓는다
pub(super) struct Slots {
    used: Mutex<usize>,
    cv: Condvar,
    limit: usize,
}

impl Slots {
    pub(super) fn new(limit: usize) -> Slots {
        Slots {
            used: Mutex::new(0),
            cv: Condvar::new(),
            limit,
        }
    }

    /// 자리 1개 확보, 취소나 수집 중단 시 false
    pub(super) fn acquire(&self, cancel: &AtomicBool, live: &AtomicBool) -> bool {
        let mut used = self.used.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if cancel.load(Ordering::Relaxed) || !live.load(Ordering::Relaxed) {
                return false;
            }
            if *used < self.limit {
                *used += 1;
                return true;
            }
            let (g, _) = self
                .cv
                .wait_timeout(used, Duration::from_millis(50))
                .unwrap_or_else(|e| e.into_inner());
            used = g;
        }
    }

    pub(super) fn release(&self) {
        let mut used = self.used.lock().unwrap_or_else(|e| e.into_inner());
        *used = used.saturating_sub(1);
        self.cv.notify_all();
    }
}

/// 청크 병렬 압축 결과, Zip 의 file = 선두로 되감긴 상태
/// TooBig = 수집 시점과 실제 크기 불일치, 실패 아니라 순차 스트리밍으로 넘길 신호
pub enum Outcome {
    Zip { tmp: TmpPath, file: File },
    TooBig,
    Canceled,
}

/// 이 경로 사용 여부
pub fn eligible(item: &InputItem, level: u8, workers: usize) -> bool {
    workers >= 2
        && level > 0
        && !item.is_dir
        && item.source.is_some()
        && item.size > MIN_SIZE
        && item.size < MAX_SIZE
}

/// SystemTime → (MS-DOS time, date), 표현 불가 시 1980-01-01 00:00
fn dos_pair(mtime: Option<SystemTime>) -> (u16, u16) {
    let Some(dt) = mtime.and_then(super::create::dos_datetime) else {
        return (0, 0x0021);
    };
    let t = ((dt.hour() as u16) << 11) | ((dt.minute() as u16) << 5) | (dt.second() as u16 / 2);
    let d = ((dt.year().saturating_sub(1980) as u16) << 9)
        | ((dt.month() as u16) << 5)
        | (dt.day() as u16);
    (t, d)
}

fn io_err(what: &str, e: std::io::Error) -> ZipManiaError {
    ZipManiaError::new("output_error", format!("{what}: {e}"))
}

/// 청크 1개 → 독립 raw deflate, Sync 마감이라 이어 붙이기 가능
/// 출력 여유가 남을 때까지 반복 필수 — DeflateEncoder 의 Write::flush 는 남은 출력을 조용히 자름
pub(super) fn deflate_chunk(input: &[u8], level: u8) -> Result<Vec<u8>, ZipManiaError> {
    const STEP: usize = 64 * 1024;
    let mut c = Compress::new_with_window_bits(Compression::new(level as u32), false, 15);
    let mut out = Vec::with_capacity(input.len() / 2 + STEP);
    let mut flush = FlushCompress::None;
    loop {
        if out.capacity() - out.len() < STEP {
            out.reserve(STEP);
        }
        let spare = out.capacity() - out.len();
        let before = c.total_out();
        let fed = (c.total_in() as usize).min(input.len());
        c.compress_vec(&input[fed..], &mut out, flush)
            .map_err(|e| ZipManiaError::new("output_error", format!("압축 쓰기 실패: {e}")))?;
        if matches!(flush, FlushCompress::None) {
            if c.total_in() as usize >= input.len() {
                flush = FlushCompress::Sync;
            }
        } else if ((c.total_out() - before) as usize) < spare {
            break;
        }
    }
    Ok(out)
}

/// 순서대로 재정렬해 파일에 기록, 반환 = (파일, 압축 바이트 수, 기록한 청크 수)
fn collect(
    mut file: File,
    rx: Receiver<(usize, Result<Vec<u8>, ZipManiaError>)>,
    slots: &Slots,
) -> Result<(File, u64, usize), ZipManiaError> {
    use std::collections::HashMap;
    let mut pending: HashMap<usize, Vec<u8>> = HashMap::new();
    let mut next = 0usize;
    let mut written = 0u64;
    {
        let mut out = std::io::BufWriter::with_capacity(1 << 20, &mut file);
        for (seq, res) in rx {
            pending.insert(seq, res?);
            while let Some(b) = pending.remove(&next) {
                out.write_all(&b).map_err(|e| io_err("압축 쓰기 실패", e))?;
                written += b.len() as u64;
                next += 1;
                slots.release();
            }
        }
        // 순서 구멍 = 중간 워커 이탈, 꼬리 누락은 여기서 안 걸리므로 개수를 따로 대조
        if !pending.is_empty() {
            return Err(ZipManiaError::new(
                "engine_error",
                "압축 작업 스레드가 예기치 않게 끝났습니다.",
            ));
        }
        out.flush().map_err(|e| io_err("압축 쓰기 실패", e))?;
    }
    Ok((file, written, next))
}

/// 중간 컨테이너 마감, 로컬 헤더를 되돌아가 채우고 중앙 디렉토리 + EOCD 추가
fn seal(
    file: &mut File,
    csize: u64,
    raw: u64,
    crc: u32,
    dos: (u16, u16),
) -> Result<(), ZipManiaError> {
    let mut lh = Vec::with_capacity(LOCAL_LEN as usize);
    lh.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
    lh.extend_from_slice(&20u16.to_le_bytes());
    lh.extend_from_slice(&0u16.to_le_bytes());
    lh.extend_from_slice(&8u16.to_le_bytes());
    lh.extend_from_slice(&dos.0.to_le_bytes());
    lh.extend_from_slice(&dos.1.to_le_bytes());
    lh.extend_from_slice(&crc.to_le_bytes());
    lh.extend_from_slice(&(csize as u32).to_le_bytes());
    lh.extend_from_slice(&(raw as u32).to_le_bytes());
    lh.extend_from_slice(&1u16.to_le_bytes());
    lh.extend_from_slice(&0u16.to_le_bytes());
    lh.push(STUB);

    file.seek(SeekFrom::Start(0))
        .map_err(|e| io_err("압축 쓰기 실패", e))?;
    file.write_all(&lh)
        .map_err(|e| io_err("압축 쓰기 실패", e))?;

    let cd_off = LOCAL_LEN + csize;
    file.seek(SeekFrom::Start(cd_off))
        .map_err(|e| io_err("압축 쓰기 실패", e))?;
    let mut cd = Vec::with_capacity(96);
    cd.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
    cd.extend_from_slice(&20u16.to_le_bytes());
    cd.extend_from_slice(&lh[4..30]);
    cd.extend_from_slice(&0u16.to_le_bytes());
    cd.extend_from_slice(&0u16.to_le_bytes());
    cd.extend_from_slice(&0u16.to_le_bytes());
    cd.extend_from_slice(&0u32.to_le_bytes());
    cd.extend_from_slice(&0u32.to_le_bytes());
    cd.push(STUB);
    let cd_len = cd.len() as u32;
    cd.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    cd.extend_from_slice(&0u16.to_le_bytes());
    cd.extend_from_slice(&0u16.to_le_bytes());
    cd.extend_from_slice(&1u16.to_le_bytes());
    cd.extend_from_slice(&1u16.to_le_bytes());
    cd.extend_from_slice(&cd_len.to_le_bytes());
    cd.extend_from_slice(&(cd_off as u32).to_le_bytes());
    cd.extend_from_slice(&0u16.to_le_bytes());
    file.write_all(&cd)
        .map_err(|e| io_err("압축 쓰기 실패", e))?;
    file.set_len(cd_off + cd.len() as u64)
        .map_err(|e| io_err("압축 쓰기 실패", e))?;
    file.seek(SeekFrom::Start(0))
        .map_err(|e| io_err("압축 쓰기 실패", e))?;
    Ok(())
}

/// 파일 1개를 청크 병렬 압축, near = 임시 파일을 놓을 기준 경로(최종 산출물)
#[allow(clippy::too_many_arguments)]
pub fn compress(
    source: &Path,
    size: u64,
    mtime: Option<SystemTime>,
    level: u8,
    near: &Path,
    workers: usize,
    on_progress: &mut ProgressFn<'_>,
    name: &str,
    done: &mut u64,
    total: u64,
    cancel: &Arc<AtomicBool>,
) -> Result<Outcome, ZipManiaError> {
    let mut src = File::open(source)
        .map_err(|e| ZipManiaError::new("io_error", format!("원본을 열지 못했습니다: {e}")))?;
    let mut tmp = reserve_tmp(near)?;
    let Some(mut file) = tmp.take_file() else {
        return Err(ZipManiaError::new(
            "output_error",
            "임시 파일 핸들을 얻지 못했습니다.",
        ));
    };
    // 로컬 헤더 자리 확보, 실제 값은 크기와 CRC 확정 후 seal 이 기록
    file.write_all(&[0u8; LOCAL_LEN as usize])
        .map_err(|e| io_err("압축 쓰기 실패", e))?;

    let (jtx, jrx) = sync_channel::<(usize, Vec<u8>)>(workers);
    let (rtx, rrx) = channel::<(usize, Result<Vec<u8>, ZipManiaError>)>();
    let jrx = Arc::new(Mutex::new(jrx));
    let mut handles = Vec::with_capacity(workers);
    for _ in 0..workers {
        let (jrx, rtx) = (jrx.clone(), rtx.clone());
        handles.push(std::thread::spawn(move || loop {
            let job = jrx.lock().unwrap_or_else(|e| e.into_inner()).recv();
            let Ok((seq, buf)) = job else { break };
            if rtx.send((seq, deflate_chunk(&buf, level))).is_err() {
                break;
            }
        }));
    }
    drop(rtx);
    let slots = Arc::new(Slots::new(workers * SLOTS_PER_WORKER));
    // 수집이 끝나면 자리를 놓아 줄 사람이 없다 — 읽기 측이 그 사실을 보고 빠져나온다
    let live = Arc::new(AtomicBool::new(true));
    let coll = {
        let (slots, live) = (slots.clone(), live.clone());
        std::thread::spawn(move || {
            let r = collect(file, rrx, &slots);
            live.store(false, Ordering::Relaxed);
            slots.cv.notify_all();
            r
        })
    };

    let mut hasher = crate::crc32::Crc32::new();
    let mut read_total = 0u64;
    let mut seq = 0usize;
    let mut outcome: Option<Outcome> = None;
    let mut err: Option<ZipManiaError> = None;
    let mut reported = false;
    loop {
        if super::canceled(cancel) {
            outcome = Some(Outcome::Canceled);
            break;
        }
        let mut buf = vec![0u8; CHUNK];
        let mut filled = 0usize;
        // 부분 읽기 누적, 짧은 청크는 블록 수만 늘어 압축률 손해
        while filled < CHUNK {
            match src.read(&mut buf[filled..]) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(e) => {
                    err = Some(ZipManiaError::new(
                        "io_error",
                        format!("원본을 읽지 못했습니다: {e}"),
                    ));
                    break;
                }
            }
        }
        if err.is_some() || filled == 0 {
            break;
        }
        read_total += filled as u64;
        // 수집 시점보다 커짐 → 순차 경로에 양보, 여기서 끊으면 내용 잘린 항목이 들어감
        if read_total > size {
            outcome = Some(Outcome::TooBig);
            break;
        }
        buf.truncate(filled);
        // 자리부터 확보, 확보 실패는 취소이거나 수집 중단(그 오류는 join 이 올린다)
        if !slots.acquire(cancel, &live) {
            if super::canceled(cancel) {
                outcome = Some(Outcome::Canceled);
            }
            break;
        }
        hasher.update(&buf);
        // 수신부 전멸 = 워커가 전부 이탈, 조용히 끊으면 꼬리 빠진 산출물이 ok 로 나감
        if jtx.send((seq, buf)).is_err() {
            err = Some(ZipManiaError::new(
                "engine_error",
                "압축 작업 스레드가 예기치 않게 끝났습니다.",
            ));
            break;
        }
        seq += 1;
        *done += filled as u64;
        // 이름은 1회만 전송, 청크마다 String 생성 시 수천 번 할당
        if reported {
            on_progress(Progress::new(*done, total, None));
        } else {
            on_progress(Progress::new(*done, total, Some(name.to_string())));
            reported = true;
        }
    }
    // 송신부 드롭 = 워커 종료 신호, 미드롭 시 join 이 영원히 대기
    drop(jtx);
    for h in handles {
        let _ = h.join();
    }
    let joined = coll.join();

    if let Some(e) = err {
        return Err(e);
    }
    if let Some(o) = outcome {
        return Ok(o);
    }
    let Ok(collected) = joined else {
        return Err(ZipManiaError::new(
            "engine_error",
            "압축 작업 스레드가 예기치 않게 끝났습니다.",
        ));
    };
    let (mut file, mut csize, got) = collected?;
    // 보낸 청크와 기록한 청크가 다르면 꼬리가 빠진 것, 크기와 CRC 는 전체 기준이라 티가 안 남
    if got != seq {
        return Err(ZipManiaError::new(
            "engine_error",
            "압축 작업 스레드가 예기치 않게 끝났습니다.",
        ));
    }
    // 신고 크기와 불일치 = 원본이 줄어듦, 순차 경로가 다시 읽어 처리
    if read_total != size {
        return Ok(Outcome::TooBig);
    }
    file.write_all(&END_BLOCK)
        .map_err(|e| io_err("압축 쓰기 실패", e))?;
    csize += END_BLOCK.len() as u64;
    if csize > MAX_SIZE {
        return Ok(Outcome::TooBig);
    }
    seal(&mut file, csize, read_total, hasher.finalize(), dos_pair(mtime))?;
    Ok(Outcome::Zip { tmp, file })
}
