//! PROPVARIANT 읽기/쓰기, 정리, 대상 타입 = BSTR/BOOL/UI4/UI8/FILETIME 만
//! 읽기: as_* → clear() 필수, 쓰기: set_* 후 clear 금지, (D6.5)

use std::ffi::c_void;

use windows_core::BSTR;

// PROPVARIANT 정리 = ole32 PropVariantClear(BSTR 등 내부 할당 해제)
#[link(name = "ole32")]
extern "system" {
    fn PropVariantClear(pvar: *mut c_void) -> i32;
}

// 사용하는 VARIANT 타입 태그
const VT_EMPTY: u16 = 0;
const VT_BSTR: u16 = 8;
const VT_BOOL: u16 = 11;
const VT_UI4: u16 = 19;
const VT_UI8: u16 = 21;
const VT_FILETIME: u16 = 64;

/// COM PROPVARIANT 최소 표현, 크기 24바이트 필수(패딩 포함), (D6.5)
#[repr(C)]
pub struct PropVariant {
    vt: u16,
    _r1: u16,
    _r2: u16,
    _r3: u16,
    val: [u8; 8],
    _tail: u64,
}

// PROPVARIANT 크기 고정: 24바이트, 8정렬
const _: () = assert!(std::mem::size_of::<PropVariant>() == 24);

impl PropVariant {
    /// 빈 값(VT_EMPTY) 생성
    pub fn empty() -> Self {
        PropVariant {
            vt: VT_EMPTY,
            _r1: 0,
            _r2: 0,
            _r3: 0,
            val: [0; 8],
            _tail: 0,
        }
    }

    /// 원시 포인터 전달용 자기 가변 포인터
    pub fn as_mut_ptr(&mut self) -> *mut c_void {
        self as *mut PropVariant as *mut c_void
    }

    // ── 읽기 ──

    /// 값 없음 != 형 불일치, 값 없음 = 정상(.bz2 는 이름 미보유), 형 불일치 = 미이해
    /// 검사 보고에서 구분 필요
    pub fn is_empty(&self) -> bool {
        self.vt == VT_EMPTY
    }

    /// 정수 값(UI4/UI8) → u64
    pub fn as_u64(&self) -> Option<u64> {
        match self.vt {
            VT_UI8 => Some(u64::from_ne_bytes(self.val)),
            VT_UI4 => Some(
                u32::from_ne_bytes([self.val[0], self.val[1], self.val[2], self.val[3]]) as u64,
            ),
            _ => None,
        }
    }

    /// UI4 값 → u32(CRC 등)
    pub fn as_u32(&self) -> Option<u32> {
        if self.vt == VT_UI4 {
            Some(u32::from_ne_bytes([
                self.val[0],
                self.val[1],
                self.val[2],
                self.val[3],
            ]))
        } else {
            None
        }
    }

    /// BOOL 값 읽기, VARIANT_BOOL: 0=false, 그 외 true
    pub fn as_bool(&self) -> Option<bool> {
        if self.vt == VT_BOOL {
            Some(i16::from_ne_bytes([self.val[0], self.val[1]]) != 0)
        } else {
            None
        }
    }

    /// FILETIME(1601 기준 100ns) 원시 u64 읽기
    pub fn as_filetime(&self) -> Option<u64> {
        if self.vt == VT_FILETIME {
            Some(u64::from_ne_bytes(self.val))
        } else {
            None
        }
    }

    /// BSTR 문자열 읽기(UTF-16 → String)
    pub fn as_string(&self) -> Option<String> {
        if self.vt != VT_BSTR {
            return None;
        }
        let ptr = u64::from_ne_bytes(self.val) as *const u16;
        if ptr.is_null() {
            return Some(String::new());
        }
        unsafe {
            let mut len = 0usize;
            while *ptr.add(len) != 0 {
                len += 1;
            }
            let slice = std::slice::from_raw_parts(ptr, len);
            Some(String::from_utf16_lossy(slice))
        }
    }

