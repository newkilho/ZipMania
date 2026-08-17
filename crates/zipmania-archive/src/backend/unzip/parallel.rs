//! 병렬 압축 파이프라인, 워커 = 항목 1개짜리 zip 을 메모리에 생성, 주 스레드 = raw_copy_file 로
//! 재압축 없이 연결, 측정값, 근거 (D3.13)
//!
//! 필수 4가지
//! 1. MAX_ENTRY 초과 항목은 병렬 제외, 그 이하는 BUDGET 안에서만 동시 실행
//! 2. 암호 걸리면 이 경로 미사용
//! 3. 중단 시 Pipeline::stop 호출 필수
//! 4. 수집 시점 크기 불신 — 읽으며 세어 초과 시 그 항목만 포기(Piece::TooBig)

use std::collections::HashMap;
use std::fs::File;
use std::io::{Cursor, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use crate::error::ZipManiaError;
use crate::inputs::InputItem;

/// 초과 시 주 스레드가 스트리밍 처리
pub const MAX_ENTRY: u64 = 32 * 1024 * 1024;

/// 동시 메모리 적재 가능한 압축 결과 총량
pub const BUDGET: u64 = 128 * 1024 * 1024;

/// 이하면 스레드 비용 > 이득
const MIN_ITEMS: usize = 8;

/// 전 코어 사용 시 UI 스레드 기아
const MAX_WORKERS: usize = 8;

/// 병렬 전송 항목 1개의 입력(워커 스레드로 이동)
#[derive(Clone)]
struct Job {
    name: String,
    source: PathBuf,
    size: u64,
}

/// 잔여 예산 카운터 잠금
struct Budget {
    used: Mutex<u64>,
    cv: Condvar,
}

impl Budget {
    /// n 바이트 확보(취소 시 false), 진행 중 항목 0개면 예산 초과도 통과
    fn acquire(&self, n: u64, stop: &AtomicBool) -> bool {
        let mut used = self.used.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if stop.load(Ordering::Relaxed) {
                return false;
            }
            if *used == 0 || *used + n <= BUDGET {
                *used += n;
                return true;
            }
            let (g, _) = self
                .cv
                .wait_timeout(used, Duration::from_millis(50))
                .unwrap_or_else(|e| e.into_inner());
            used = g;
        }
    }

    fn release(&self, n: u64) {
        let mut used = self.used.lock().unwrap_or_else(|e| e.into_inner());
        *used = used.saturating_sub(n);
        self.cv.notify_all();
    }
}

/// 워커 출력 조각 1개
pub enum Piece {
    /// 항목 1개짜리 zip, 주 스레드가 raw_copy_file 로 재압축 없이 연결
    Zip(Vec<u8>),
    /// 확보 예산 초과, 실패 아님 — 주 스레드 순차 스트리밍으로 처리하면 결과 동일
    TooBig,
}

/// 완성 결과 보관소
struct Done {
    map: Mutex<HashMap<usize, Result<Piece, ZipManiaError>>>,
    cv: Condvar,
}

/// 병렬 압축 파이프라인, 주 스레드가 take 로 순서대로 회수
pub struct Pipeline {
    done: Arc<Done>,
    budget: Arc<Budget>,
    stop: Arc<AtomicBool>,
    alive: Arc<AtomicUsize>,
    handles: Vec<std::thread::JoinHandle<()>>,
    sizes: HashMap<usize, u64>,
}

/// 항목 1개짜리 zip 을 메모리에 생성, Ok(None) = 취소(주 스레드도 같은 플래그 확인)
fn one_entry_zip(
    job: &Job,
    opt: SimpleFileOptions,
    buf: &mut [u8],
    cancel: &AtomicBool,
    stop: &AtomicBool,
) -> Result<Option<Piece>, ZipManiaError> {
    // 압축 후 크기 미상 → 원본의 절반, 재할당 횟수 감소 목적
    let cap = (job.size / 2).min(MAX_ENTRY) as usize + 256;
    let mut zw = ZipWriter::new(Cursor::new(Vec::with_capacity(cap)));
    zw.start_file(&job.name, opt)
        .map_err(|e| super::map_err(e, false))?;
    let mut src = File::open(&job.source)
        .map_err(|e| ZipManiaError::new("io_error", format!("원본을 열지 못했습니다: {e}")))?;
    let mut read_total: u64 = 0;
    loop {
        // 읽기 루프 안에서도 취소 확인, 항목 사이에서만 보면 크게 자란 파일에서 [취소] 가 안 먹음
        if cancel.load(Ordering::Relaxed) || stop.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let n = src
            .read(buf)
            .map_err(|e| ZipManiaError::new("io_error", format!("원본을 읽지 못했습니다: {e}")))?;
        if n == 0 {
            break;
        }
        read_total += n as u64;
        // 예산 초과 시 병렬 포기, 읽기를 끊고 지금까지 것을 내면 내용 잘린 항목이 들어감
        if read_total > job.size {
            return Ok(Some(Piece::TooBig));
        }
        zw.write_all(&buf[..n])
            .map_err(|e| ZipManiaError::new("output_error", format!("압축 쓰기 실패: {e}")))?;
    }
    Ok(Some(Piece::Zip(
        zw.finish()
            .map_err(|e| super::map_err(e, false))?
            .into_inner(),
    )))
}

