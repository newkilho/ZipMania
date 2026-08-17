//! COM 스트림 구현(파일, 메모리)
//! InStream → IInStream, ISequentialInStream, IStreamGetSize
//! OutStream → IOutStream, ISequentialOutStream

use std::ffi::c_void;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use windows_core::HRESULT;

use super::com::*;
use crate::error::ZipManiaError;

fn io_err(ctx: &str, e: std::io::Error) -> ZipManiaError {
    ZipManiaError::new("io_error", format!("{ctx}: {e}"))
}

// ─────────────────────────── 입력 스트림 ───────────────────────────

enum InBacking {
    File(File),
    Mem { data: Vec<u8>, pos: u64 },
}

impl InBacking {
    fn read(&mut self, buf: &mut [u8]) -> usize {
        match self {
            InBacking::File(f) => f.read(buf).unwrap_or(0),
            InBacking::Mem { data, pos } => {
                let cur = (*pos).min(data.len() as u64) as usize;
                let n = (data.len() - cur).min(buf.len());
                buf[..n].copy_from_slice(&data[cur..cur + n]);
                *pos = (cur + n) as u64;
                n
            }
        }
    }

    fn seek(&mut self, offset: i64, origin: u32) -> Option<u64> {
        match self {
            InBacking::File(f) => {
                let from = match origin {
                    STREAM_SEEK_SET => SeekFrom::Start(offset.max(0) as u64),
                    STREAM_SEEK_CUR => SeekFrom::Current(offset),
                    STREAM_SEEK_END => SeekFrom::End(offset),
                    _ => return None,
                };
                f.seek(from).ok()
            }
            InBacking::Mem { data, pos } => {
                let base = match origin {
                    STREAM_SEEK_SET => 0i64,
                    STREAM_SEEK_CUR => *pos as i64,
                    STREAM_SEEK_END => data.len() as i64,
                    _ => return None,
                };
                let np = (base + offset).max(0) as u64;
                *pos = np;
                Some(np)
            }
        }
    }

    fn size(&mut self) -> u64 {
        match self {
            InBacking::File(f) => f.metadata().map(|m| m.len()).unwrap_or(0),
            InBacking::Mem { data, .. } => data.len() as u64,
        }
    }
}

/// 파일, 메모리 백킹 입력 스트림
#[windows_core::implement(IInStream, ISequentialInStream, IStreamGetSize)]
struct InStream {
    inner: Mutex<InBacking>,
}

impl InStream {
    unsafe fn do_read(&self, data: *mut c_void, size: u32, processed: *mut u32) -> HRESULT {
        if !processed.is_null() {
            *processed = 0;
        }
        if size == 0 || data.is_null() {
            return S_OK;
        }
        let buf = std::slice::from_raw_parts_mut(data as *mut u8, size as usize);
        let n = self.inner.lock().unwrap().read(buf);
        if !processed.is_null() {
            *processed = n as u32;
        }
        S_OK
    }

    unsafe fn do_seek(&self, offset: i64, origin: u32, new_pos: *mut u64) -> HRESULT {
        match self.inner.lock().unwrap().seek(offset, origin) {
            Some(np) => {
                if !new_pos.is_null() {
                    *new_pos = np;
                }
                S_OK
            }
            None => E_INVALIDARG,
        }
    }
}

impl IInStream_Impl for InStream_Impl {
    unsafe fn Read(&self, data: *mut c_void, size: u32, processed: *mut u32) -> HRESULT {
        self.do_read(data, size, processed)
    }
    unsafe fn Seek(&self, offset: i64, origin: u32, new_pos: *mut u64) -> HRESULT {
        self.do_seek(offset, origin, new_pos)
    }
}

impl ISequentialInStream_Impl for InStream_Impl {
    unsafe fn Read(&self, data: *mut c_void, size: u32, processed: *mut u32) -> HRESULT {
        self.do_read(data, size, processed)
    }
}

