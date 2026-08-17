//! EGG 포맷 파서, 명세 = nest 역추출(E3.1), 실측 대상 = test.egg(E10.1)
//!
//! [Global Header 14B]
//! [Archive Extra Fields ...] END
//!  ├ (일반) FileExtrasEND BlockExtrasdata ... END
//!  └ (솔리드) FileExtrasEND File ... END
//!  BlockExtrasdata ... END ← 전 파일이 공유하는 블록 스트림
//! [Footer Extra Fields ...] END

use std::collections::HashMap;

use crate::error::ZipManiaError;

use super::codec::Method;
use super::reader::Reader;
use super::times;

// ── 시그니처 (nest/format/egg/EggTypes.h) ────────────────────────────────────
pub const SIG_EGG: u32 = 0x4147_4745; // "EGGA"
const SIG_FILE: u32 = 0x0A85_90E3;
const SIG_BLOCK: u32 = 0x02B5_0C13;
const SIG_ENCRYPT: u32 = 0x08D1_470F;
const SIG_WINDOWS: u32 = 0x2C86_950B;
const SIG_POSIX: u32 = 0x1EE9_22E5;
const SIG_FILENAME: u32 = 0x0A85_91AC;
/// 주석 확장필드, 주석 노출 구현 시 사용, README §3.1.5
#[allow(dead_code)]
const SIG_COMMENT: u32 = 0x04C6_3672;
const SIG_SPLIT: u32 = 0x24F5_A262;
const SIG_SOLID: u32 = 0x24E5_A060;
const SIG_END: u32 = 0x08E2_8222;

/// 원본 상수(EggTypes.h), 손상 파일, 압축 폭탄 방어 상한으로 그대로 사용
const MAX_HEADERS: usize = 100;
const MAX_BLOCKS: usize = 65_536;
const MAX_EXTRA_BYTES: u64 = 16 << 20;
const MAX_FILES: usize = 1_000_000;

/// 확장필드 1개
#[derive(Debug, Clone)]
pub struct Extra {
    pub gpb: u8,
    pub data: Vec<u8>,
}

impl Extra {
    /// 1-base 비트 플래그, 원본 GetBitFlag(n) = gpb & (1 << (n-1)) 와 동일
    fn bit(&self, index: u8) -> bool {
        self.gpb & (1 << (index - 1)) != 0
    }
}

/// 압축 블록 1개
#[derive(Debug, Clone)]
pub struct Block {
    pub method: Method,
    pub unpacked_size: u32,
    pub packed_size: u32,
    pub crc: u32,
    pub data_offset: usize,
}

/// 아카이브 내 파일 항목
#[derive(Debug, Clone)]
pub struct EggFile {
    pub id: u32,
    pub size: u64,
    pub extras: HashMap<u32, Extra>,
    pub blocks: Vec<Block>,
}

impl EggFile {
    /// bit4 꺼짐 = 참(FilenameField::IsUTF8), 주석은 판정 반대
    fn name_is_utf8(&self) -> bool {
        self.extras.get(&SIG_FILENAME).map_or(true, |e| !e.bit(4))
    }

    fn name_is_absolute(&self) -> bool {
        self.extras.get(&SIG_FILENAME).is_some_and(|e| e.bit(5))
    }

    /// 파일명, UTF-8 플래그 없으면 CP949 디코드
    pub fn name(&self) -> String {
        let Some(e) = self.extras.get(&SIG_FILENAME) else {
            return format!("<이름없음 #{}>", self.id);
        };
        // [locale u16 (UTF-8 아닐 때)] [parent_id u32 (절대경로일 때)] [name...]
        let off = (if self.name_is_absolute() { 6 } else { 2 })
            - (if self.name_is_utf8() { 2 } else { 0 });
        let raw = e.data.get(off..).unwrap_or_default();
        if self.name_is_utf8() {
            String::from_utf8_lossy(raw).into_owned()
        } else {
            super::decode_legacy(raw)
        }
    }

