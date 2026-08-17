//! EGG / ALZ 백엔드, 순수 Rust, 플랫폼 무관, 7z.dll 미지원 이스트소프트 포맷 담당
//!
//! 모듈: reader(엔디안 독립 유일 지점), egg/alz(컨테이너 파서), codec(해제, 미지원 거부)
//! , times, extract, 경로 정규화 = 백엔드 공용 crate::paths
//! 미지원(AZO, ALZ 변형 BZip2, EGG BZip2/LZMA, 솔리드, 분할) 전부 unsupported 거부
//! 명세, 검증 현황, 설계 근거 (D3.9)

pub mod alz;
pub mod codec;
pub mod crypto;
pub mod egg;
pub mod extract;
pub mod reader;
pub mod times;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::crc32::Crc32;
use crate::error::ZipManiaError;
use crate::models::ArchiveEntry;

use super::{
    ArchiveBackend, CreateOptions, CreateResult, ExtractOptions, ExtractResult, ProgressFn,
};
use codec::Method;

/// CP949/EUC-KR 바이트열 → 문자열, EGG = UTF-8 플래그 없을 때, ALZ = 항상
fn decode_legacy(raw: &[u8]) -> String {
    let (text, _, _) = encoding_rs::EUC_KR.decode(raw);
    text.into_owned()
}

/// 포맷 무관 정규화 항목, 목록, 해제, 읽기 공용
#[derive(Debug, Clone)]
pub struct Item {
    pub path: String,
    pub size: u64,
    pub packed_size: u64,
    pub crc: Option<u32>,
    pub is_dir: bool,
    pub modified: String,
    pub enc: Option<Enc>,
    pub blocks: Vec<BlockRef>,
}

impl Item {
    pub fn encrypted(&self) -> bool {
        self.enc.is_some()
    }
}

/// 항목 암호화 정보
#[derive(Debug, Clone)]
pub struct Enc {
    pub method: u8,
    pub verify: Vec<u8>,
    pub verify_byte: u8,
}

/// 항목을 이루는 압축 블록 1개의 위치
#[derive(Debug, Clone, Copy)]
pub struct BlockRef {
    pub method: Method,
    pub data_offset: usize,
    pub packed_size: u64,
    pub unpacked_size: u64,
    pub crc: u32,
}

/// EGG/ALZ 읽기 백엔드
pub struct Unegg;

impl Default for Unegg {
    fn default() -> Self {
        Unegg::new()
    }
}

impl Unegg {
    pub fn new() -> Self {
        Unegg
    }
}

/// 전체 일괄 읽기, 분할 볼륨 지원 시 여기가 다중 볼륨 리더
fn load(archive: &str) -> Result<Vec<u8>, ZipManiaError> {
    std::fs::read(archive)
        .map_err(|e| ZipManiaError::new("io_error", format!("아카이브를 열지 못했습니다: {e}")))
}

/// 바이트열 파싱 → 정규화 항목 목록
fn parse_items(data: &[u8]) -> Result<Vec<Item>, ZipManiaError> {
    if egg::sniff(data) {
        let ar = egg::parse(data)?;
        if ar.is_solid() {
            return Err(ZipManiaError::new(
                "unsupported",
                "솔리드 EGG 아카이브는 아직 지원하지 않습니다.",
            ));
        }
        if ar.is_spanned() {
            return Err(ZipManiaError::new(
                "unsupported",
                "분할된 EGG 아카이브는 아직 지원하지 않습니다.",
            ));
        }
        Ok(ar
            .files
            .iter()
            .map(|f| Item {
                path: f.name().replace('\\', "/"),
                size: f.size,
                packed_size: f.packed_size(),
                crc: f.crc(),
                is_dir: f.is_dir(),
                modified: f.modified(),
                enc: f.zip_verify().map(|(verify, verify_byte)| Enc {
                    method: f.encrypt_method().unwrap_or(0),
                    verify,
                    verify_byte,
                }),
                blocks: f
                    .blocks
                    .iter()
                    .map(|b| BlockRef {
                        method: b.method,
                        data_offset: b.data_offset,
                        packed_size: b.packed_size as u64,
                        unpacked_size: b.unpacked_size as u64,
                        crc: b.crc,
                    })
                    .collect(),
            })
            .collect())
    } else if alz::sniff(data) {
        let ar = alz::parse(data)?;
        if ar.is_spanned {
            return Err(ZipManiaError::new(
                "unsupported",
                "분할된 ALZ 아카이브는 아직 지원하지 않습니다.",
            ));
        }
        Ok(ar
            .files
            .iter()
            .map(|f| Item {
                path: f.name().replace('\\', "/"),
                size: f.unpacked_size,
                packed_size: f.packed_size,
                crc: Some(f.crc),
                is_dir: f.is_dir(),
                modified: f.modified(),
                enc: if f.is_encrypted() {
                    Some(Enc {
                        method: 0, // ALZ 는 ZipCrypto 고정(ALZFormat::PreprocessDecrypt)
                        verify: f.verify_data.clone(),
                        verify_byte: f.zip_verify_byte(),
                    })
                } else {
                    None
                },
                blocks: if f.is_dir() {
                    Vec::new()
                } else {
                    vec![BlockRef {
                        method: f.method,
                        data_offset: f.data_offset,
                        packed_size: f.packed_size,
                        unpacked_size: f.unpacked_size,
                        crc: f.crc,
                    }]
                },
            })
            .collect())
    } else {
        Err(ZipManiaError::new(
            "unsupported",
            "EGG/ALZ 아카이브가 아닙니다.",
        ))
    }
}