impl IStreamGetSize_Impl for InStream_Impl {
    unsafe fn GetSize(&self, size: *mut u64) -> HRESULT {
        if !size.is_null() {
            *size = self.inner.lock().unwrap().size();
        }
        S_OK
    }
}

/// 디스크 파일 입력 스트림(IInStream), 아카이브 열기용
pub fn open_input_file(path: &Path) -> Result<IInStream, ZipManiaError> {
    let f = File::open(path).map_err(|e| io_err("아카이브를 열지 못했습니다", e))?;
    Ok(InStream {
        inner: Mutex::new(InBacking::File(f)),
    }
    .into())
}

/// 메모리 바이트 백킹 입력 스트림(IInStream), 테스트, 왕복용
pub fn input_from_mem(data: Vec<u8>) -> IInStream {
    InStream {
        inner: Mutex::new(InBacking::Mem { data, pos: 0 }),
    }
    .into()
}

/// 디스크 파일 생성 소스 스트림(ISequentialInStream)
pub fn source_file(path: &Path) -> Result<ISequentialInStream, ZipManiaError> {
    let f = File::open(path).map_err(|e| io_err("입력 파일을 열지 못했습니다", e))?;
    Ok(InStream {
        inner: Mutex::new(InBacking::File(f)),
    }
    .into())
}

// ─────────────────────────── 출력 스트림 ───────────────────────────

enum OutBacking {
    File(File),
    Mem(Arc<Mutex<Vec<u8>>>),
    /// 임의 writer 순차 스트리밍(드래그 지연 렌더링 바운디드 채널 등), 탐색 불가
    Writer(Box<dyn std::io::Write + Send>),
}

struct OutInner {
    backing: OutBacking,
    pos: u64,
    mem_cap: u64,
}

impl OutInner {
    fn write(&mut self, src: &[u8]) -> bool {
        match &mut self.backing {
            OutBacking::File(f) => {
                if f.write_all(src).is_err() {
                    return false;
                }
                self.pos += src.len() as u64;
                true
            }
            OutBacking::Mem(buf) => {
                // 압축 폭탄 방어 — 실제로 흘러나온 바이트를 세고 넘으면 E_FAIL(자르지 않는다)
                // 덧셈 오버플로 주의 — Seek 로 위치를 크게 잡으면 검사 우회
                let end = match self.pos.checked_add(src.len() as u64) {
                    Some(v) => v,
                    None => return false,
                };
                if end > self.mem_cap {
                    return false;
                }
                let mut v = buf.lock().unwrap();
                let p = self.pos as usize;
                if p + src.len() > v.len() {
                    v.resize(p + src.len(), 0);
                }
                v[p..p + src.len()].copy_from_slice(src);
                self.pos += src.len() as u64;
                true
            }
            OutBacking::Writer(w) => {
                if w.write_all(src).is_err() {
                    return false;
                }
                self.pos += src.len() as u64;
                true
            }
        }
    }

    fn seek(&mut self, offset: i64, origin: u32) -> Option<u64> {
        let len = match &mut self.backing {
            OutBacking::File(f) => f.metadata().map(|m| m.len()).unwrap_or(0) as i64,
            OutBacking::Mem(buf) => buf.lock().unwrap().len() as i64,
            // 스트리밍 writer = 탐색 불가, 현재 위치 기준만 인정
            OutBacking::Writer(_) => self.pos as i64,
        };
        let base = match origin {
            STREAM_SEEK_SET => 0i64,
            STREAM_SEEK_CUR => self.pos as i64,
            STREAM_SEEK_END => len,
            _ => return None,
        };
        // 오버플로 시 실패 반환(랩어라운드로 엉뚱한 위치 방지)
        let np = base.checked_add(offset)?.max(0) as u64;
        match &mut self.backing {
            OutBacking::File(f) => {
                f.seek(SeekFrom::Start(np)).ok()?;
            }
            // 상한 밖 이동 금지, 이동만 해도 다음 쓰기가 그 위치 기준 판정 → 상한 무의미
            OutBacking::Mem(_) if np > self.mem_cap => return None,
            _ => {}
        }
        self.pos = np;
        Some(np)
    }