    /// EggFormat::IsTargetDir, Windows = attr & 128(EGG 자체 비트, Win32 의 0x10 아님)
    /// POSIX = mode & 0x40000
    pub fn is_dir(&self) -> bool {
        if let Some(e) = self.extras.get(&SIG_WINDOWS) {
            if e.data.len() >= 9 {
                return e.data[8] & 128 != 0;
            }
        }
        if let Some(e) = self.extras.get(&SIG_POSIX) {
            if e.data.len() >= 4 {
                let mode = u32::from_le_bytes([e.data[0], e.data[1], e.data[2], e.data[3]]);
                return mode & 0x0004_0000 != 0;
            }
        }
        false
    }

    /// 수정 시각 문자열 YYYY-MM-DD HH:MM:SS, 없으면 빈 문자열
    pub fn modified(&self) -> String {
        if let Some(e) = self.extras.get(&SIG_WINDOWS) {
            if e.data.len() >= 8 {
                let mut b = [0u8; 8];
                b.copy_from_slice(&e.data[..8]);
                return times::filetime_to_string(i64::from_le_bytes(b));
            }
        }
        if let Some(e) = self.extras.get(&SIG_POSIX) {
            if e.data.len() >= 20 {
                let mut b = [0u8; 8];
                b.copy_from_slice(&e.data[12..20]);
                return times::unix_to_string(i64::from_le_bytes(b));
            }
        }
        String::new()
    }

    pub fn is_encrypted(&self) -> bool {
        self.extras.contains_key(&SIG_ENCRYPT)
    }

    /// 암호화 방식: 0=ZipCrypto 1=AES-128 2=AES-256
    pub fn encrypt_method(&self) -> Option<u8> {
        self.extras.get(&SIG_ENCRYPT).and_then(|e| e.data.first().copied())
    }

    /// 배치: 0=method, 1..13=verify, 13..17=crc
    /// 검증 바이트 = 기록된 CRC 의 최상위 1바이트(원본 ZipDecoder 와 동일)
    pub fn zip_verify(&self) -> Option<(Vec<u8>, u8)> {
        let e = self.extras.get(&SIG_ENCRYPT)?;
        if e.data.len() < 17 {
            return None;
        }
        let crc = u32::from_le_bytes([e.data[13], e.data[14], e.data[15], e.data[16]]);
        Some((e.data[1..13].to_vec(), (crc >> 24) as u8))
    }

    pub fn packed_size(&self) -> u64 {
        self.blocks.iter().map(|b| b.packed_size as u64).sum()
    }

    pub fn crc(&self) -> Option<u32> {
        self.blocks.first().map(|b| b.crc)
    }
}

/// 파싱된 EGG 아카이브
#[derive(Debug, Clone)]
pub struct EggArchive {
    pub version: u16,
    pub files: Vec<EggFile>,
    pub solid_blocks: Vec<Block>,
    extras: HashMap<u32, Extra>,
    pub parsed_end: usize,
}

impl EggArchive {
    pub fn is_solid(&self) -> bool {
        self.extras.contains_key(&SIG_SOLID)
    }

    pub fn is_spanned(&self) -> bool {
        self.extras.contains_key(&SIG_SPLIT)
    }
}

/// 선두가 EGG 시그니처인가
pub fn sniff(data: &[u8]) -> bool {
    super::reader::peek_sig(data) == Some(SIG_EGG)
}

/// 알려진 시그니처인가, 확장필드 경계 판정용
fn is_known_sig(sig: u32) -> bool {
    matches!(
        sig,
        SIG_END
            | SIG_FILE
            | SIG_BLOCK
            | SIG_ENCRYPT
            | SIG_WINDOWS
            | SIG_POSIX
            | SIG_FILENAME
            | SIG_COMMENT
            | SIG_SPLIT
            | SIG_SOLID
    )
}

