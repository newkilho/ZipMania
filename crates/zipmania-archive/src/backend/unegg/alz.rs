//! ALZ 포맷 파서, 규범 = nest/format/alz, 교차 검증 = unalz, 실측 대상 = test.alz (E10.2)
//!
//! [Global 8B] [File Header]filename[(암호 시 verify 12B)]data ... [Comment Block (선택)]
//! [Footer 16B: END / 주석크기 / CRC / FILE_END]

use crate::error::ZipManiaError;

use super::codec::Method;
use super::reader::Reader;
use super::times;

// ── 시그니처 (nest/format/alz/ALZTypes.h) ────────────────────────────────────
pub const SIG_ALZ: u32 = 0x015A_4C41; // "ALZ\x01"
const SIG_FILE: u32 = 0x015A_4C42;
const SIG_END: u32 = 0x015A_4C43;
const SIG_COMMENT: u32 = 0x015A_4C45;
const SIG_SPLIT: u32 = 0x035A_4C43; // 뒤에 볼륨 추가 존재
const SIG_FILE_END: u32 = 0x025A_4C43; // 마지막(또는 단일) 볼륨

/// 원본이 요구하는 유일 버전(ALZFormat::Open)
const SUPPORTED_VERSION: u16 = 10;

const ATTR_DIRECTORY: u8 = 0x10;
const MAX_FILES: usize = 1_000_000;
/// 주석 블록 상한, 주석 파싱 구현 시 사용, README §3.2.4
#[allow(dead_code)]
const MAX_COMMENT_BYTES: u32 = 16 << 20;

/// ALZ 파일 항목
#[derive(Debug, Clone)]
pub struct AlzFile {
    pub raw_name: Vec<u8>,
    pub attributes: u8,
    pub dos_time: u32,
    pub flags: u16,
    pub method: Method,
    pub crc: u32,
    pub packed_size: u64,
    pub unpacked_size: u64,
    pub data_offset: usize,
    pub verify_data: Vec<u8>,
    pub incomplete: bool,
}

impl AlzFile {
    /// ALZ 는 UTF-8 플래그 없음 → 항상 CP949
    pub fn name(&self) -> String {
        super::decode_legacy(&self.raw_name)
    }

    pub fn is_dir(&self) -> bool {
        self.attributes & ATTR_DIRECTORY != 0
    }

    pub fn is_encrypted(&self) -> bool {
        self.flags & 0x01 != 0
    }

    pub fn modified(&self) -> String {
        times::dos_to_string(self.dos_time)
    }

    /// ZipCrypto 검증 바이트, bit3 = unalz 해석(데이터 디스크립터), 실측 샘플 없음
    pub fn zip_verify_byte(&self) -> u8 {
        if self.flags & 0x08 != 0 {
            ((self.dos_time >> 8) & 0xFF) as u8
        } else {
            ((self.crc >> 24) & 0xFF) as u8
        }
    }
}

/// 파싱된 ALZ 아카이브
#[derive(Debug, Clone)]
pub struct AlzArchive {
    pub version: u16,
    pub files: Vec<AlzFile>,
    pub is_spanned: bool,
    pub parsed_end: usize,
}

/// 선두가 ALZ 시그니처인가
pub fn sniff(data: &[u8]) -> bool {
    super::reader::peek_sig(data) == Some(SIG_ALZ)
}

/// 파일 헤더 + 이름 읽기 → 데이터 구간 건너뜀
fn read_file(r: &mut Reader<'_>, total: usize) -> Result<AlzFile, ZipManiaError> {
    r.u32()?; // FILE 시그니처(호출자가 확인 완료)
    let name_len = r.u16()? as usize;
    let attributes = r.u8()?;
    let dos_time = r.u32()?;
    // nest = u16 flags, unalz = fileDescriptor + unknown 2바이트, 이름만 다르고 해석 결과 동일
    let flags = r.u16()?;

    let mut method = Method::Store;
    let mut crc = 0;
    let mut packed = 0u64;
    let mut unpacked = 0u64;
    if flags != 0 {
        // 상위 니블 = 크기 필드 바이트 폭(1/2/4/8), nest, unalz 일치
        let base = ((flags & 0xF0) >> 4) as u8;
        if !matches!(base, 1 | 2 | 4 | 8) {
            return Err(ZipManiaError::new(
                "corrupt",
                format!("잘못된 크기 필드 폭입니다({base}, flags=0x{flags:04X})"),
            ));
        }
        method = Method::from_alz(r.u16()?);
        crc = r.u32()?;
        packed = r.var_uint(base)?;
        unpacked = r.var_uint(base)?;
    }

    let raw_name = r.bytes(name_len)?.to_vec();

    let verify_data = if flags & 0x01 != 0 {
        r.bytes(12)?.to_vec() // ZipCrypto 검증 헤더
    } else {
        Vec::new()
    };

    let data_offset = r.pos();
    let incomplete = data_offset as u64 + packed > total as u64;
    if incomplete {
        // 분할 볼륨 앞부분만 존재(nest Result::NeedMoreStream)
        r.seek(total);
    } else {
        r.skip(packed as usize)?;
    }

    Ok(AlzFile {
        raw_name,
        attributes,
        dos_time,
        flags,
        method,
        crc,
        packed_size: packed,
        unpacked_size: unpacked,
        data_offset,
        verify_data,
        incomplete,
    })
}

/// 끝 16바이트로 분할 여부 판정
/// 마지막 u32 = ScanFooters 가 읽고 안 쓰는 값, 실측 결과 FILE_END 시그니처
fn read_footer(data: &[u8]) -> bool {
    if data.len() < 16 {
        return false;
    }
    super::reader::peek_sig(&data[data.len() - 4..]) == Some(SIG_SPLIT)
}

/// ALZ 아카이브 전체 구조 읽기, 압축 데이터 미접근
pub fn parse(data: &[u8]) -> Result<AlzArchive, ZipManiaError> {
    let mut r = Reader::new(data);
    if r.peek_u32() != Some(SIG_ALZ) {
        return Err(ZipManiaError::new("unsupported", "ALZ 시그니처가 아닙니다."));
    }
    r.u32()?;

    let version = r.u16()?;
    let _disk_id = r.u16()?;
    if version != SUPPORTED_VERSION {
        return Err(ZipManiaError::new(
            "unsupported",
            format!("지원하지 않는 ALZ 버전입니다({version}, 10만 지원)."),
        ));
    }

    let mut files = Vec::new();
    while r.peek_u32() == Some(SIG_FILE) {
        if files.len() >= MAX_FILES {
            return Err(ZipManiaError::new("corrupt", "파일 수 상한을 넘습니다."));
        }
        let entry = read_file(&mut r, data.len())?;
        let stop = entry.incomplete;
        files.push(entry);
        if stop {
            break;
        }
    }

    Ok(AlzArchive {
        version,
        files,
        is_spanned: read_footer(data),
        parsed_end: r.pos(),
    })
}

/// 미사용 상수의 死코드 경고 억제, 주석 블록, 다중 볼륨 구현 시 사용
#[allow(dead_code)]
const _RESERVED_SIGS: [u32; 3] = [SIG_END, SIG_COMMENT, SIG_FILE_END];