    fn set_size(&mut self, new_size: u64) -> bool {
        match &mut self.backing {
            OutBacking::File(f) => f.set_len(new_size).is_ok(),
            OutBacking::Mem(buf) => {
                // Write 와 동일 상한, 여기만 열면 상한 없음과 동일
                // SetSize 로 크기를 선점하는 포맷에서 작은 아카이브 1개가 1회 호출로 수 GB 할당
                if new_size > self.mem_cap {
                    return false;
                }
                buf.lock().unwrap().resize(new_size as usize, 0);
                true
            }
            // writer 는 크기 선지정 개념 없음 → 무시하고 성공 처리
            OutBacking::Writer(_) => true,
        }
    }
}

/// 파일, 메모리 백킹 탐색 가능 출력 스트림
#[windows_core::implement(IOutStream, ISequentialOutStream)]
struct OutStream {
    inner: Mutex<OutInner>,
}

impl OutStream {
    unsafe fn do_write(&self, data: *const u8, size: u32, processed: *mut u32) -> HRESULT {
        if size > 0 && !data.is_null() {
            let src = std::slice::from_raw_parts(data, size as usize);
            if !self.inner.lock().unwrap().write(src) {
                return HRESULT(0x8007_0000u32 as i32); // E_FAIL 계열
            }
        }
        if !processed.is_null() {
            *processed = size;
        }
        S_OK
    }
}

impl IOutStream_Impl for OutStream_Impl {
    unsafe fn Write(&self, data: *const u8, size: u32, processed: *mut u32) -> HRESULT {
        self.do_write(data, size, processed)
    }
    unsafe fn Seek(&self, offset: i64, origin: u32, new_pos: *mut u64) -> HRESULT {
        match self.inner.lock().unwrap().seek(offset, origin) {
            Some(np) => {
                if !new_pos.is_null() {
                    *new_pos = np;
                }
                S_OK
            }
            None => E_INVALIDARG,
        }
    }
    unsafe fn SetSize(&self, new_size: u64) -> HRESULT {
        if self.inner.lock().unwrap().set_size(new_size) {
            S_OK
        } else {
            HRESULT(0x8007_0000u32 as i32)
        }
    }
}

impl ISequentialOutStream_Impl for OutStream_Impl {
    unsafe fn Write(&self, data: *const u8, size: u32, processed: *mut u32) -> HRESULT {
        self.do_write(data, size, processed)
    }
}

// 최종 경로를 직접 여는 헬퍼 금지, 있으면 다시 호출하게 됨
// StagedFile::create 의 핸들을 output_file_from 에 전달

/// 기존 핸들 기반 해제 출력 스트림, 기록 위치 결정 = crate::outfile::StagedFile,
/// 스트림은 받은 곳에 기록만, 경로 아닌 핸들을 받는 이유
pub fn output_file_from(f: std::fs::File) -> ISequentialOutStream {
    OutStream {
        inner: Mutex::new(OutInner {
            backing: OutBacking::File(f),
            pos: 0,
            mem_cap: crate::formats::MAX_MEMORY_ENTRY_BYTES,
        }),
    }
    .into()
}

/// 임의 writer 순차 스트리밍 추출 출력 스트림(ISequentialOutStream)
/// 용도 = 드래그 지연 렌더링의 바운디드 채널, 임시 파일, 전체 메모리 없음
pub fn output_writer(w: Box<dyn std::io::Write + Send>) -> ISequentialOutStream {
    OutStream {
        inner: Mutex::new(OutInner {
            backing: OutBacking::Writer(w),
            pos: 0,
            mem_cap: crate::formats::MAX_MEMORY_ENTRY_BYTES,
        }),
    }
    .into()
}

