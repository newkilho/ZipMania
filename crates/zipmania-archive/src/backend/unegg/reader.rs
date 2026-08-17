//! 바이트 슬라이스 리더, EGG/ALZ 정수 = 전부 리틀엔디안, 변환은 이 파일만 경유(호스트 엔디안 무관)
//! 끝 초과 읽기 = 패닉 아닌 오류

use crate::error::ZipManiaError;

/// 위치 보유 읽기 전용 커서
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

fn truncated(need: usize, pos: usize) -> ZipManiaError {
    ZipManiaError::new(
        "corrupt",
        format!("아카이브가 잘렸습니다(오프셋 {pos} 에서 {need}바이트 필요)"),
    )
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Reader { data, pos: 0 }
    }

    /// 지정 위치에서 시작하는 리더
    pub fn at(data: &'a [u8], pos: usize) -> Self {
        Reader {
            data,
            pos: pos.min(data.len()),
        }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    pub fn seek(&mut self, pos: usize) {
        self.pos = pos.min(self.data.len());
    }

    /// n 바이트 읽어 슬라이스 반환
    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8], ZipManiaError> {
        if self.remaining() < n {
            return Err(truncated(n, self.pos));
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    /// n 바이트 건너뜀
    pub fn skip(&mut self, n: usize) -> Result<(), ZipManiaError> {
        if self.remaining() < n {
            return Err(truncated(n, self.pos));
        }
        self.pos += n;
        Ok(())
    }

    pub fn u8(&mut self) -> Result<u8, ZipManiaError> {
        Ok(self.bytes(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16, ZipManiaError> {
        let b = self.bytes(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub fn u32(&mut self) -> Result<u32, ZipManiaError> {
        let b = self.bytes(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn u64(&mut self) -> Result<u64, ZipManiaError> {
        let b = self.bytes(8)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    /// 위치 이동 없이 u32 미리보기, 잔여 바이트 부족 시 None
    pub fn peek_u32(&self) -> Option<u32> {
        if self.remaining() < 4 {
            return None;
        }
        let b = &self.data[self.pos..self.pos + 4];
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// 현재 위치 + off 의 u32 미리보기, 경계 초과 시 None
    pub fn peek_u32_at(&self, off: usize) -> Option<u32> {
        let p = self.pos.checked_add(off)?;
        if self.data.len().checked_sub(p)? < 4 {
            return None;
        }
        let b = &self.data[p..p + 4];
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// 가변 폭 정수(ALZ 크기 필드: 1/2/4/8 바이트)
    pub fn var_uint(&mut self, width: u8) -> Result<u64, ZipManiaError> {
        Ok(match width {
            1 => self.u8()? as u64,
            2 => self.u16()? as u64,
            4 => self.u32()? as u64,
            8 => self.u64()?,
            other => {
                return Err(ZipManiaError::new(
                    "corrupt",
                    format!("잘못된 크기 필드 폭: {other}"),
                ))
            }
        })
    }
}

/// 슬라이스 선두의 리틀엔디안 u32 읽기, 시그니처 판별용
pub fn peek_sig(data: &[u8]) -> Option<u32> {
    if data.len() < 4 {
        return None;
    }
    Some(u32::from_le_bytes([data[0], data[1], data[2], data[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 리틀엔디안으로_읽는다() {
        let mut r = Reader::new(&[0x45, 0x47, 0x47, 0x41, 0x00, 0x01]);
        assert_eq!(r.u32().unwrap(), 0x41474745); // "EGGA"
        assert_eq!(r.u16().unwrap(), 0x0100);
        assert_eq!(r.remaining(), 0);
    }

    #[test]
    fn 끝을_넘으면_패닉이_아니라_오류다() {
        let mut r = Reader::new(&[1, 2]);
        assert_eq!(r.u32().unwrap_err().code, "corrupt");
        assert!(r.peek_u32().is_none());
    }

    #[test]
    fn 가변폭_정수() {
        let mut r = Reader::new(&[0x01, 0x02, 0x00, 0x03, 0x00, 0x00, 0x00]);
        assert_eq!(r.var_uint(1).unwrap(), 1);
        assert_eq!(r.var_uint(2).unwrap(), 2);
        assert_eq!(r.var_uint(4).unwrap(), 3);
        assert!(Reader::new(&[0]).var_uint(3).is_err());
    }
}
