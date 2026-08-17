//! 오류 타입 + COM 결과(HRESULT, opResult) → 사용자 메시지 변환
//! SevenZipError = Rust 내부용, ZipManiaError = UI 직렬화용(기계용 code + 한국어 message)
//!
//! HRESULT 단독 판정 금지, opResult 와 암호 요청 여부까지 함께 확인 (D6.5)

use serde::Serialize;
use thiserror::Error;

/// Rust 내부 오류, DLL 로드 실패 또는 배포 리소스 경로 해석 실패
#[allow(dead_code)]
#[derive(Debug, Error)]
pub enum SevenZipError {
    #[error("7z.dll 로드 실패: {0}")]
    Load(String),

    #[error("리소스 경로 해석 실패: {0}")]
    Resource(String),
}

/// UI 반환용 직렬화 오류, code = 프런트 분기용, message = 그대로 노출
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ZipManiaError {
    pub code: String,
    pub message: String,
}

impl ZipManiaError {
    /// 코드 + 메시지 → 오류 생성
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        ZipManiaError {
            code: code.to_string(),
            message: message.into(),
        }
    }
}

/// 내부 오류 → UI 오류 변환
impl From<SevenZipError> for ZipManiaError {
    fn from(e: SevenZipError) -> Self {
        match e {
            SevenZipError::Load(m) => {
                ZipManiaError::new("spawn_failed", format!("7z 엔진을 불러오지 못했습니다: {m}"))
            }
            SevenZipError::Resource(m) => {
                ZipManiaError::new("resource_error", format!("리소스 경로를 찾지 못했습니다: {m}"))
            }
        }
    }
}

// 자주 쓰는 분류 결과

fn wrong_password() -> ZipManiaError {
    ZipManiaError::new("wrong_password", "암호가 올바르지 않습니다.")
}
fn password_required() -> ZipManiaError {
    ZipManiaError::new("password_required", "이 아카이브는 암호가 필요합니다.")
}
fn unsupported() -> ZipManiaError {
    ZipManiaError::new(
        "unsupported",
        "지원하지 않는 형식이거나 아카이브 파일이 아닙니다.",
    )
}
fn corrupt() -> ZipManiaError {
    ZipManiaError::new("corrupt", "아카이브가 손상되었거나 읽을 수 없습니다.")
}

/// IInArchive::Open 실패 분류
/// crypto_requested(헤더암호) → 암호 유무로 wrong_password, password_required
/// 아니면 S_FALSE 포함 미지원, 비아카이브
pub fn classify_open_failure(
    _hr: i32,
    crypto_requested: bool,
    password_provided: bool,
) -> ZipManiaError {
    if crypto_requested {
        if password_provided {
            wrong_password()
        } else {
            password_required()
        }
    } else {
        unsupported()
    }
}

/// SetOperationResult(opResult) 분류
/// 0=OK 1=UnsupportedMethod 2=DataError 3=CRCError 5=UnexpectedEnd 7=IsNotArc
/// 8=HeadersError 9=WrongPassword(신형)
/// 암호 요청 상태의 DataError, CRCError = 암호 실패 (D6.5)
pub fn classify_operation(
    op_result: i32,
    crypto_requested: bool,
    password_provided: bool,
) -> ZipManiaError {
    let pw_related = op_result == 9 || (crypto_requested && matches!(op_result, 2 | 3));
    if pw_related {
        return if password_provided {
            wrong_password()
        } else {
            password_required()
        };
    }
    match op_result {
        1 | 7 => unsupported(),
        _ => corrupt(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 헤더암호_무암호_open실패면_password_required() {
        // crypto 요청됨(헤더암호) + 암호 미제공
        let e = classify_open_failure(1, true, false);
        assert_eq!(e.code, "password_required");
    }

    #[test]
    fn 헤더암호_틀린암호_open실패면_wrong_password() {
        let e = classify_open_failure(1, true, true);
        assert_eq!(e.code, "wrong_password");
    }

    #[test]
    fn 암호요청없는_open실패는_unsupported() {
        let e = classify_open_failure(0x8000_0000u32 as i32, false, false);
        assert_eq!(e.code, "unsupported");
    }

    #[test]
    fn 해제_데이터오류_암호요청_무암호면_password_required() {
        let e = classify_operation(2, true, false);
        assert_eq!(e.code, "password_required");
    }

    #[test]
    fn 해제_데이터오류_암호요청_암호제공이면_wrong_password() {
        let e = classify_operation(2, true, true);
        assert_eq!(e.code, "wrong_password");
    }

    #[test]
    fn 해제_wrongpassword_코드9는_암호오류() {
        assert_eq!(classify_operation(9, false, false).code, "password_required");
        assert_eq!(classify_operation(9, false, true).code, "wrong_password");
    }

    #[test]
    fn 해제_암호무관_데이터오류는_corrupt() {
        // 암호 요청 없던 DataError → 손상
        assert_eq!(classify_operation(2, false, false).code, "corrupt");
    }

    #[test]
    fn 해제_미지원메서드_비아카이브는_unsupported() {
        assert_eq!(classify_operation(1, false, false).code, "unsupported");
        assert_eq!(classify_operation(7, false, false).code, "unsupported");
    }
}
