//! 목록 수정 시각의 표시 문자열, UTC 기반 시각(FILETIME, Unix epoch) → 컴퓨터 시간대 YYYY-MM-DD HH:MM:SS
//! zip, alz 의 DOS 시각은 기록 당시 로컬이라 여기를 거치지 않음, 세 백엔드가 같은 형식 (D3.18)

use std::sync::OnceLock;

use time::{OffsetDateTime, UtcOffset};

/// FILETIME 기점(1601-01-01) ~ Unix epoch(1970-01-01) 사이 100ns 틱 수
pub const FILETIME_EPOCH_DELTA: i64 = 116_444_736_000_000_000;

/// 프로세스 시간대, 못 얻으면 UTC(Linux 다중 스레드 보호), 한 번만 조회
pub fn local_offset() -> UtcOffset {
    static OFF: OnceLock<UtcOffset> = OnceLock::new();
    *OFF.get_or_init(|| UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC))
}

/// Unix epoch 초 → 로컬 YYYY-MM-DD HH:MM:SS, 0 이하 또는 범위 밖 = 빈 문자열
pub fn unix_to_local_string(sec: i64) -> String {
    if sec <= 0 {
        return String::new();
    }
    let Ok(t) = OffsetDateTime::from_unix_timestamp(sec) else {
        return String::new();
    };
    let t = t.to_offset(local_offset());
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        t.year(),
        t.month() as u8,
        t.day(),
        t.hour(),
        t.minute(),
        t.second()
    )
}

/// Windows FILETIME(100ns, 1601 기점) → 로컬 문자열, 0 이하 = 빈 문자열
pub fn filetime_to_local_string(ft: i64) -> String {
    if ft <= 0 {
        return String::new();
    }
    unix_to_local_string((ft - FILETIME_EPOCH_DELTA) / 10_000_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 기대값 = UTC 문자열을 프로세스 시간대만큼 옮긴 것, 시간대가 UTC 인 환경에서도 성립
    fn expect_local(unix: i64) -> String {
        let t = OffsetDateTime::from_unix_timestamp(unix).unwrap().to_offset(local_offset());
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            t.year(),
            t.month() as u8,
            t.day(),
            t.hour(),
            t.minute(),
            t.second()
        )
    }

    #[test]
    fn 로컬_시간대로_표시한다() {
        // 2026-07-24 06:02:37 UTC
        let unix: i64 = 1_784_872_957;
        let s = unix_to_local_string(unix);
        assert_eq!(s, expect_local(unix));
        // UTC 와의 차 = 시간대 오프셋(분)
        let utc = OffsetDateTime::from_unix_timestamp(unix).unwrap();
        let local = utc.to_offset(local_offset());
        let diff = (local.hour() as i32 * 60 + local.minute() as i32) - (utc.hour() as i32 * 60 + utc.minute() as i32);
        let off = local_offset().whole_minutes() as i32;
        assert!((diff - off).rem_euclid(1440) == 0, "diff={diff} off={off}");

        let ft = (unix + 11_644_473_600) * 10_000_000;
        assert_eq!(filetime_to_local_string(ft), s);
        assert_eq!(filetime_to_local_string(0), "");
        assert_eq!(unix_to_local_string(0), "");
        assert_eq!(unix_to_local_string(-5), "");
    }
}
