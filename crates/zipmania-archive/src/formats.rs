//! 포맷과 옵션 정의, 플랫폼 무관 — OS 의존 코드 금지(windows-core 금지)
//! 소비자 참조 타입과 상수 = 백엔드 없는 플랫폼에서도 컴파일 가능해야 함 (D3.5)

/// 대상 파일 존재 시 정책, UI 의 묻기는 해제 전 확정 → 여기 미도달
/// 파일별 지정 = ExtractOptions::decisions 의 경로별 값
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverwriteMode {
    Overwrite,
    Skip,
    Rename,
}

impl OverwriteMode {
    /// 알 수 없는 문자열 → Skip(기존 파일 미변경 쪽)
    pub fn from_str(s: &str) -> Self {
        match s {
            "overwrite" => OverwriteMode::Overwrite,
            "rename" => OverwriteMode::Rename,
            _ => OverwriteMode::Skip,
        }
    }
}

/// 겹치지 않는 이름 생성 = 문서.txt → 문서 (2).txt → (3), 확장자 보존, 상한 존재
pub fn unique_path(target: &std::path::Path) -> std::path::PathBuf {
    if !target.exists() {
        return target.to_path_buf();
    }
    let parent = target.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let stem = target
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("파일")
        .to_string();
    let ext = target.extension().and_then(|s| s.to_str()).unwrap_or("").to_string();

    for n in 2..10_000u32 {
        let name = if ext.is_empty() {
            format!("{stem} ({n})")
        } else {
            format!("{stem} ({n}).{ext}")
        };
        let cand = parent.join(name);
        if !cand.exists() {
            return cand;
        }
    }
    target.to_path_buf() // 사실상 도달 불가, 도달하면 덮어쓰기와 같아진다
}

/// 압축 대상 포맷, COM 에서는 포맷 핸들러 CLSID 로 재해석
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressFormat {
    SevenZip,
    Zip,
    Tar,
}

impl CompressFormat {
    /// 알 수 없는 문자열 → 7z
    pub fn from_str(s: &str) -> Self {
        match s {
            "zip" => CompressFormat::Zip,
            "tar" => CompressFormat::Tar,
            _ => CompressFormat::SevenZip,
        }
    }

    /// 압축 레벨(ISetProperties x) 유효 여부, TAR = 무압축이라 제외
    pub fn has_level(self) -> bool {
        !matches!(self, CompressFormat::Tar)
    }

    /// 암호화 지원 여부, TAR 불가
    pub fn supports_password(self) -> bool {
        !matches!(self, CompressFormat::Tar)
    }

    /// 헤더암호(ISetProperties he) 지원 여부, 7z 만 가능
    pub fn supports_header_encryption(self) -> bool {
        matches!(self, CompressFormat::SevenZip)
    }
}

/// 바이러스 검사 콜백, (내부경로, 내용) → 상태 문자열(clean | malware | error | skipped)
pub type ScanFn = Box<dyn FnMut(&str, &[u8]) -> String + Send>;

/// 항목 1개 메모리 해제 시 강제 상한, 기준 = 실제 출력 바이트(신고 크기 아님)
/// 디스크 경로 미적용 (D3.5)
pub const MAX_MEMORY_ENTRY_BYTES: u64 = 256 * 1024 * 1024;

