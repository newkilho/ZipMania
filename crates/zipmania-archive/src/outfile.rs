//! 산출물 마감 공용, 임시 파일 생성 → rename, 압축 = TmpPath, 해제 = StagedFile
//! 기존 파일 선삭제 금지, 선truncate 금지, 백엔드별 사본 금지 (D3.5)

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::ZipManiaError;
use crate::volumes::volume_name;

/// 임시 이름용 128비트 난수(32자리 16진수), CSPRNG 아님 → 삭제 판단 근거 금지 (D3.5)
fn random_token() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);

    let mut halves = [0u64; 2];
    for (i, h) in halves.iter_mut().enumerate() {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(n);
        hasher.write_u64(nanos);
        hasher.write_u32(std::process::id());
        hasher.write_usize(i);
        // 스택 주소도 혼합(ASLR)
        hasher.write_usize(&hasher as *const _ as usize);
        *h = hasher.finish();
    }
    format!("{:016x}{:016x}", halves[0], halves[1])
}

// 진단 전용, 이 목록으로 삭제 금지, 산출물 경로만으론 위치 특정 불가 → 정확한 경로 필요
static LIVE_TMP: std::sync::Mutex<Option<std::collections::BTreeSet<PathBuf>>> =
    std::sync::Mutex::new(None);

fn live_insert(p: &Path) {
    let mut g = LIVE_TMP.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(Default::default).insert(p.to_path_buf());
}

fn live_remove(p: &Path) {
    let mut g = LIVE_TMP.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = g.as_mut() {
        s.remove(p);
    }
}

/// 현재 살아 있는 임시 경로, 종료 기록용, 자동 삭제 없음
pub fn live_temp_paths() -> Vec<PathBuf> {
    let g = LIVE_TMP.lock().unwrap_or_else(|e| e.into_inner());
    g.as_ref().map(|s| s.iter().cloned().collect()).unwrap_or_default()
}

/// 임시 경로 이름 = <이름>.zmtmp-<128비트 난수>
fn tmp_name(target: &Path) -> PathBuf {
    let name = target
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    target.with_file_name(format!("{name}.zmtmp-{}", random_token()))
}

/// 임시 파일 독점 생성(create_new) → 핸들과 경로, 경로만 넘기는 재오픈 금지
fn open_new_tmp(target: &Path) -> Result<(std::fs::File, PathBuf), ZipManiaError> {
    for _ in 0..8 {
        let tmp = tmp_name(target);
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&tmp)
        {
            Ok(f) => {
                live_insert(&tmp);
                return Ok((f, tmp));
            }
            // 128비트 충돌 = 난수 아니라 버그, 그래도 몇 번 재시도
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(ZipManiaError::new(
                    "output_error",
                    format!("임시 파일을 만들지 못했습니다: {e}"),
                ))
            }
        }
    }
    Err(ZipManiaError::new(
        "output_error",
        "임시 파일 이름을 얻지 못했습니다.",
    ))
}

/// 잡아 둔 임시 자리, 핸들 보유, Drop 시 자동 정리
pub struct TmpPath {
    path: PathBuf,
    target: PathBuf,
    file: Option<std::fs::File>,
    committed: bool,
}

impl TmpPath {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 생성 성공한 핸들(1회 회수), 경로 재오픈 금지
    pub fn take_file(&mut self) -> Option<std::fs::File> {
        self.file.take()
    }

    /// 완성 → 제자리 이동, rename 성공 후 소유 해제 (D3.5)
    pub fn commit(mut self) -> Result<(), ZipManiaError> {
        // Windows = 열린 파일 rename 불가 → 보유 중이면 여기서 닫음
        self.file = None;
        commit_replace(&self.path, &self.target)?;
        self.committed = true;
        live_remove(&self.path);
        Ok(())
    }
}

impl Drop for TmpPath {
    fn drop(&mut self) {
        if !self.committed {
            self.file = None;
            let _ = std::fs::remove_file(&self.path);
        }
        live_remove(&self.path);
    }
}

impl AsRef<Path> for TmpPath {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl std::ops::Deref for TmpPath {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.path
    }
}

/// 산출물 옆에 임시 파일 독점 생성, rename 은 볼륨 넘지 못함(%TEMP% 는 다른 드라이브)
pub fn reserve_tmp(target: &Path) -> Result<TmpPath, ZipManiaError> {
    let (file, path) = open_new_tmp(target)?;
    Ok(TmpPath {
        path,
        target: target.to_path_buf(),
        file: Some(file),
        committed: false,
    })
}

