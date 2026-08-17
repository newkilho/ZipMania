//! 압축 해제 디스패치, 방식 값이 포맷별로 다름
//! EGG: 0=Store 1=Deflate 2=BZip2 3=AZO 4=LZMA
//! ALZ: 0=Store 1=변형BZip2 2=Deflate
//! 해제 가능 = Store, Deflate 뿐, 나머지 전부 unsupported 거부

use crate::error::ZipManiaError;

/// 블록 1개의 압축 방식
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Store,
    Deflate,
    /// EGG 표준 bzip2
    BZip2,
    /// EGG LZMA(raw LZMA1, 데이터 헤더 9바이트)
    Lzma,
    /// 이스트소프트 독자 알고리즘
    Azo,
    /// ALZ 변형 bzip2(비표준)
    AlzBZip2,
    Unknown(u16),
}

impl Method {
    /// EGG 블록 헤더 compress method 값(nest/nest.h CompressionMethod)
    pub fn from_egg(id: u8) -> Method {
        match id {
            0 => Method::Store,
            1 => Method::Deflate,
            2 => Method::BZip2,
            3 => Method::Azo,
            4 => Method::Lzma,
            other => Method::Unknown(other as u16),
        }
    }

    /// ALZ 파일 헤더 compression method 값
    pub fn from_alz(id: u16) -> Method {
        match id {
            0 => Method::Store,
            1 => Method::AlzBZip2,
            2 => Method::Deflate,
            other => Method::Unknown(other),
        }
    }

    /// UI 표시용 이름
    pub fn name(self) -> &'static str {
        match self {
            Method::Store => "STORE",
            Method::Deflate => "DEFLATE",
            Method::BZip2 => "BZIP2",
            Method::Lzma => "LZMA",
            Method::Azo => "AZO",
            Method::AlzBZip2 => "ALZ-BZIP2",
            Method::Unknown(_) => "UNKNOWN",
        }
    }

    /// 압축 데이터 앞 알고리즘 전용 헤더 크기, LZMA 만 9
    pub fn data_header_size(self) -> usize {
        match self {
            Method::Lzma => 9,
            _ => 0,
        }
    }

    /// 현재 해제 가능 방식인가
    pub fn is_supported(self) -> bool {
        matches!(self, Method::Store | Method::Deflate)
    }
}

/// 블록 1개 해제 → out 스트리밍, 반환 = 출력 바이트
/// data = 알고리즘 헤더 제외 압축 바이트열, expected = 신고 크기, limit != 0 이면 출력 상한
/// Vec 반환 금지, CRC 선검증 불가 대신 호출측이 되돌림, (D3.5)
pub fn decompress_to(
    method: Method,
    data: &[u8],
    expected: usize,
    limit: u64,
    out: &mut dyn FnMut(&[u8]) -> Result<(), ZipManiaError>,
) -> Result<u64, ZipManiaError> {
    if limit > 0 && expected as u64 > limit {
        return Err(ZipManiaError::new(
            "corrupt",
            format!("해제 크기 상한을 넘습니다({expected} > {limit})"),
        ));
    }

    match method {
        // 길이 부족 = 손상, min 으로 뭉개면 잘린 아카이브가 성공으로 나감
        Method::Store => {
            if data.len() < expected {
                return Err(ZipManiaError::new(
                    "corrupt",
                    format!("저장 블록이 짧습니다({} < {expected})", data.len()),
                ));
            }
            out(&data[..expected])?;
            Ok(expected as u64)
        }

        Method::Deflate => inflate_to(data, expected, limit, out),

        Method::BZip2 | Method::Lzma => Err(ZipManiaError::new(
            "unsupported",
            format!("{} 압축은 아직 지원하지 않습니다.", method.name()),
        )),

        Method::Azo => Err(ZipManiaError::new(
            "unsupported",
            "AZO 압축은 아직 지원하지 않습니다(이스트소프트 독자 알고리즘).",
        )),

        Method::AlzBZip2 => Err(ZipManiaError::new(
            "unsupported",
            "ALZ 의 변형 bzip2(method=1)는 지원하지 않습니다. 표준 bzip2 가 아니라 전용 디코더가 필요합니다.",
        )),

        Method::Unknown(id) => Err(ZipManiaError::new(
            "unsupported",
            format!("알 수 없는 압축 방식입니다(id={id})."),
        )),
    }
}