/// 항목 1개 → 블록 단위 해제 → sink 스트리밍, 반환 = 기록 바이트, Vec 수집 금지
/// limit != 0 이면 실제 출력 바이트로 판정, (D3.5)
fn read_item_to(
    data: &[u8],
    item: &Item,
    password: Option<&str>,
    limit: u64,
    sink: &mut dyn std::io::Write,
) -> Result<u64, ZipManiaError> {
    // 암호 아카이브 = 키 선생성, 키 상태는 블록을 거치며 연속
    let mut cipher = match &item.enc {
        None => None,
        Some(enc) if enc.method != 0 => {
            return Err(ZipManiaError::new(
                "unsupported",
                "AES 로 암호화된 EGG 는 아직 지원하지 않습니다(검증할 샘플이 없습니다).",
            ))
        }
        Some(enc) => {
            let Some(pw) = password else {
                return Err(crypto::password_required());
            };
            let mut z = crypto::ZipCrypto::new(pw.as_bytes());
            // 1차 판정, 오답도 1/256 통과 → 최종 판정은 CRC
            if !z.check_header(&enc.verify, enc.verify_byte) {
                return Err(crypto::wrong_password());
            }
            Some(z)
        }
    };

    let mut written: u64 = 0;
    for b in &item.blocks {
        let header = b.method.data_header_size();
        let start = b
            .data_offset
            .checked_add(header)
            .ok_or_else(|| ZipManiaError::new("corrupt", "블록 오프셋이 잘못되었습니다."))?;
        let end = start
            .checked_add((b.packed_size as usize).saturating_sub(header))
            .ok_or_else(|| ZipManiaError::new("corrupt", "블록 크기가 잘못되었습니다."))?;
        let packed = data
            .get(start..end)
            .ok_or_else(|| ZipManiaError::new("corrupt", "압축 데이터가 잘렸습니다."))?;

        // 암호화 시 복호 → 해제
        let plain: Vec<u8>;
        let packed = if let Some(z) = cipher.as_mut() {
            plain = {
                let mut buf = packed.to_vec();
                z.decrypt(&mut buf);
                buf
            };
            &plain[..]
        } else {
            packed
        };

        // 잔여 예산 전달, 블록마다 상한 재부여 시 항목 전체 상한이 사라짐
        // codec 에서 0 = 무제한 → 예산 소진 시 전달 말고 여기서 중단
        let budget = if limit == 0 {
            0
        } else {
            match limit.checked_sub(written).filter(|r| *r > 0) {
                Some(rest) => rest,
                None => {
                    return Err(ZipManiaError::new(
                        "corrupt",
                        format!("해제 크기 상한을 넘습니다({limit} 이상)"),
                    ))
                }
            }
        };
        // 조각 단위 수신 + CRC 연속 계산 + 스트리밍, 일괄 수신 시 블록 1개 수 GiB 신고에 그대로 당함
        let mut c = Crc32::new();
        let mut block_written: u64 = 0;
        let n = codec::decompress_to(
            b.method,
            packed,
            b.unpacked_size as usize,
            budget,
            &mut |chunk| {
                c.update(chunk);
                block_written += chunk.len() as u64;
                if limit > 0 && written + block_written > limit {
                    return Err(ZipManiaError::new(
                        "corrupt",
                        format!(
                            "해제 크기 상한을 넘습니다({} > {limit})",
                            written + block_written
                        ),
                    ));
                }
                sink.write_all(chunk)
                    .map_err(|e| ZipManiaError::new("io_error", format!("파일을 쓰지 못했습니다: {e}")))
            },
        )?;

        // CRC 0 = 기록 없음 → 미검증, 검증이 쓰기보다 늦으므로 호출측이 되돌릴 수 있어야 함
        // (해제 = StagedFile, 메모리 경로 = 버퍼 폐기)
        if b.crc != 0 && c.finalize() != b.crc {
            // 암호 아카이브 CRC 오류 = 손상보다 오답 비밀번호 우선(검증 헤더 1/256 통과)
            return Err(if item.encrypted() {
                crypto::wrong_password()
            } else {
                ZipManiaError::new("corrupt", format!("CRC 가 일치하지 않습니다: {}", item.path))
            });
        }
        written += n;
    }
    Ok(written)
}