/// 지정 버퍼로 추출하는 메모리 출력 스트림(ISequentialOutStream)
pub fn mem_out_from(buf: Arc<Mutex<Vec<u8>>>) -> ISequentialOutStream {
    mem_out_capped(buf, crate::formats::MAX_MEMORY_ENTRY_BYTES)
}

/// 상한 하향 메모리 출력, 바이러스 검사용, 기준 = 실제 출력 바이트, (D3.5)
pub fn mem_out_capped(buf: Arc<Mutex<Vec<u8>>>, cap: u64) -> ISequentialOutStream {
    OutStream {
        inner: Mutex::new(OutInner {
            backing: OutBacking::Mem(buf),
            pos: 0,
            mem_cap: cap.min(crate::formats::MAX_MEMORY_ENTRY_BYTES),
        }),
    }
    .into()
}

/// 탐색 가능 생성 출력(IOutStream), 7z 생성 필수 — 인코더가 헤더를 되감아 패치
/// 경로 아닌 crate::outfile::reserve_tmp 의 독점 생성 핸들 수신, (D3.5)
pub fn output_seekable_file(f: std::fs::File) -> Result<IOutStream, ZipManiaError> {
    Ok(OutStream {
        inner: Mutex::new(OutInner {
            backing: OutBacking::File(f),
            pos: 0,
            mem_cap: crate::formats::MAX_MEMORY_ENTRY_BYTES,
        }),
    }
    .into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem(cap: u64) -> (OutInner, Arc<Mutex<Vec<u8>>>) {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let inner = OutInner {
            backing: OutBacking::Mem(buf.clone()),
            pos: 0,
            mem_cap: cap,
        };
        (inner, buf)
    }

    /// 상한 기준 = 실제 출력 바이트(신고 크기 무관), 초과 = 실패, 조용한 절단 금지
    #[test]
    fn 메모리_출력은_상한을_넘기지_못한다() {
        let (mut o, buf) = mem(8);
        assert!(o.write(&[7u8; 8]), "한도까지는 쓸 수 있어야 한다");
        assert!(!o.write(&[7u8; 1]), "한 바이트만 더 나와도 실패해야 한다");
        assert_eq!(buf.lock().unwrap().len(), 8, "넘친 만큼이 담기면 안 된다");
    }

    /// 분할 전송도 누적 판정, 조각 분할로 우회 불가
    #[test]
    fn 상한은_누적으로_판정한다() {
        let (mut o, _buf) = mem(4);
        assert!(o.write(&[1u8; 3]));
        assert!(!o.write(&[1u8; 2]), "3+2 는 한도 4 를 넘는다");
    }

    /// Seek 우회 불가, 위치 이동만으로도 다음 쓰기가 그 기준으로 판정, 덧셈 오버플로 시 검사 통과
    #[test]
    fn seek_으로_상한을_우회하지_못한다() {
        let (mut o, _buf) = mem(16);
        assert!(o.seek(16, super::STREAM_SEEK_SET).is_some(), "한도까지는 옮길 수 있다");
        assert!(o.seek(17, super::STREAM_SEEK_SET).is_none(), "한도 밖으로는 못 옮긴다");

        // 위치를 끝까지 올린 뒤 쓰면 덧셈 오버플로 → 랩어라운드 통과 금지
        let (mut o2, _b2) = mem(16);
        o2.pos = u64::MAX;
        assert!(!o2.write(&[0u8; 8]), "정수 넘침으로 상한을 통과했다");
    }

    /// SetSize 도 동일 상한 적용, 미적용 시 1회 호출로 수 GB 할당하는 뒷문
    #[test]
    fn set_size_도_같은_상한을_본다() {
        let (mut o, buf) = mem(16);
        assert!(o.set_size(16));
        assert_eq!(buf.lock().unwrap().len(), 16);
        assert!(!o.set_size(17), "한도를 넘는 크기 지정은 거부해야 한다");
        assert_eq!(buf.lock().unwrap().len(), 16, "거부했으면 늘어나지도 않는다");
    }
}
