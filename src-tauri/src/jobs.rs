//! 작업 큐, 취소, job_id → Arc<AtomicBool>, 실제 중단은 백엔드가 플래그를 보고, 동시 1작업

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Condvar, Mutex};

use zipmania_archive::ZipManiaError;

/// 실행 중 작업 신원, 취소 플래그 + 종류, 대상 경로 + 소유 세션
#[derive(Clone)]
pub struct JobInfo {
    pub kind: &'static str,
    pub target: String,
    pub owner: String,
}

struct ActiveEntry {
    cancel: std::sync::Arc<AtomicBool>,
    info: JobInfo,
}

/// 매니저 가변 상태, 검사와 등록이 한 자물쇠 안(D3.5)
#[derive(Default)]
struct JobState {
    active: HashMap<String, ActiveEntry>,
    retired: std::collections::HashSet<String>,
    closing: bool,
}

/// 실행 중 작업 관리자, Tauri managed state
pub struct JobManager {
    counter: AtomicU64,
    state: Mutex<JobState>,
    idle: Condvar,
}

impl Default for JobManager {
    fn default() -> Self {
        Self::new()
    }
}

impl JobManager {
    pub fn new() -> Self {
        JobManager {
            counter: AtomicU64::new(0),
            state: Mutex::new(JobState::default()),
            idle: Condvar::new(),
        }
    }

    /// 자물쇠, 중독돼도 그대로 사용
    fn lock(&self) -> std::sync::MutexGuard<'_, JobState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// job_id 발급(job-1)
    pub fn next_id(&self) -> String {
        let n = self.counter.fetch_add(1, Ordering::SeqCst) + 1;
        format!("job-{n}")
    }

    /// 작업 등록 → 취소 플래그 반환, 거절 = 종료 중, 소유 창 물러남, 이미 진행 중
    /// 세 판정과 등록은 한 임계구역(D3.5)
    pub fn start(
        &self,
        id: &str,
        info: JobInfo,
    ) -> Result<std::sync::Arc<AtomicBool>, ZipManiaError> {
        let mut st = self.lock();
        if st.closing {
            return Err(ZipManiaError::new(
                "job_busy",
                "종료 중입니다. 새 작업을 시작할 수 없습니다.",
            ));
        }
        if st.retired.contains(&info.owner) {
            return Err(ZipManiaError::new(
                "window_closed",
                "창이 닫혀 작업을 시작할 수 없습니다.",
            ));
        }
        if !st.active.is_empty() {
            return Err(ZipManiaError::new(
                "job_busy",
                "이미 진행 중인 작업이 있습니다. 완료 후 다시 시도하세요.",
            ));
        }
        let flag = std::sync::Arc::new(AtomicBool::new(false));
        st.active.insert(
            id.to_string(),
            ActiveEntry {
                cancel: std::sync::Arc::clone(&flag),
                info,
            },
        );
        Ok(flag)
    }

    /// 취소 요청(플래그 설정), 없으면 false
    pub fn cancel(&self, id: &str) -> bool {
        let st = self.lock();
        match st.active.get(id) {
            Some(e) => {
                e.cancel.store(true, Ordering::SeqCst);
                true
            }
            None => false,
        }
    }

    /// 창 퇴장, 묘비 + 그 세션 작업 취소(같은 자물쇠), 반환 = 취소 건 개수
    pub fn retire_session(&self, session: &str) -> usize {
        let mut st = self.lock();
        st.retired.insert(session.to_string());
        let mut n = 0;
        for e in st.active.values() {
            if e.info.owner == session {
                e.cancel.store(true, Ordering::SeqCst);
                n += 1;
            }
        }
        n
    }

    /// 그 세션 작업이 다 물러날 때까지 대기(참 = 다 물러남), 압축 창 재오픈용
    pub fn wait_session_retired(&self, session: &str, timeout: std::time::Duration) -> bool {
        let st = self.lock();
        let owned = |m: &mut JobState| m.active.values().any(|e| e.info.owner == session);
        let (st, _res) = self
            .idle
            .wait_timeout_while(st, timeout, owned)
            .unwrap_or_else(|e| e.into_inner());
        // 시간 초과가 아니라 사실 기준 판정
        !st.active.values().any(|e| e.info.owner == session)
    }