/// raw deflate 조각 단위 해제(zlib 헤더 없음, miniz_oxide 스트리밍)
/// 종료 표시 없이 끝나도 신고 크기만큼 나왔으면 정상(알집 스트림이 그럼)
fn inflate_to(
    data: &[u8],
    expected: usize,
    limit: u64,
    out: &mut dyn FnMut(&[u8]) -> Result<(), ZipManiaError>,
) -> Result<u64, ZipManiaError> {
    use miniz_oxide::inflate::stream::{inflate, InflateState};
    use miniz_oxide::{DataFormat, MZFlush, MZStatus};

    // 신고 크기 있으면 그 값(초과 = 거짓말), 없으면 상한
    let cap: u64 = match (expected, limit) {
        (0, 0) => crate::formats::MAX_MEMORY_ENTRY_BYTES,
        (0, l) => l,
        (e, _) => e as u64,
    };

    let mut state = InflateState::new_boxed(DataFormat::Raw);
    let mut buf = vec![0u8; 64 * 1024];
    let mut input = data;
    let mut written: u64 = 0;

    loop {
        let r = inflate(&mut state, input, &mut buf, MZFlush::None);
        input = &input[r.bytes_consumed..];
        if r.bytes_written > 0 {
            written += r.bytes_written as u64;
            if written > cap {
                return Err(ZipManiaError::new(
                    "corrupt",
                    format!("해제 크기가 신고값을 넘습니다({written} > {cap})"),
                ));
            }
            out(&buf[..r.bytes_written])?;
        }
        match r.status {
            Ok(MZStatus::StreamEnd) => break,
            Ok(_) => {
                // 진전, 입력 모두 없음 = 스트림 끊김, 신고량만큼 나왔으면 정상
                if r.bytes_consumed == 0 && r.bytes_written == 0 {
                    break;
                }
            }
            Err(e) => {
                return Err(ZipManiaError::new(
                    "corrupt",
                    format!("압축 해제에 실패했습니다({e:?})"),
                ))
            }
        }
    }

    // 신고 크기 != 실제 = 손상(Store 블록과 동일 규칙)
    if expected > 0 && written != expected as u64 {
        return Err(ZipManiaError::new(
            "corrupt",
            format!("해제 크기가 신고값과 다릅니다({written} != {expected})"),
        ));
    }
    Ok(written)
}

/// 메모리 반환판, 테스트, 짧은 블록 전용
#[cfg(test)]
fn decompress(
    method: Method,
    data: &[u8],
    expected: usize,
    limit: u64,
) -> Result<Vec<u8>, ZipManiaError> {
    let mut out = Vec::new();
    decompress_to(method, data, expected, limit, &mut |c| {
        out.extend_from_slice(c);
        Ok(())
    })?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 메서드_매핑() {
        assert_eq!(Method::from_egg(1), Method::Deflate);
        assert_eq!(Method::from_egg(2), Method::BZip2);
        // ALZ 의 1, 2 는 EGG 와 의미 다름, 혼동 시 데이터 손상
        assert_eq!(Method::from_alz(1), Method::AlzBZip2);
        assert_eq!(Method::from_alz(2), Method::Deflate);
        assert_eq!(Method::from_egg(4).data_header_size(), 9);
    }

    #[test]
    fn 미지원은_조용히_통과하지_않는다() {
        for m in [Method::Azo, Method::AlzBZip2, Method::BZip2, Method::Lzma, Method::Unknown(9)] {
            let e = decompress(m, b"", 0, 0).unwrap_err();
            assert_eq!(e.code, "unsupported", "{m:?} 가 오류를 내지 않았다");
        }
    }

    #[test]
    fn store_와_deflate_왕복() {
        assert_eq!(decompress(Method::Store, b"abc", 3, 0).unwrap(), b"abc");
        let packed = miniz_oxide::deflate::compress_to_vec(b"hello hello hello", 6);
        let out = decompress(Method::Deflate, &packed, 17, 0).unwrap();
        assert_eq!(out, b"hello hello hello");
    }

    #[test]
    fn 상한을_넘으면_거부한다() {
        let e = decompress(Method::Store, b"abc", 100, 10).unwrap_err();
        assert_eq!(e.code, "corrupt");
    }

    /// 저장 블록 < 신고 길이 = 손상, min 으로 뭉개면 잘린 파일이 성공으로 나감
    #[test]
    fn 저장_블록이_짧으면_손상이다() {
        let e = decompress(Method::Store, b"abc", 10, 0).unwrap_err();
        assert_eq!(e.code, "corrupt", "{}", e.message);
        // 길이 일치 → 통과
        assert_eq!(decompress(Method::Store, b"abcde", 5, 0).unwrap(), b"abcde");
        // 뒤에 여분 있으면 신고 길이까지만 사용
        assert_eq!(decompress(Method::Store, b"abcdeXYZ", 5, 0).unwrap(), b"abcde");
    }
}