    /// 읽기 경로에서 값 회수 후 호출 필수, 미호출 = BSTR 누수
    pub fn clear(&mut self) {
        unsafe {
            let _ = PropVariantClear(self.as_mut_ptr());
        }
    }

    // ── 쓰기(생성 콜백에서 채움 — 소유권은 7z 로 이관) ──

    /// VT_EMPTY 설정
    pub fn set_empty(&mut self) {
        self.vt = VT_EMPTY;
        self.val = [0; 8];
        self._tail = 0;
    }

    /// VARIANT_BOOL 설정, true = -1
    pub fn set_bool(&mut self, v: bool) {
        self.vt = VT_BOOL;
        let x: i16 = if v { -1 } else { 0 };
        let b = x.to_ne_bytes();
        self.val = [b[0], b[1], 0, 0, 0, 0, 0, 0];
    }

    /// UI4 설정
    pub fn set_u32(&mut self, v: u32) {
        self.vt = VT_UI4;
        let b = v.to_ne_bytes();
        self.val = [b[0], b[1], b[2], b[3], 0, 0, 0, 0];
    }

    /// UI8 설정
    pub fn set_u64(&mut self, v: u64) {
        self.vt = VT_UI8;
        self.val = v.to_ne_bytes();
    }

    /// FILETIME 설정(1601 기준 100ns 원시값)
    pub fn set_filetime(&mut self, v: u64) {
        self.vt = VT_FILETIME;
        self.val = v.to_ne_bytes();
    }

    /// 할당한 BSTR 소유권 → 7z 로 이관, 7z 가 PropVariantClear 수행
    pub fn set_bstr(&mut self, s: &str) {
        self.vt = VT_BSTR;
        let b = BSTR::from(s);
        let ptr = b.into_raw();
        self.val = (ptr as usize as u64).to_ne_bytes();
    }
}

/// FILETIME → YYYY-MM-DD HH:MM:SS(UTC), 0 또는 범위 밖 = 빈 문자열
pub fn filetime_to_string(ft: u64) -> String {
    if ft == 0 {
        return String::new();
    }
    // 100ns → 초, 1601→1970 오프셋 보정
    let secs_since_1601 = (ft / 10_000_000) as i64;
    let unix = secs_since_1601 - 11_644_473_600;

    let days = unix.div_euclid(86_400);
    let secs_of_day = unix.rem_euclid(86_400);
    let (h, mi, s) = (
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60,
    );

    // Howard Hinnant civil_from_days
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = y + if m <= 2 { 1 } else { 0 };

    format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 정수_불리언_왕복() {
        let mut p = PropVariant::empty();
        p.set_u64(1234567890123);
        assert_eq!(p.as_u64(), Some(1234567890123));

        let mut p = PropVariant::empty();
        p.set_u32(0x93F7B375);
        assert_eq!(p.as_u32(), Some(0x93F7B375));
        assert_eq!(p.as_u64(), Some(0x93F7B375));

        let mut p = PropVariant::empty();
        p.set_bool(true);
        assert_eq!(p.as_bool(), Some(true));
        p.set_bool(false);
        assert_eq!(p.as_bool(), Some(false));
    }

    #[test]
    fn 빈값은_none() {
        let p = PropVariant::empty();
        assert_eq!(p.as_u64(), None);
        assert_eq!(p.as_bool(), None);
        assert_eq!(p.as_string(), None);
    }

    #[test]
    fn filetime_문자열_변환() {
        // 2026-07-24 06:02:37 UTC 의 FILETIME
        // unix = 1784872957 → filetime = (unix + 11644473600) * 10^7
        let unix: u64 = 1_784_872_957;
        let ft = (unix + 11_644_473_600) * 10_000_000;
        assert_eq!(filetime_to_string(ft), "2026-07-24 06:02:37");
        assert_eq!(filetime_to_string(0), "");
    }
}