/// 지원 확장자 정본(소문자, 점 없음), 사본 3곳(shellext/ZipManiaShell.cpp, src/lib/*.js)
/// 아래 일치 테스트가 대조, 추가 절차 5단계 + 핸들러 매핑 필수 (D3.8)
pub const READ_EXTS: &[&str] = &[
    // 주력 압축 포맷
    "7z", "zip", "zipx", "jar", "rar", "r00", "arj", "lzh", "lha", "cab", //
    // tar 계열, 단일 스트림 압축
    "tar", "ova", "gz", "gzip", "tgz", "tpz", "bz2", "bzip2", "tbz", "tbz2", "xz", "txz", "zst",
    "tzst", "z", "taz", "lzma", //
    // 디스크, 설치 이미지
    "iso", "img", "udf", "wim", "swm", "esd", "dmg", "squashfs", //
    // 패키지, 설치 파일
    "msi", "msp", "msm", "cpio", "rpm", "deb", "xar", "pkg", "chm", "nsis", //
    // 분할 파일
    "001", //
    // 만화책 아카이브, 7z.dll 이 모르는 별칭 → 매핑에서 zip, rar, 7z 로 전달
    "cbz", "cbr", "cb7", //
    // 이스트소프트 포맷, 7z.dll 아니라 unegg 백엔드(순수 Rust) 담당
    "egg", "alz",
];

/// 7z.dll 담당 = READ_EXTS - 다른 백엔드 소관
/// 겹치면 먼저 등록된 쪽이 가로챔, 검사 = 백엔드_분담이_정본을_덮는다
pub const SEVENZIP_EXTS: &[&str] = &[
    "7z", "rar", "r00", "arj", "lzh", "lha", "cab", //
    "tar", "ova", "gz", "gzip", "tgz", "tpz", "bz2", "bzip2", "tbz", "tbz2", "xz", "txz", "zst",
    "tzst", "z", "taz", "lzma", //
    "iso", "img", "udf", "wim", "swm", "esd", "dmg", "squashfs", //
    "msi", "msp", "msm", "cpio", "rpm", "deb", "xar", "pkg", "chm", "nsis", //
    "001", //
    "cbr", "cb7",
];

/// unzip 담당 = ZIP 계열 전부(읽기, 쓰기 동일 목록), 001 추가 금지 — 7z 분할 조각
pub const UNZIP_EXTS: &[&str] = &["zip", "zipx", "jar", "cbz"];

/// unegg 백엔드(순수 Rust) 담당 확장자
pub const UNEGG_EXTS: &[&str] = &["egg", "alz"];

/// 경로 → 소문자 확장자(점 없음), 없으면 빈 문자열
pub fn ext_of(path: &str) -> String {
    std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default()
}

/// 경로가 READ_EXTS 소속 확장자인가, 소비자용 편의 함수
pub fn is_archive_path(path: &str) -> bool {
    READ_EXTS.contains(&ext_of(path).as_str())
}

// 정본 일치 테스트, 사본(셸 확장 C++, 프런트 JS) 갈라짐 방지
// 파일 없으면(크레이트만 떼어 쓰는 외부 프로젝트) 통과

#[cfg(test)]
mod ext_tests {
    use super::*;
    use std::collections::BTreeSet;

    /// 워크스페이스 루트 기준 파일 읽기, 없으면 None
    fn read_repo_file(rel: &str) -> Option<String> {
        // 이 크레이트 위치 = <root>/crates/zipmania-archive
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..");
        std::fs::read_to_string(root.join(rel)).ok()
    }

    /// start~end 사이 큰따옴표 토큰, C++ L"zip" 과 JS "zip" 둘 다 처리
    fn quoted_between(haystack: &str, start: &str, end: &str) -> BTreeSet<String> {
        let from = match haystack.find(start) {
            Some(i) => i + start.len(),
            None => return BTreeSet::new(),
        };
        let rest = &haystack[from..];
        let to = rest.find(end).unwrap_or(rest.len());
        let mut out = BTreeSet::new();
        let mut it = rest[..to].chars().peekable();
        while let Some(c) = it.next() {
            if c != '"' {
                continue;
            }
            let mut s = String::new();
            for c2 in it.by_ref() {
                if c2 == '"' {
                    break;
                }
                s.push(c2);
            }
            if !s.is_empty() {
                out.insert(s.to_ascii_lowercase());
            }
        }
        out
    }

    fn canonical() -> BTreeSet<String> {
        READ_EXTS.iter().map(|s| s.to_string()).collect()
    }