// 남은 임시 파일 자동 정리 금지, 사고 4회, 전부 남의 파일 삭제
// 재도입 조건 = CSPRNG, 저널 무결성, 파일 ID 대조 동시 충족 (D3.5)

/// 임시 파일 → 제자리 이동, 선삭제 금지(Windows rename 은 덮어씀)
pub fn commit_replace(tmp: &Path, target: &Path) -> Result<(), ZipManiaError> {
    match std::fs::rename(tmp, target) {
        Ok(()) => Ok(()),
        // 여기서 삭제 금지, 소유자 Drop 몫, 두 곳에서 지우면 한쪽 실패가 묻힘
        Err(e) => Err(ZipManiaError::new(
            "output_error",
            format!("산출물을 제자리에 놓지 못했습니다(기존 파일은 그대로입니다): {e}"),
        )),
    }
}

/// 해제 대상 파일 하나, 대상 유무와 무관하게 항상 임시 파일 → rename
/// 비용 = 항목당 rename 1회 (D3.5)
pub struct StagedFile {
    path: PathBuf,
    target: PathBuf,
    settled: bool,
}

impl StagedFile {
    /// 임시 파일 독점 생성(create_new) → 오픈, 핸들 = 확실히 우리가 만든 파일
    pub fn create(target: &Path) -> std::io::Result<(std::fs::File, Self)> {
        match open_new_tmp(target) {
            Ok((f, tmp)) => Ok((
                f,
                Self {
                    path: tmp,
                    target: target.to_path_buf(),
                    settled: false,
                },
            )),
            Err(e) => Err(std::io::Error::other(e.message)),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 성공 → 제자리 이동, 호출 전 파일 핸들 닫기 필수(Windows)
    pub fn commit(mut self) -> Result<(), ZipManiaError> {
        let r = commit_replace(&self.path, &self.target);
        // rename 실패 = 마감 아님, Drop 이 임시 파일 정리, 기존 파일 유지
        self.settled = r.is_ok();
        if self.settled {
            live_remove(&self.path);
        }
        r
    }

    /// 실패나 취소 → 쓰던 것만 삭제, 삭제 실패 = 마감 아님(Drop 이 재시도)
    pub fn abort(mut self) {
        self.settled = std::fs::remove_file(&self.path).is_ok();
    }
}

impl Drop for StagedFile {
    fn drop(&mut self) {
        if !self.settled {
            let _ = std::fs::remove_file(&self.path);
        }
        live_remove(&self.path);
    }
}

/// 분할 산출 writer(Write + Seek), 볼륨마다 임시 파일, commit 에서 일괄 이동
/// 볼륨 1개로 끝나면 대상 이름 그대로(.001 하나짜리 세트 금지), 7z, zip 공용 (D3.18)
pub struct VolumeSet {
    target: PathBuf,
    size: u64,
    vols: Vec<(TmpPath, std::fs::File)>,
    pos: u64,
    last_err: Option<String>,
}

impl VolumeSet {
    /// size = 볼륨 바이트 수, 0 금지
    pub fn new(target: &Path, size: u64) -> Result<Self, ZipManiaError> {
        if size == 0 {
            return Err(ZipManiaError::new("invalid_volume", "분할 크기가 0 입니다."));
        }
        Ok(Self {
            target: target.to_path_buf(),
            size,
            vols: Vec::new(),
            pos: 0,
            last_err: None,
        })
    }

    /// 마지막 쓰기, 확장 오류(1회 회수), COM 스트림은 HRESULT 만 돌려주므로 원문은 여기서
    pub fn take_error(&mut self) -> Option<String> {
        self.last_err.take()
    }

    fn fail<T>(&mut self, e: std::io::Error) -> std::io::Result<T> {
        self.last_err = Some(e.to_string());
        Err(e)
    }

    pub fn volume_count(&self) -> usize {
        self.vols.len()
    }

    /// 전체 길이 = 마지막 볼륨 이전은 size, 마지막은 실제 파일 길이
    pub fn len(&self) -> u64 {
        match self.vols.last() {
            None => 0,
            Some((_, f)) => {
                let last = f.metadata().map(|m| m.len()).unwrap_or(0);
                (self.vols.len() as u64 - 1) * self.size + last
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.vols.is_empty()
    }

    /// idx(0부터) 볼륨까지 확보, 앞 볼륨은 size 로 채움(건너뛴 seek 뒤 쓰기)
    fn ensure(&mut self, idx: usize) -> std::io::Result<()> {
        while self.vols.len() <= idx {
            let n = self.vols.len() + 1;
            let mut tmp = reserve_tmp(&volume_name(&self.target, n))
                .map_err(|e| std::io::Error::other(e.message))?;
            let f = tmp
                .take_file()
                .ok_or_else(|| std::io::Error::other("임시 파일 핸들 없음"))?;
            if let Some((_, prev)) = self.vols.last() {
                prev.set_len(self.size)?;
            }
            self.vols.push((tmp, f));
        }
        Ok(())
    }

    /// 길이 변경, 줄이면 뒤 볼륨 삭제(Drop), 늘리면 볼륨 추가
    pub fn set_len(&mut self, new_len: u64) -> std::io::Result<()> {
        if new_len == 0 {
            self.vols.clear();
            return Ok(());
        }
        let count = new_len.div_ceil(self.size) as usize;
        let last_len = new_len - (count as u64 - 1) * self.size;
        self.ensure(count - 1)?;
        self.vols.truncate(count);
        self.vols[count - 1].1.set_len(last_len)
    }

    /// 완성 → 볼륨 전부 제자리 이동, 하나라도 실패하면 옮긴 것을 되돌림, 반환 = 최종 경로들
    /// 옛 세트의 다음 번호가 남아 있으면 이동 전에 거부(섞이면 Split 핸들러가 이어 붙임)
    pub fn commit(mut self) -> Result<Vec<PathBuf>, ZipManiaError> {
        let n = self.vols.len();
        if n == 0 {
            return Err(ZipManiaError::new("output_error", "산출물이 비었습니다."));
        }
        let finals: Vec<PathBuf> = if n == 1 {
            vec![self.target.clone()]
        } else {
            (1..=n).map(|i| volume_name(&self.target, i)).collect()
        };
        let stale = if n == 1 {
            volume_name(&self.target, 1)
        } else {
            volume_name(&self.target, n + 1)
        };
        if stale.exists() {
            return Err(ZipManiaError::new(
                "stale_volume",
                format!("옛 분할 파일이 남아 있습니다: {}", stale.display()),
            ));
        }
        // Windows = 열린 파일 rename 불가
        let mut vols: Vec<TmpPath> = self.vols.drain(..).map(|(t, _)| t).collect();
        let mut moved = 0usize;
        let mut err = None;
        for (t, dst) in vols.iter().zip(finals.iter()) {
            if let Err(e) = commit_replace(&t.path, dst) {
                err = Some(e);
                break;
            }
            moved += 1;
        }
        match err {
            None => {
                for t in vols.iter_mut() {
                    t.committed = true;
                    live_remove(&t.path);
                }
                Ok(finals)
            }
            Some(e) => {
                // 되돌리기 실패분은 최종 이름으로 남음(불완전 세트), Drop 정리 대상 아님
                for (t, dst) in vols.iter_mut().zip(finals.iter()).take(moved) {
                    if std::fs::rename(dst, &t.path).is_err() {
                        t.committed = true;
                        live_remove(&t.path);
                    }
                }
                Err(e)
            }
        }
    }

    fn set_pos(&mut self, p: u64) -> u64 {
        self.pos = p;
        p
    }
}

impl std::io::Write for VolumeSet {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        use std::io::{Seek, SeekFrom};
        if buf.is_empty() {
            return Ok(0);
        }
        let idx = (self.pos / self.size) as usize;
        let off = self.pos % self.size;
        let room = (self.size - off) as usize;
        let n = buf.len().min(room);
        if let Err(e) = self.ensure(idx) {
            return self.fail(e);
        }
        let f = &mut self.vols[idx].1;
        if let Err(e) = f.seek(SeekFrom::Start(off)).and_then(|_| f.write_all(&buf[..n])) {
            return self.fail(e);
        }
        self.pos += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        for (_, f) in self.vols.iter_mut() {
            f.flush()?;
        }
        Ok(())
    }
}

impl std::io::Seek for VolumeSet {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        use std::io::SeekFrom;
        let (base, delta) = match pos {
            SeekFrom::Start(p) => return Ok(self.set_pos(p)),
            SeekFrom::Current(d) => (self.pos as i64, d),
            SeekFrom::End(d) => (self.len() as i64, d),
        };
        let np = base
            .checked_add(delta)
            .filter(|v| *v >= 0)
            .ok_or_else(|| std::io::Error::other("탐색 위치 범위 밖"))?;
        Ok(self.set_pos(np as u64))
    }
}

/// 생성 산출 writer(Write + Seek), 단일 임시 파일 또는 분할, 마감은 호출측이 variant 별로
pub enum OutSink {
    File(std::fs::File),
    Volumes(VolumeSet),
}

impl std::io::Write for OutSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            OutSink::File(f) => f.write(buf),
            OutSink::Volumes(v) => v.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            OutSink::File(f) => f.flush(),
            OutSink::Volumes(v) => v.flush(),
        }
    }
}