    /// 실행 중 작업 신원, 종료 기록 사용 금지 — begin_shutdown 반환값 사용
    #[cfg(test)]
    pub fn active_info(&self) -> Vec<(String, JobInfo)> {
        let st = self.lock();
        st.active
            .iter()
            .map(|(id, e)| (id.clone(), e.info.clone()))
            .collect()
    }

    /// 맵에서 제거(완료, 취소, 오류 후), 다중 호출 안전
    pub fn finish(&self, id: &str) {
        {
            let mut st = self.lock();
            st.active.remove(id);
        }
        // 대기 중인 쪽 깨우기
        self.idle.notify_all();
    }

    /// 종료 절차 시작, 새 작업 차단 + 신원 스냅샷 + 취소를 한 자물쇠 안에서
    /// 반환 = 종료 기록에 쓸 신원 목록(D3.5)
    pub fn begin_shutdown(&self) -> Vec<(String, JobInfo)> {
        let mut st = self.lock();
        st.closing = true;
        let mut snapshot = Vec::with_capacity(st.active.len());
        for (id, e) in st.active.iter() {
            e.cancel.store(true, Ordering::SeqCst);
            snapshot.push((id.clone(), e.info.clone()));
        }
        snapshot
    }

    /// 모든 작업 퇴장 대기(begin_shutdown 뒤), 맵이 비는 것을 보고 시간이 지나면 종료
    /// 반환 = 제때 물러났나, (D3.5)
    pub fn wait_retired(&self, timeout: std::time::Duration) -> bool {
        let st = self.lock();
        if st.active.is_empty() {
            return true;
        }
        let (st, res) = self
            .idle
            .wait_timeout_while(st, timeout, |m: &mut JobState| !m.active.is_empty())
            .unwrap_or_else(|e| e.into_inner());
        // 시간 초과가 아니라 사실 기준 판정, 마지막 작업이 같은 순간에 이탈 가능
        let _ = res;
        st.active.is_empty()
    }

    /// 종료 절차 한 벌(취소 → 대기), 신원이 필요하면 begin_shutdown 을 직접 부를 것
    #[cfg(test)]
    pub fn cancel_all_and_wait(&self, timeout: std::time::Duration) -> bool {
        self.begin_shutdown();
        self.wait_retired(timeout)
    }
}

/// 작업 등록 RAII 보증, 값 소멸 시 등록 해제 보장
/// 정상 경로는 release 를 손으로 부른다(결과 emit 전에)
pub struct JobGuard {
    app: tauri::AppHandle,
    id: String,
}

impl JobGuard {
    pub fn new(app: tauri::AppHandle, id: String) -> Self {
        JobGuard { app, id }
    }

    /// 정상 마감, 결과 통지 전에 호출
    pub fn release(&self) {
        use tauri::Manager;
        self.app.state::<JobManager>().finish(&self.id);
    }
}

impl Drop for JobGuard {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 테스트용 신원
    fn info(owner: &str) -> JobInfo {
        JobInfo {
            kind: "compress",
            target: "C:/out.zip".to_string(),
            owner: owner.to_string(),
        }
    }

    /// JobManager: 동시 1작업 제한 + 취소 플래그 설정 + 완료 후 재시작 가능
    #[test]
    fn job_manager_동시1작업_제한과_취소() {
        let jm = JobManager::new();

        let id1 = jm.next_id();
        let flag1 = jm.start(&id1, info("main")).expect("첫 작업은 시작 가능해야 함");

        // 이미 실행 중이면 두 번째 작업은 job_busy
        let id2 = jm.next_id();
        let err = jm.start(&id2, info("main")).expect_err("동시 두 번째 작업은 거부되어야 함");
        assert_eq!(err.code, "job_busy");

        // 취소 → 플래그 설정 확인
        assert!(!flag1.load(Ordering::SeqCst));
        assert!(jm.cancel(&id1));
        assert!(flag1.load(Ordering::SeqCst));

        // 없는 작업 취소는 false
        assert!(!jm.cancel("job-없음"));

        // 완료 처리 후에는 새 작업 시작 가능
        jm.finish(&id1);
        let id3 = jm.next_id();
        assert!(jm.start(&id3, info("main")).is_ok());
    }

