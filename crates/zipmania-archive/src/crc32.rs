//! CRC-32(IEEE 802.3, reflected 0xEDB88320 = 7z kpidCRC 와 동일), 무결성 테스트용
//! CrcWriter = 디스크 미기록 + 누적, 무결성 테스트 = 널 싱크 + CRC

use std::sync::{Arc, Mutex};

/// 컴파일 타임 생성 CRC-32 룩업 테이블
const fn make_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

static CRC_TABLE: [u32; 256] = make_table();

/// 누적 CRC-32 상태
pub struct Crc32 {
    state: u32,
}

impl Crc32 {
    pub fn new() -> Self {
        Crc32 { state: 0xFFFF_FFFF }
    }

    /// 바이트 슬라이스 누적
    pub fn update(&mut self, buf: &[u8]) {
        let mut c = self.state;
        for &b in buf {
            c = CRC_TABLE[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
        }
        self.state = c;
    }

    /// 최종 CRC-32 값
    pub fn finalize(&self) -> u32 {
        self.state ^ 0xFFFF_FFFF
    }
}

impl Default for Crc32 {
    fn default() -> Self {
        Self::new()
    }
}

/// 해제 데이터 폐기 + CRC-32 누적 writer
pub struct CrcWriter {
    crc: Arc<Mutex<Crc32>>,
}

impl CrcWriter {
    pub fn new(crc: Arc<Mutex<Crc32>>) -> Self {
        CrcWriter { crc }
    }
}

impl std::io::Write for CrcWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Ok(mut c) = self.crc.lock() {
            c.update(buf);
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_known_vectors() {
        // "123456789" → 0xCBF43926(표준 CRC-32 체크값)
        let mut c = Crc32::new();
        c.update(b"123456789");
        assert_eq!(c.finalize(), 0xCBF4_3926);

        // 빈 입력 → 0
        assert_eq!(Crc32::new().finalize(), 0);
    }
}
