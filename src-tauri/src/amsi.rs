//! AMSI 버퍼 검사, AmsiScanBuffer 로 푼 버퍼를 백신에 전달
//! 제공자 없거나 실시간 보호 꺼짐 = AmsiSession::new 가 None
//! 컨텍스트는 검사 스레드 1개에서 순차 재사용

use windows::core::HSTRING;
use windows::Win32::System::Antimalware::{
    AmsiInitialize, AmsiScanBuffer, AmsiUninitialize, HAMSICONTEXT,
};

/// AMSI_RESULT_DETECTED — 이 값 이상 = 위협 판정(AmsiResultIsMalware 와 같은 기준)
const AMSI_RESULT_DETECTED: i32 = 32768;

/// AMSI_RESULT_BLOCKED_BY_ADMIN_START..=END, DETECTED 보다 작아 그냥 두면 clean 이 되므로 error 로 분류
const AMSI_RESULT_BLOCKED_BY_ADMIN_START: i32 = 16384;
const AMSI_RESULT_BLOCKED_BY_ADMIN_END: i32 = 20479;

/// AMSI 컨텍스트 래퍼, 생성 시 초기화 + drop 시 해제
pub struct AmsiSession {
    ctx: HAMSICONTEXT,
}

// 컨텍스트는 검사 스레드로 이동해 그 스레드에서만 사용(단일 스레드 → Send 안전)
unsafe impl Send for AmsiSession {}

impl AmsiSession {
    /// AMSI 초기화, 실패(제공자 없음/미지원) 시 None
    pub fn new(app_name: &str) -> Option<Self> {
        let name = HSTRING::from(app_name);
        // AmsiInitialize 의 결과 = 컨텍스트
        let ctx = unsafe { AmsiInitialize(&name) }.ok()?;
        Some(AmsiSession { ctx })
    }

    /// 버퍼 1개 검사, 반환 = clean | malware | error
    pub fn scan(&self, name: &str, data: &[u8]) -> String {
        if data.is_empty() {
            return "clean".to_string();
        }
        let content = HSTRING::from(name);
        // AmsiScanBuffer 의 반환 = 판정 결과(AMSI_RESULT)
        let result = unsafe {
            AmsiScanBuffer(
                self.ctx,
                data.as_ptr() as *const core::ffi::c_void,
                data.len() as u32,
                &content,
                None,
            )
        };
        match result {
            Ok(r) if r.0 >= AMSI_RESULT_DETECTED => "malware".to_string(),
            Ok(r)
                if (AMSI_RESULT_BLOCKED_BY_ADMIN_START..=AMSI_RESULT_BLOCKED_BY_ADMIN_END)
                    .contains(&r.0) =>
            {
                "error".to_string()
            }
            Ok(_) => "clean".to_string(),
            Err(_) => "error".to_string(),
        }
    }
}

impl Drop for AmsiSession {
    fn drop(&mut self) {
        unsafe { AmsiUninitialize(self.ctx) };
    }
}