    /// 창 닫힘 시 그 창이 시작한 작업만 취소(다른 창의 작업 미간섭)
    #[test]
    fn 창이_닫히면_그_창의_작업만_취소한다() {
        let jm = JobManager::new();
        let id = jm.next_id();
        let flag = jm.start(&id, info("compress")).expect("작업 시작");

        // 다른 창이 닫힌 것으로는 취소되지 않는다
        assert_eq!(jm.retire_session("extract"), 0);
        assert!(!flag.load(Ordering::SeqCst), "남의 창이 닫혔는데 취소됐다");

        assert_eq!(jm.retire_session("compress"), 1);
        assert!(flag.load(Ordering::SeqCst), "제 창이 닫혔는데 취소되지 않았다");

        // 물러난 창의 지연 IPC 는 새 작업 시작 불가
        jm.finish(&id);
        let late = jm.next_id();
        let err = jm
            .start(&late, info("compress"))
            .expect_err("죽은 창의 지연 호출이 작업을 시작했다");
        assert_eq!(err.code, "window_closed");
        // 살아 있는 창은 그대로 시작 가능(extract 도 위에서 퇴장)
        assert!(jm.start(&late, info("main")).is_ok());

        // 종료 기록에 쓸 신원도 함께 얻는다(임시 파일이 생기는 자리)
        let info = jm.active_info();
        assert_eq!(info.len(), 1);
        assert_eq!(info[0].1.target, "C:/out.zip");
        assert_eq!(info[0].1.kind, "compress");
    }

    /// 종료 기록용 신원과 취소가 한 호출
    #[test]
    fn 종료_신원과_취소는_한_호출이다() {
        let jm = JobManager::new();
        let id = jm.next_id();
        let flag = jm.start(&id, info("compress")).expect("작업 시작");

        let snapshot = jm.begin_shutdown();
        assert_eq!(snapshot.len(), 1, "기록에 쓸 신원이 빠졌다");
        assert_eq!(snapshot[0].0, id);
        assert_eq!(snapshot[0].1.target, "C:/out.zip");
        assert!(flag.load(Ordering::SeqCst), "취소가 걸리지 않았다");

        // 그 뒤로는 새 작업을 받지 않는다(받으면 아무도 기다려 주지 않는다)
        jm.finish(&id);
        let err = jm
            .start(&jm.next_id(), info("main"))
            .expect_err("종료 중에 작업이 시작됐다");
        assert_eq!(err.code, "job_busy");
    }

    /// 종료 시 취소 + 퇴장 대기, 물러나지 않으면 시간이 지나 그냥 반환
    #[test]
    fn 종료할_때_작업을_취소하고_기다린다() {
        use std::time::Duration;

        // 아무것도 없으면 즉시 성공, 그 뒤로는 새 작업을 받지 않는다
        let empty = JobManager::new();
        assert!(empty.cancel_all_and_wait(Duration::from_millis(50)));
        let after = empty.next_id();
        assert_eq!(
            empty.start(&after, info("main")).expect_err("종료 중에는 거부").code,
            "job_busy"
        );

        // 워커의 취소 수용 후 퇴장 → 대기 성공 종료
        let jm = std::sync::Arc::new(JobManager::new());
        let id = jm.next_id();
        let flag = jm.start(&id, info("main")).expect("작업 시작");
        let worker = {
            let jm = std::sync::Arc::clone(&jm);
            let id = id.clone();
            std::thread::spawn(move || {
                while !flag.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(5));
                }
                jm.finish(&id);
            })
        };
        assert!(jm.cancel_all_and_wait(Duration::from_secs(5)));
        worker.join().expect("워커 종료");

        // 물러나지 않는 워커: 시간이 지나면 false 로 돌아오고 막히지 않는다
        let stuck = JobManager::new();
        let id2 = stuck.next_id();
        let _flag2 = stuck.start(&id2, info("main")).expect("작업 시작");
        assert!(!stuck.cancel_all_and_wait(Duration::from_millis(100)));
    }
}
