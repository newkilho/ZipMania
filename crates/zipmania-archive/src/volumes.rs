//! 분할 볼륨 이름 규칙(<대상>.001 ..)과 읽기 스트림, 쓰기는 outfile::VolumeSet
//! 7-Zip 규칙 = 통짜 바이트열을 볼륨 크기로 자른 것, 안의 포맷은 첫 볼륨을 뗀 이름의 확장자 (D3.18)

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// 분할 볼륨 경로 = <대상>.NNN(1부터, 3자리, 1000 이상은 자릿수 확장)
pub fn volume_name(target: &Path, index: usize) -> PathBuf {
    let name = target
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    target.with_file_name(format!("{name}.{index:03}"))
}

/// 첫 볼륨 경로(<대상>.001)면 대상 경로, 아니면 None(.002 이후, 일반 파일)
pub fn split_base(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_str()?;
    let (base, num) = name.rsplit_once('.')?;
    if base.is_empty() || num.len() < 3 || !num.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if num.parse::<u64>().ok()? != 1 {
        return None;
    }
    Some(path.with_file_name(base))
}

/// 볼륨 세트 이어 읽기(Read + Seek), 첫 볼륨 경로로 열고 번호가 끊길 때까지 수집
pub struct VolumeReader {
    files: Vec<(File, u64)>,
    total: u64,
    pos: u64,
}

impl VolumeReader {
    /// first = <대상>.001, 볼륨은 연속 번호, 중간 결번 = 거기까지
    pub fn open(first: &Path) -> std::io::Result<Self> {
        let base = split_base(first)
            .ok_or_else(|| std::io::Error::other("첫 볼륨(.001)이 아님"))?;
        let mut files = Vec::new();
        let mut total = 0u64;
        for i in 1.. {
            let p = volume_name(&base, i);
            if i > 1 && !p.exists() {
                break;
            }
            let f = File::open(&p)?;
            let len = f.metadata()?.len();
            total += len;
            files.push((f, len));
        }
        Ok(Self { files, total, pos: 0 })
    }

    pub fn volume_count(&self) -> usize {
        self.files.len()
    }

    pub fn len(&self) -> u64 {
        self.total
    }

    pub fn is_empty(&self) -> bool {
        self.total == 0
    }

    /// 절대 위치 → (볼륨 번호, 볼륨 안 오프셋), 끝 이후 = None
    fn locate(&self, pos: u64) -> Option<(usize, u64)> {
        let mut start = 0u64;
        for (i, (_, len)) in self.files.iter().enumerate() {
            if pos < start + *len {
                return Some((i, pos - start));
            }
            start += *len;
        }
        None
    }
}

impl Read for VolumeReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let Some((idx, off)) = self.locate(self.pos) else {
            return Ok(0);
        };
        let (f, len) = &mut self.files[idx];
        let room = (*len - off) as usize;
        let want = buf.len().min(room);
        f.seek(SeekFrom::Start(off))?;
        let n = f.read(&mut buf[..want])?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for VolumeReader {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let (base, delta) = match pos {
            SeekFrom::Start(p) => {
                self.pos = p;
                return Ok(p);
            }
            SeekFrom::Current(d) => (self.pos as i64, d),
            SeekFrom::End(d) => (self.total as i64, d),
        };
        let np = base
            .checked_add(delta)
            .filter(|v| *v >= 0)
            .ok_or_else(|| std::io::Error::other("탐색 위치 범위 밖"))?;
        self.pos = np as u64;
        Ok(self.pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn 첫_볼륨만_분할로_본다() {
        assert_eq!(split_base(Path::new("a/b.7z.001")), Some(PathBuf::from("a/b.7z")));
        assert_eq!(split_base(Path::new("b.zip.0001")), Some(PathBuf::from("b.zip")));
        assert!(split_base(Path::new("b.7z.002")).is_none());
        assert!(split_base(Path::new("b.7z")).is_none());
        assert!(split_base(Path::new("b.01")).is_none());
        assert!(split_base(Path::new(".001")).is_none());
    }

    /// 경계 넘는 읽기, 끝 너머 seek 뒤 읽기 = 0, 결번에서 멈춤
    #[test]
    fn 볼륨을_이어_읽는다() {
        let root = std::env::temp_dir().join(format!("zm_vr_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let base = root.join("x.7z");
        let data: Vec<u8> = (0u8..25).collect();
        for (i, chunk) in data.chunks(10).enumerate() {
            fs::write(volume_name(&base, i + 1), chunk).unwrap();
        }
        // 결번 뒤의 볼륨은 세트 밖
        fs::write(volume_name(&base, 5), b"zz").unwrap();

        let mut r = VolumeReader::open(&volume_name(&base, 1)).unwrap();
        assert_eq!(r.volume_count(), 3);
        assert_eq!(r.len(), 25);
        let mut all = Vec::new();
        r.read_to_end(&mut all).unwrap();
        assert_eq!(all, data);

        r.seek(SeekFrom::Start(8)).unwrap();
        let mut b = [0u8; 4];
        r.read_exact(&mut b).unwrap();
        assert_eq!(b, [8, 9, 10, 11]);

        assert_eq!(r.seek(SeekFrom::End(-1)).unwrap(), 24);
        assert_eq!(r.read(&mut b).unwrap(), 1);
        assert_eq!(r.read(&mut b).unwrap(), 0);
        r.seek(SeekFrom::Start(100)).unwrap();
        assert_eq!(r.read(&mut b).unwrap(), 0);

        assert!(VolumeReader::open(&volume_name(&base, 2)).is_err());
        let _ = fs::remove_dir_all(&root);
    }
}