impl std::io::Seek for OutSink {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        match self {
            OutSink::File(f) => f.seek(pos),
            OutSink::Volumes(v) => v.seek(pos),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{Read, Seek, SeekFrom, Write};
    use crate::volumes::volume_name;

    /// 분할 writer 계약, 경계 넘는 쓰기, 되감기 패치, 줄이기, 1개면 대상 이름
    #[test]
    fn 분할_볼륨_쓰기와_마감() {
        let root = std::env::temp_dir().join(format!("zm_vol_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let target = root.join("결과.7z");

        // 1. 10바이트 볼륨에 25바이트 → 3볼륨, 경계에서 잘림
        let mut v = super::VolumeSet::new(&target, 10).unwrap();
        let data: Vec<u8> = (0u8..25).collect();
        v.write_all(&data).unwrap();
        assert_eq!(v.volume_count(), 3);
        assert_eq!(v.len(), 25);

        // 2. 되감아 첫 볼륨 패치(7z 시작 헤더), 위치는 다시 끝으로
        v.seek(SeekFrom::Start(3)).unwrap();
        v.write_all(&[0xAA, 0xBB]).unwrap();
        assert_eq!(v.seek(SeekFrom::End(0)).unwrap(), 25);

        // 3. 줄이면 뒤 볼륨이 사라진다
        v.set_len(12).unwrap();
        assert_eq!(v.volume_count(), 2);
        assert_eq!(v.len(), 12);
        v.set_len(25).unwrap();
        assert_eq!(v.volume_count(), 3);

        // 4. 마감 → .001 .002 .003, 임시 파일 없음
        let finals = v.commit().unwrap();
        assert_eq!(finals.len(), 3);
        assert!(finals[0].ends_with("결과.7z.001"));
        let mut all = Vec::new();
        for p in &finals {
            fs::File::open(p).unwrap().read_to_end(&mut all).unwrap();
        }
        let mut want = data.clone();
        want[3] = 0xAA;
        want[4] = 0xBB;
        // 12 로 줄였다 25 로 늘린 구간은 0
        for b in want.iter_mut().skip(12) {
            *b = 0;
        }
        assert_eq!(all, want);
        assert!(
            fs::read_dir(&root)
                .unwrap()
                .all(|e| !e.unwrap().file_name().to_string_lossy().contains(".zmtmp-")),
            "임시 파일이 남았다"
        );

        // 5. 옛 세트의 다음 번호가 남아 있으면 거부, 아무것도 옮기지 않는다
        let target2 = root.join("둘.7z");
        fs::write(volume_name(&target2, 3), b"x").unwrap();
        let mut v = super::VolumeSet::new(&target2, 10).unwrap();
        v.write_all(&data[..15]).unwrap();
        let e = v.commit().unwrap_err();
        assert_eq!(e.code, "stale_volume");
        assert!(!volume_name(&target2, 1).exists());

        // 6. 볼륨 1개면 .001 없이 대상 이름
        let target3 = root.join("하나.7z");
        let mut v = super::VolumeSet::new(&target3, 100).unwrap();
        v.write_all(&data).unwrap();
        let finals = v.commit().unwrap();
        assert_eq!(finals, vec![target3.clone()]);
        assert!(target3.exists());

        // 7. 놓으면 볼륨 전부 정리
        let target4 = root.join("버림.7z");
        let mut v = super::VolumeSet::new(&target4, 10).unwrap();
        v.write_all(&data).unwrap();
        drop(v);
        assert!(!volume_name(&target4, 1).exists());

        let _ = fs::remove_dir_all(&root);
    }

    /// 임시 파일 소유권 계약, 정상 경로 = 반드시 정리, 이름 = 예측 불가
    #[test]
    fn 임시파일_소유권() {
        let root = std::env::temp_dir().join(format!("zm_own_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let target = root.join("결과.zip");

        // 1. 이름 = 128비트 난수, 겹침 없음
        let mut seen = std::collections::HashSet::new();
        for _ in 0..32 {
            let t = super::reserve_tmp(&target).unwrap();
            let name = t.path().file_name().unwrap().to_string_lossy().to_string();
            let (_, tok) = name.rsplit_once(".zmtmp-").expect("이름 규칙이 다르다");
            assert_eq!(tok.len(), 32, "128비트가 아니다: {tok}");
            assert!(tok.bytes().all(|b| b.is_ascii_hexdigit()), "16진수가 아니다");
            assert!(seen.insert(tok.to_string()), "임시 이름이 겹쳤다");
        }

        // 2. 만든 핸들 그대로 전달, 경로 재오픈 없음
        let mut t = super::reserve_tmp(&target).unwrap();
        assert!(t.take_file().is_some(), "만든 핸들을 주지 않는다");
        assert!(t.take_file().is_none(), "핸들을 두 번 내줬다");

        // 3. commit 없이 놓으면 자동 정리
        let left = t.path().to_path_buf();
        drop(t);
        assert!(!left.exists(), "놓았는데 임시 파일이 남았다");

        // 4. commit → 제자리 이동, 이후 Drop 의 재삭제 없음
        // commit_replace 직접 호출 시 committed 미설정 → Drop 이 재삭제 시도
        let t2 = super::reserve_tmp(&target).unwrap();
        let tmp2 = t2.path().to_path_buf();
        t2.commit().unwrap();
        assert!(target.exists(), "산출물이 제자리에 없다");
        assert!(!tmp2.exists(), "임시 파일이 남았다");

        // 5. 옆의 비슷한 이름 미변경
        let fake = root.join("notes.zmtmp-0123456789abcdef0123456789abcdef");
        fs::write(&fake, b"user").unwrap();
        let t3 = super::reserve_tmp(&target).unwrap();
        drop(t3);
        assert!(fake.exists(), "옆의 사용자 파일을 지웠다");

        let _ = fs::remove_dir_all(&root);
    }

    /// StagedFile 도 마감 못 부른 경로에서 자동 정리(? 이탈, 워커 패닉)
    /// 이동 후 재삭제 금지, 그 자리에 산출물 존재
    #[test]
    fn 해제_임시파일도_스스로_치운다() {
        let root = std::env::temp_dir().join(format!("zm_staged_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let target = root.join("풀린파일.txt");

        // 1. 마감 없이 놓으면(패닉, 조기 반환) 임시 파일 잔존 없음
        let (f, staged) = super::StagedFile::create(&target).unwrap();
        let tmp = staged.path().to_path_buf();
        drop(f);
        drop(staged);
        assert!(!tmp.exists(), "마감하지 않았는데 임시 파일이 남았다");
        assert!(!target.exists(), "쓰지도 않은 대상이 생겼다");

        // 2. commit → 제자리 이동, 이후 Drop 의 재삭제 없음
        let (mut f2, staged2) = super::StagedFile::create(&target).unwrap();
        {
            use std::io::Write;
            f2.write_all("내용".as_bytes()).unwrap();
        }
        let tmp2 = staged2.path().to_path_buf();
        drop(f2);
        staged2.commit().unwrap();
        assert!(target.exists(), "산출물이 제자리에 없다");
        assert_eq!(fs::read(&target).unwrap(), "내용".as_bytes(), "내용이 다르다");
        assert!(!tmp2.exists(), "임시 파일이 남았다");

        // 3. abort 는 기존 파일 미변경
        let (f3, staged3) = super::StagedFile::create(&target).unwrap();
        let tmp3 = staged3.path().to_path_buf();
        drop(f3);
        staged3.abort();
        assert!(!tmp3.exists(), "임시 파일이 남았다");
        assert_eq!(fs::read(&target).unwrap(), "내용".as_bytes(), "기존 파일을 건드렸다");

        let _ = fs::remove_dir_all(&root);
    }

    /// rename 실패 = 마감 아님, 소유를 먼저 놓으면 남은 파일을 Drop 도 종료 기록도 모름
    #[test]
    fn 옮기지_못하면_소유를_놓지_않는다() {
        let dir = std::env::temp_dir().join(format!("zm-commit-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("작업 폴더");

        // 대상 자리에 폴더 → 그 이름으로 rename 불가
        let target = dir.join("out.zip");
        fs::create_dir_all(&target).expect("대상 폴더");

        let tmp = super::reserve_tmp(&target).expect("임시 자리");
        let path = tmp.path().to_path_buf();
        assert!(path.exists(), "임시 파일이 만들어지지 않았다");
        assert!(
            super::live_temp_paths().contains(&path),
            "살아 있는 임시 파일 목록에 없다"
        );

        let err = tmp.commit().expect_err("옮길 수 있으면 안 되는 상황이다");
        assert_eq!(err.code, "output_error");

        // 소유 유지 → Drop 이 정리, 파일과 목록 모두 없음
        assert!(!path.exists(), "임시 파일이 남았다");
        assert!(
            !super::live_temp_paths().contains(&path),
            "치운 파일이 목록에 남았다"
        );
        // 기존 대상(폴더) 그대로
        assert!(target.is_dir(), "대상을 건드렸다");

        let _ = fs::remove_dir_all(&dir);
    }
}
