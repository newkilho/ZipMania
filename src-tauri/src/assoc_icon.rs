//! 파일 연결 아이콘, 확장자 → exe 아이콘 리소스 ID, 레지스트리 값 = <exe 경로>,-<ID>
//! ID 표 = build.rs 가 icon/ 폴더에서 생성 — 중복 기재 금지(D3.12)

#![allow(dead_code)]

include!(concat!(env!("OUT_DIR"), "/assoc_icon_ids.rs"));

/// 전용 아이콘이 없는 확장자가 쓸 기본 아이콘(파일 이름 = icon/ZipMania.ico)
const DEFAULT_STEM: &str = "zipmania";

/// 파일 이름이 확장자와 다른 경우의 별칭, 확실한 것만
const ALIASES: &[(&str, &str)] = &[
    // zip 컨테이너
    ("jar", "zip"),
    ("cbz", "zip"),
    // rar
    ("cbr", "rar"),
    ("r00", "rar"),
    // 7z
    ("cb7", "7z"),
    // tar 계열, 단일 스트림
    ("ova", "tar"),
    ("taz", "tar"),
    ("tzst", "tar"),
    ("gzip", "gz"),
    ("tpz", "tgz"),
    ("bzip2", "bz2"),
    ("txz", "xz"),
    ("lzma", "xz"),
    // 디스크 이미지
    ("img", "iso"),
    ("udf", "iso"),
    ("dmg", "iso"),
];

/// 확장자 → 아이콘 리소스 ID, 없으면 기본 아이콘 ID
pub fn icon_id(ext: &str) -> u16 {
    let ext = ext.trim_start_matches('.').to_lowercase();
    let stem = ALIASES
        .iter()
        .find(|(from, _)| *from == ext)
        .map(|(_, to)| *to)
        .unwrap_or(ext.as_str());

    lookup(stem).or_else(|| lookup(DEFAULT_STEM)).unwrap_or(0)
}

fn lookup(stem: &str) -> Option<u16> {
    ICON_IDS.iter().find(|(name, _)| *name == stem).map(|(_, id)| *id)
}

/// 레지스트리 DefaultIcon 에 넣을 값("C:\…\ZipMania.exe,-107")
pub fn icon_ref(exe: &str, ext: &str) -> String {
    format!("{exe},-{}", icon_id(ext))
}

#[cfg(test)]
mod tests {
    use super::{icon_id, icon_ref, lookup, ALIASES, DEFAULT_STEM, ICON_IDS};

    /// 기본 아이콘 부재 시 전용 아이콘 없는 확장자가 통째로 빈 아이콘
    #[test]
    fn 기본_아이콘이_박혀_있다() {
        assert!(lookup(DEFAULT_STEM).is_some(), "icon/ZipMania.ico 가 없다");
    }

    /// 같은 ID 를 두 아이콘이 쓰면 리소스 컴파일 자체는 통과하고 그림만 엉뚱해진다
    #[test]
    fn 리소스_id_가_겹치지_않는다() {
        let mut ids: Vec<u16> = ICON_IDS.iter().map(|(_, id)| *id).collect();
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n, "아이콘 리소스 ID 가 겹친다");
        // 앱 아이콘(32512)보다 커야 exe 가 제 아이콘으로 보인다(가장 작은 번호가 exe 아이콘)
        assert!(ids.first().map(|id| *id > 32512).unwrap_or(true), "앱 아이콘보다 앞선다");
    }

    /// 별칭 없는 아이콘 지정 시 조용히 기본 아이콘으로 낙착 — 의도한 그림 미출력
    #[test]
    fn 별칭은_실제_아이콘을_가리킨다() {
        for (from, to) in ALIASES {
            assert!(lookup(to).is_some(), "{from} → {to}: icon/{to}.ico 가 없다");
            assert!(lookup(from).is_none(), "{from} 은 전용 아이콘이 있으니 별칭이 필요 없다");
        }
    }

    /// 지원 확장자는 전부 그리기 가능해야 함(전용이든 기본이든)
    #[test]
    fn 모든_지원_확장자가_아이콘을_얻는다() {
        for ext in zipmania_archive::READ_EXTS {
            assert!(icon_id(ext) > 0, "{ext} 에 줄 아이콘이 없다");
        }
    }

    #[test]
    fn 참조_형식은_음수_리소스_id() {
        let zip = icon_id("zip");
        assert_eq!(icon_ref(r"C:\a\ZipMania.exe", "zip"), format!(r"C:\a\ZipMania.exe,-{zip}"));
        // 점, 대문자가 붙어 와도 같은 아이콘
        assert_eq!(icon_id(".ZIP"), zip);
        // 별칭은 원본과 같은 아이콘, 모르는 확장자는 기본 아이콘
        assert_eq!(icon_id("cbz"), zip);
        assert_eq!(icon_id("모르는것"), lookup(DEFAULT_STEM).unwrap());
    }
}
