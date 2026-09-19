//! 시각 변환, 입력 3종 → 출력 YYYY-MM-DD HH:MM:SS(7z 백엔드와 동일 형식)
//! EGG WINDOWS_FILEINFO(FILETIME), EGG POSIX_FILEINFO(Unix epoch i64), ALZ 헤더(MS-DOS, 1980 기준 로컬)
//! FILETIME, Unix epoch 는 crate::times 로 로컬 시간대 변환, DOS 는 기록 당시 로컬 그대로

/// Unix epoch 초 → 로컬 YYYY-MM-DD HH:MM:SS, 0 이하 = 빈 문자열
pub fn unix_to_string(sec: i64) -> String {
    crate::times::unix_to_local_string(sec)
}

/// Windows FILETIME → 로컬 문자열
pub fn filetime_to_string(ft: i64) -> String {
    crate::times::filetime_to_local_string(ft)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filetime_변환() {
        // 실측 샘플 test.egg: 0x01D21282DFFB4300 = 2016-09-19 14:34:06 UTC → 컴퓨터 시간대로
        let unix = (0x01D2_1282_DFFB_4300_i64 - crate::times::FILETIME_EPOCH_DELTA) / 10_000_000;
        assert_eq!(filetime_to_string(0x01D2_1282_DFFB_4300), crate::times::unix_to_local_string(unix));
        assert!(filetime_to_string(0x01D2_1282_DFFB_4300).starts_with("2016-09-"));
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
        // 로컬 시간대로 옮기므로 기대값도 같은 길로, 윤년 일자가 날짜 범위 안인지만
        assert_eq!(unix_to_string(1), crate::times::unix_to_local_string(1));
        let leap = unix_to_string(951_782_400 + 43_200); // 2000-02-29 12:00 UTC, 어느 시간대도 같은 날
        assert!(leap.starts_with("2000-02-29 "), "{leap}");
    }
}