/// 항목 1개 → 메모리(미리보기, 중첩 열기용), 상한 항상 적용 — 없으면 압축 폭탄에 프로세스 소진
fn read_item(data: &[u8], item: &Item, password: Option<&str>) -> Result<Vec<u8>, ZipManiaError> {
    let mut out: Vec<u8> = Vec::with_capacity(
        item.size
            .min(crate::formats::MAX_MEMORY_ENTRY_BYTES.min(64 << 20)) as usize,
    );
    read_item_to(
        data,
        item,
        password,
        crate::formats::MAX_MEMORY_ENTRY_BYTES,
        &mut out,
    )?;
    Ok(out)
}

impl ArchiveBackend for Unegg {
    fn id(&self) -> &'static str {
        "unegg"
    }

    fn read_exts(&self) -> &'static [&'static str] {
        crate::formats::UNEGG_EXTS
    }

    fn write_exts(&self) -> &'static [&'static str] {
        // egg/alz 생성 지원 계획 없음(해제 전용)
        &[]
    }

    fn engine_version(&self) -> Result<String, ZipManiaError> {
        Ok(format!("UnEGG (Rust) {}", env!("CARGO_PKG_VERSION")))
    }

    fn list(&self, archive: &str, _password: Option<&str>) -> Result<Vec<ArchiveEntry>, ZipManiaError> {
        let data = load(archive)?;
        let items = parse_items(&data)?;
        Ok(items
            .into_iter()
            .map(|i| ArchiveEntry {
                path: i.path,
                size: i.size,
                packed_size: i.packed_size,
                modified: i.modified,
                is_dir: i.is_dir,
                crc: i.crc.map(|c| format!("{c:08X}")),
            })
            .collect())
    }

    fn extract(
        &self,
        opts: &ExtractOptions,
        on_progress: &mut ProgressFn<'_>,
        cancel: Arc<AtomicBool>,
    ) -> ExtractResult {
        let data = match load(&opts.archive) {
            Ok(d) => d,
            Err(e) => return ExtractResult::Failed(e),
        };
        let items = match parse_items(&data) {
            Ok(i) => i,
            Err(e) => return ExtractResult::Failed(e),
        };
        extract::extract_all(&data, &items, opts, on_progress, cancel)
    }

    fn create(
        &self,
        _opts: &CreateOptions,
        _on_progress: &mut ProgressFn<'_>,
        _cancel: Arc<AtomicBool>,
    ) -> CreateResult {
        CreateResult::Failed(ZipManiaError::new(
            "unsupported",
            "EGG/ALZ 형식으로 압축하는 기능은 지원하지 않습니다(해제 전용).",
        ))
    }

    fn test(&self, archive: &str, password: Option<&str>) -> Result<(), ZipManiaError> {
        let data = load(archive)?;
        let items = parse_items(&data)?;
        for item in items.iter().filter(|i| !i.is_dir) {
            // 결과 폐기 → 미적재 싱크로 스트리밍, 검사 목적으로 메모리 적재 불필요
            read_item_to(&data, item, password, 0, &mut std::io::sink())?;
        }
        Ok(())
    }

    fn read_entry_to_memory(
        &self,
        archive: &str,
        inner_path: &str,
        password: Option<&str>,
    ) -> Result<Vec<u8>, ZipManiaError> {
        let data = load(archive)?;
        let items = parse_items(&data)?;
        let want = inner_path.replace('\\', "/");
        let item = items
            .iter()
            .find(|i| !i.is_dir && i.path == want)
            .ok_or_else(|| {
                ZipManiaError::new("not_found", "아카이브 안에서 해당 항목을 찾지 못했습니다.")
            })?;
        read_item(&data, item, password)
    }
}