/// 병렬 전송 인덱스, 제외 = 폴더, MAX_ENTRY 초과, 원본 없음
/// 조건 미달 시 빈 목록 → 호출측이 전부 순차 처리
pub fn eligible(items: &[InputItem], workers: usize) -> Vec<usize> {
    if workers < 2 {
        return Vec::new();
    }
    let v: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, i)| !i.is_dir && i.source.is_some() && i.size <= MAX_ENTRY)
        .map(|(n, _)| n)
        .collect();
    if v.len() < MIN_ITEMS {
        return Vec::new();
    }
    v
}

/// 워커 수, 코어 1개는 주 스레드(쓰기)용으로 예약
pub fn worker_count() -> usize {
    std::thread::available_parallelism()
        .map(|v| v.get().saturating_sub(1))
        .unwrap_or(1)
        .clamp(1, MAX_WORKERS)
}

impl Pipeline {
    /// 워커 기동, opt_for = 항목별 쓰기 옵션(레벨, 시각), 암호 압축에는 사용 금지
    pub fn start(
        items: &[InputItem],
        eligible: &[usize],
        workers: usize,
        opt_for: impl Fn(&InputItem) -> SimpleFileOptions,
        cancel: Arc<AtomicBool>,
    ) -> Pipeline {
        let jobs: Arc<Vec<(usize, Job, SimpleFileOptions)>> = Arc::new(
            eligible
                .iter()
                .map(|&i| {
                    let it = &items[i];
                    (
                        i,
                        Job {
                            name: it.name.replace('\\', "/"),
                            source: it.source.clone().unwrap_or_default(),
                            size: it.size,
                        },
                        opt_for(it),
                    )
                })
                .collect(),
        );
        let sizes = jobs.iter().map(|(i, j, _)| (*i, j.size)).collect();

        let done = Arc::new(Done {
            map: Mutex::new(HashMap::new()),
            cv: Condvar::new(),
        });
        let budget = Arc::new(Budget {
            used: Mutex::new(0),
            cv: Condvar::new(),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let next = Arc::new(AtomicUsize::new(0));
        let alive = Arc::new(AtomicUsize::new(workers));

        let mut handles = Vec::with_capacity(workers);
        for _ in 0..workers {
            let (jobs, done, budget, stop, next, cancel, alive) = (
                jobs.clone(),
                done.clone(),
                budget.clone(),
                stop.clone(),
                next.clone(),
                cancel.clone(),
                alive.clone(),
            );
            handles.push(std::thread::spawn(move || {
                // 패닉 이탈에도 카운터 정합 유지 → Drop 에서 감소
                struct Leave(Arc<AtomicUsize>, Arc<Done>);
                impl Drop for Leave {
                    fn drop(&mut self) {
                        self.0.fetch_sub(1, Ordering::SeqCst);
                        self.1.cv.notify_all();
                    }
                }
                let _leave = Leave(alive, done.clone());
                let mut buf = vec![0u8; 256 * 1024];
                loop {
                    if stop.load(Ordering::Relaxed) || cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let k = next.fetch_add(1, Ordering::Relaxed);
                    let Some((index, job, opt)) = jobs.get(k) else {
                        break;
                    };
                    if !budget.acquire(job.size, &stop) {
                        break;
                    }
                    let out = match one_entry_zip(job, *opt, &mut buf, &cancel, &stop) {
                        // 취소 → 결과 미삽입, 주 스레드는 같은 플래그로 take 에서 None 수신
                        Ok(None) => break,
                        Ok(Some(p)) => Ok(p),
                        Err(e) => Err(e),
                    };
                    // 실패해도 예산 반납은 주 스레드 몫(결과 회수 시점)
                    done.map.lock().unwrap_or_else(|e| e.into_inner()).insert(*index, out);
                    done.cv.notify_all();
                }
            }));
        }

        Pipeline {
            done,
            budget,
            stop,
            handles,
            sizes,
            alive,
        }
    }

    /// index 항목 결과 대기 후 회수(취소 시 None)
    pub fn take(&self, index: usize, cancel: &AtomicBool) -> Option<Result<Piece, ZipManiaError>> {
        let mut map = self.done.map.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(v) = map.remove(&index) {
                drop(map);
                // 이 항목의 예산 반납
                if let Some(n) = self.sizes.get(&index) {
                    self.budget.release(*n);
                }
                return Some(v);
            }
            if cancel.load(Ordering::Relaxed) || self.stop.load(Ordering::Relaxed) {
                return None;
            }
            // 워커 종료 + 결과 없음 = 영영 안 옴(워커 패닉)
            if self.alive.load(Ordering::SeqCst) == 0 {
                return Some(Err(ZipManiaError::new(
                    "engine_error",
                    "압축 작업 스레드가 예기치 않게 끝났습니다.",
                )));
            }
            let (g, _) = self
                .done
                .cv
                .wait_timeout(map, Duration::from_millis(50))
                .unwrap_or_else(|e| e.into_inner());
            map = g;
        }
    }

    /// 취소, 오류 이탈 시 호출 필수, 미호출 시 예산 대기 워커가 남아 멈춤
    pub fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
        self.budget.cv.notify_all();
        self.done.cv.notify_all();
        for h in self.handles {
            let _ = h.join();
        }
    }
}