/// END 까지 확장필드 읽기, END 는 소비
fn read_extras(r: &mut Reader<'_>) -> Result<HashMap<u32, Extra>, ZipManiaError> {
    let mut out = HashMap::new();
    for _ in 0..=MAX_HEADERS {
        let Some(sig) = r.peek_u32() else {
            return Err(ZipManiaError::new(
                "corrupt",
                format!("확장필드 목록이 END 없이 끝났습니다(오프셋 {})", r.pos()),
            ));
        };
        r.u32()?;
        if sig == SIG_END {
            return Ok(out);
        }
        let gpb = r.u8()?;
        let mut size = if gpb & 1 != 0 {
            r.u32()? as u64
        } else {
            r.u16()? as u64
        };
        // ENCRYPT 크기 해석: 원본 vs 실측 불일치
        // 원본 ExtraField.cpp = size_ -= 7. 실측(test2.egg) = size 가 데이터 크기 그대로(17)
        // 7 빼면 스트림 어긋나 뒤가 전부 깨짐 → 뒤따르는 시그니처가 맞는 쪽 선택, 애매하면 실측 우선
        if sig == SIG_ENCRYPT && size >= 7 {
            let as_is = r.peek_u32_at(size as usize);
            let minus7 = r.peek_u32_at(size as usize - 7);
            if !as_is.is_some_and(is_known_sig) && minus7.is_some_and(is_known_sig) {
                size -= 7;
            }
        }
        if size > MAX_EXTRA_BYTES {
            return Err(ZipManiaError::new(
                "corrupt",
                format!("확장필드가 너무 큽니다({size}바이트)"),
            ));
        }
        let data = r.bytes(size as usize)?.to_vec();
        out.entry(sig).or_insert(Extra { gpb, data });
    }
    Err(ZipManiaError::new(
        "corrupt",
        format!("확장필드가 {MAX_HEADERS}개를 넘습니다"),
    ))
}

/// 블록 헤더 + 확장필드 읽기 → 데이터 구간 건너뜀
fn read_block(r: &mut Reader<'_>) -> Result<Block, ZipManiaError> {
    let sig = r.u32()?;
    if sig != SIG_BLOCK {
        return Err(ZipManiaError::new(
            "corrupt",
            format!("블록 시그니처가 아닙니다(오프셋 {})", r.pos() - 4),
        ));
    }
    let method = Method::from_egg(r.u8()?);
    let _hint = r.u8()?;
    let unpacked_size = r.u32()?;
    let packed_size = r.u32()?;
    let crc = r.u32()?;
    let _extras = read_extras(r)?;
    let data_offset = r.pos();
    r.skip(packed_size as usize)?;
    Ok(Block {
        method,
        unpacked_size,
        packed_size,
        crc,
        data_offset,
    })
}

/// EGG 아카이브 전체 구조 읽기, 압축 데이터 미접근
pub fn parse(data: &[u8]) -> Result<EggArchive, ZipManiaError> {
    let mut r = Reader::new(data);
    if r.peek_u32() != Some(SIG_EGG) {
        return Err(ZipManiaError::new("unsupported", "EGG 시그니처가 아닙니다."));
    }
    r.u32()?;

    let version = r.u16()?;
    let _id = r.u32()?;
    let _reserved = r.u32()?;
    if version > 0x0100 {
        return Err(ZipManiaError::new(
            "unsupported",
            format!("지원하지 않는 EGG 버전입니다(0x{version:04X})."),
        ));
    }

    let extras = read_extras(&mut r)?;
    let solid = extras.contains_key(&SIG_SOLID);

    let mut files: Vec<EggFile> = Vec::new();
    while r.peek_u32() == Some(SIG_FILE) {
        if files.len() >= MAX_FILES {
            return Err(ZipManiaError::new("corrupt", "파일 수 상한을 넘습니다."));
        }
        r.u32()?;
        let id = r.u32()?;
        let size = r.u64()?;
        let file_extras = read_extras(&mut r)?;
        let mut blocks = Vec::new();
        if !solid {
            while r.peek_u32() == Some(SIG_BLOCK) {
                if blocks.len() >= MAX_BLOCKS {
                    return Err(ZipManiaError::new("corrupt", "블록 수 상한을 넘습니다."));
                }
                blocks.push(read_block(&mut r)?);
            }
        }
        files.push(EggFile {
            id,
            size,
            extras: file_extras,
            blocks,
        });
    }

    let mut solid_blocks = Vec::new();
    if solid {
        while r.peek_u32() == Some(SIG_BLOCK) {
            if solid_blocks.len() >= MAX_BLOCKS {
                return Err(ZipManiaError::new("corrupt", "블록 수 상한을 넘습니다."));
            }
            solid_blocks.push(read_block(&mut r)?);
        }
    }

    // 목록 종료 END 1개가 footer END 겸용(원본도 되감아 재독)
    if r.peek_u32().is_some() {
        let _footer = read_extras(&mut r)?;
    }

    Ok(EggArchive {
        version,
        files,
        solid_blocks,
        extras,
        parsed_end: r.pos(),
    })
}
