//! ZipCrypto(PKWARE 전통 암호) 복호, 사용처 = EGG encrypt method=0, ALZ 전체
//! AES(EGG method 1/2) 미구현, (D3.9)

use crate::error::ZipManiaError;

/// CRC-32(reflected 0xEDB88320) 테이블, 키 갱신용
fn crc_table() -> &'static [u32; 256] {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, e) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            *e = c;
        }
        t
    })
}

/// 32비트 키 3개 보유 스트림 복호기, 호출 순서대로 상태 연속
pub struct ZipCrypto {
    k0: u32,
    k1: u32,
    k2: u32,
}

impl ZipCrypto {
    /// 비밀번호로 키 초기화, 초기값 = 규격 상수
    pub fn new(password: &[u8]) -> Self {
        let mut z = ZipCrypto {
            k0: 0x1234_5678,
            k1: 0x2345_6789,
            k2: 0x3456_7890,
        };
        for &b in password {
            z.update(b);
        }
        z
    }

    fn update(&mut self, b: u8) {
        let t = crc_table();
        self.k0 = t[((self.k0 ^ b as u32) & 0xFF) as usize] ^ (self.k0 >> 8);
        self.k1 = self.k1.wrapping_add(self.k0 & 0xFF);
        self.k1 = self.k1.wrapping_mul(134_775_813).wrapping_add(1);
        self.k2 = t[((self.k2 ^ (self.k1 >> 24)) & 0xFF) as usize] ^ (self.k2 >> 8);
    }

    /// 버퍼 제자리 복호
    pub fn decrypt(&mut self, data: &mut [u8]) {
        for c in data.iter_mut() {
            let t = (self.k2 | 2) & 0xFFFF;
            let key = ((t.wrapping_mul(t ^ 1)) >> 8) as u8;
            *c ^= key;
            let p = *c;
            self.update(p);
        }
    }

    /// 12바이트 검증 헤더, 오답도 1/256 통과 → 최종 판정은 CRC
    pub fn check_header(&mut self, header: &[u8], verify_byte: u8) -> bool {
        if header.len() < 12 {
            return false;
        }
        let mut buf = [0u8; 12];
        buf.copy_from_slice(&header[..12]);
        self.decrypt(&mut buf);
        buf[11] == verify_byte
    }
}

/// 암호 필요 + 미제공
pub fn password_required() -> ZipManiaError {
    ZipManiaError::new("password_required", "암호가 필요한 아카이브입니다.")
}

/// 검증 헤더 불일치
pub fn wrong_password() -> ZipManiaError {
    ZipManiaError::new("wrong_password", "암호가 올바르지 않습니다.")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 같은 키 상태에서 복호 = 암호화의 역, 왕복으로 알고리즘 정합성 검사
    fn encrypt(password: &[u8], plain: &[u8]) -> Vec<u8> {
        let mut z = ZipCrypto::new(password);
        let mut out = Vec::with_capacity(plain.len());
        for &p in plain {
            let t = (z.k2 | 2) & 0xFFFF;
            let key = ((t.wrapping_mul(t ^ 1)) >> 8) as u8;
            out.push(p ^ key);
            z.update(p);
        }
        out
    }

    #[test]
    fn 암복호_왕복() {
        let plain = b"ZipMania ZipCrypto \xed\x85\x8c\xec\x8a\xa4\xed\x8a\xb8";
        let cipher = encrypt(b"test", plain);
        assert_ne!(&cipher[..], &plain[..]);
        let mut buf = cipher.clone();
        ZipCrypto::new(b"test").decrypt(&mut buf);
        assert_eq!(buf, plain);
    }

    #[test]
    fn 다른_비밀번호는_다른_결과() {
        let cipher = encrypt(b"test", b"hello");
        let mut buf = cipher.clone();
        ZipCrypto::new(b"wrong").decrypt(&mut buf);
        assert_ne!(buf, b"hello");
    }

    #[test]
    fn 검증_헤더_판정() {
        // 마지막 바이트 0xAB 인 평문 12바이트 암호화 → 같은 암호로 통과해야 함
        let mut plain = [0u8; 12];
        plain[11] = 0xAB;
        let header = encrypt(b"test", &plain);
        assert!(ZipCrypto::new(b"test").check_header(&header, 0xAB));
        assert!(!ZipCrypto::new(b"test").check_header(&header, 0x01));
        assert!(!ZipCrypto::new(b"other").check_header(&header, 0xAB));
        // 12바이트 미만 = 판정 불가
        assert!(!ZipCrypto::new(b"test").check_header(&header[..5], 0xAB));
    }
}
