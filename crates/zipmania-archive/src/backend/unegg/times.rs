//! 시각 변환, 입력 3종 → 출력 YYYY-MM-DD HH:MM:SS(7z 백엔드와 동일 형식)
//! EGG WINDOWS_FILEINFO(FILETIME), EGG POSIX_FILEINFO(Unix epoch i64), ALZ 헤더(MS-DOS, 1980 기준 로컬)
//! 외부 의존 없이 직접 변환

/// FILETIME 기점(1601-01-01) ~ Unix epoch(1970-01-01) 사이 100ns 틱 수
const FILETIME_EPOCH_DELTA: i64 = 116_444_736_000_000_000;

/// Unix epoch 초 → YYYY-MM-DD HH:MM:SS, 0 이하, 범위 초과 = 빈 문자열
pub fn unix_to_string(sec: i64) -> String {
    if sec <= 0 {
        return String::new();
    }
    let days = sec.div_euclid(86_400);
    let secs_of_day = sec.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format(
        y,
        m,
        d,
        (secs_of_day / 3600) as u32,
        ((secs_of_day % 3600) / 60) as u32,
        (secs_of_day % 60) as u32,
    )
}

/// Windows FILETIME → 문자열
pub fn filetime_to_string(ft: i64) -> String {
    if ft <= 0 {
        return String::new();
    }
    unix_to_string((ft - FILETIME_EPOCH_DELTA) / 10_000_000)
}

/// MS-DOS date/time(u32) → 문자열, 상위 16비트 = 날짜, 하위 = 시각
/// DOS 시각은 타임존 개념 없음 → 기록 당시 로컬 시각 그대로 표시
pub fn dos_to_string(dt: u32) -> String {
    if dt == 0 {
        return String::new();
    }
    let date = (dt >> 16) & 0xFFFF;
    let time = dt & 0xFFFF;
    let year = 1980 + ((date >> 9) & 0x7F) as i64;
    let month = (date >> 5) & 0x0F;
    let day = date & 0x1F;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return String::new();
    }
    let hour = (time >> 11) & 0x1F;
    let minute = (time >> 5) & 0x3F;
    let second = ((time & 0x1F) * 2).min(59);
    format(year, month, day, hour, minute, second)
}

fn format(y: i64, m: u32, d: u32, hh: u32, mm: u32, ss: u32) -> String {
    format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}:{ss:02}")
}

/// 1970-01-01 기준 일수 → (년, 월, 일), Howard Hinnant civil_from_days
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as i64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filetime_변환() {
        // 실측 샘플 test.egg: 0x01D21282DFFB4300 → 2016-09-19 14:34:06 (UTC)
        assert_eq!(filetime_to_string(0x01D2_1282_DFFB_4300), "2016-09-19 14:34:06");
        assert_eq!(filetime_to_string(0), "");
    }

    #[test]
    fn dos_변환() {
        // 실측 샘플 test.alz: 0x49337443 → 2016-09-19 14:34:06 (로컬)
        assert_eq!(dos_to_string(0x4933_7443), "2016-09-19 14:34:06");
        assert_eq!(dos_to_string(0), "");
    }

    #[test]
    fn unix_변환_경계() {
        assert_eq!(unix_to_string(0), "");
        assert_eq!(unix_to_string(1), "1970-01-01 00:00:01");
        assert_eq!(unix_to_string(951_782_400), "2000-02-29 00:00:00"); // 윤년
    }
}