    /// 사본 vs 정본 비교, 어긋나면 어느 쪽에 무엇이 빠졌는지 출력 후 실패
    fn assert_matches(what: &str, copy: BTreeSet<String>) {
        let canon = canonical();
        let missing: Vec<_> = canon.difference(&copy).cloned().collect();
        let extra: Vec<_> = copy.difference(&canon).cloned().collect();
        assert!(
            missing.is_empty() && extra.is_empty(),
            "{what} 이(가) READ_EXTS 와 어긋납니다.\n  빠짐: {missing:?}\n  잉여: {extra:?}\n\
             → 정본(crates/zipmania-archive READ_EXTS)에 맞춰 고치십시오. 절차는 DESIGN.md 3.8."
        );
    }

    #[test]
    fn 정본에_중복이_없다() {
        let uniq: BTreeSet<&&str> = READ_EXTS.iter().collect();
        assert_eq!(uniq.len(), READ_EXTS.len(), "READ_EXTS 에 중복 확장자가 있습니다.");
    }

    #[test]
    fn 백엔드_분담이_정본을_덮는다() {
        // 백엔드 3개가 정본을 빈틈없이, 겹침없이 분담
        let lists: [(&str, &[&str]); 3] = [
            ("sevenzip", SEVENZIP_EXTS),
            ("unzip", UNZIP_EXTS),
            ("unegg", UNEGG_EXTS),
        ];
        // 겹치면 먼저 등록된 백엔드가 가로챔 → 라우팅이 조용히 어긋남
        for (i, (an, a)) in lists.iter().enumerate() {
            for (bn, b) in lists.iter().skip(i + 1) {
                let sa: BTreeSet<&&str> = a.iter().collect();
                let sb: BTreeSet<&&str> = b.iter().collect();
                let overlap: Vec<_> = sa.intersection(&sb).collect();
                assert!(
                    overlap.is_empty(),
                    "{an} 과 {bn} 이 같은 확장자를 선언했습니다: {overlap:?}"
                );
            }
        }

        let union: BTreeSet<&&str> = lists.iter().flat_map(|(_, l)| l.iter()).collect();
        let canon: BTreeSet<&&str> = READ_EXTS.iter().collect();
        let missing: Vec<_> = canon.difference(&union).collect();
        let extra: Vec<_> = union.difference(&canon).collect();
        assert!(
            missing.is_empty() && extra.is_empty(),
            "백엔드 분담이 READ_EXTS 와 어긋납니다.
  맡는 백엔드 없음: {missing:?}
  정본에 없음: {extra:?}"
        );
    }

    #[test]
    fn 확장자_판정() {
        assert!(is_archive_path(r"D:\a\b.7z"));
        assert!(is_archive_path("만화.CBZ")); // 대소문자 무관
        assert!(!is_archive_path("사진.png"));
        assert!(!is_archive_path("확장자없음"));
    }

    #[test]
    fn 셸확장_목록이_정본과_일치() {
        let Some(src) = read_repo_file("shellext/ZipManiaShell.cpp") else {
            return;
        };
        assert_matches(
            "shellext/ZipManiaShell.cpp 의 kArchiveExts",
            quoted_between(&src, "kArchiveExts[] = {", "};"),
        );
    }

    #[test]
    fn 프론트_목록이_정본과_일치() {
        let Some(src) = read_repo_file("src/lib/stores.js") else {
            return;
        };
        assert_matches(
            "src/lib/stores.js 의 ARCHIVE_EXTS",
            quoted_between(&src, "const ARCHIVE_EXTS = new Set([", "]);"),
        );
    }

    #[test]
    fn 열기필터_목록이_정본과_일치() {
        let Some(src) = read_repo_file("src/lib/api.js") else {
            return;
        };
        assert_matches(
            "src/lib/api.js 의 열기 다이얼로그 필터",
            quoted_between(&src, "name: \"압축 파일\",", "]"),
        );
    }
}
