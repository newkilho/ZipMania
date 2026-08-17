//! 아카이브 내부 경로 → 안전한 로컬 상대경로, Zip Slip 방어, 전 백엔드 공용 (D3.5)

use std::path::{Component, Path, PathBuf};

use crate::error::ZipManiaError;

/// Windows 예약 장치 이름, 확장자 붙여도 예약(CON.txt 포함)
const WIN_RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

fn unsafe_path(name: &str, why: &str) -> ZipManiaError {
    ZipManiaError::new("unsafe_path", format!("{why}: {name}"))
}

/// 내부 이름 → 출력 루트 기준 상대경로
/// 제거 = 드라이브, 선행 구분자, UNC, 점
/// 거부 = .., 제어문자
/// Windows 한정 = 금지문자 → _, 예약어 → _ 접두, 끝 점 제거, 끝 공백 제거
pub fn sanitize(name: &str) -> Result<PathBuf, ZipManiaError> {
    sanitize_for(name, cfg!(windows))
}

/// 플랫폼 규칙 = 인자, 테스트가 양쪽 검사
pub fn sanitize_for(name: &str, for_windows: bool) -> Result<PathBuf, ZipManiaError> {
    if name.contains('\0') {
        return Err(unsafe_path(name, "NUL 문자가 포함된 이름"));
    }

    // 드라이브 접두사 제거 → 구분자 통일
    let unified = name.replace('\\', "/");
    // C:/…, C: 만 제거, a:b?c.txt 는 파일명
    let body = match unified.as_bytes() {
        [d, b':'] if d.is_ascii_alphabetic() => "",
        [d, b':', b'/', ..] if d.is_ascii_alphabetic() => &unified[2..],
        _ => &unified[..],
    };

    let mut parts: Vec<String> = Vec::new();
    for raw in body.split('/') {
        let part = raw.trim();
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            return Err(unsafe_path(name, "상위 경로로 탈출하는 이름"));
        }
        if part.chars().any(|c| (c as u32) < 0x20) {
            return Err(unsafe_path(name, "제어문자가 포함된 이름"));
        }

        let mut part = part.to_string();
        if for_windows {
            part = part
                .chars()
                .map(|c| if "<>:\"|?*".contains(c) { '_' } else { c })
                .collect();
            part = part.trim_end_matches([' ', '.']).to_string();
            if part.is_empty() {
                continue;
            }
            let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
            if WIN_RESERVED.contains(&stem.as_str()) {
                part.insert(0, '_');
            }
        }
        parts.push(part);
    }

    if parts.is_empty() {
        return Err(unsafe_path(name, "빈 경로가 되었습니다"));
    }
    Ok(parts.iter().collect())
}

/// root 하위 절대경로, sanitize 다음 호출, 조상에 낀 링크와 정션까지 검사
pub fn resolve_under(root: &Path, relative: &Path) -> Result<PathBuf, ZipManiaError> {
    // 비교 기준 = 정규화한 루트
    let root_abs = root.canonicalize().unwrap_or_else(|_| absolutize(root));
    let target = root_abs.join(relative);

    // 실재하는 가장 깊은 조상까지 검사, 루트에서 중단 (D3.5)
    let mut probe = target.clone();
    while probe != root_abs && probe.starts_with(&root_abs) {
        if let Ok(real) = probe.canonicalize() {
            if !real.starts_with(&root_abs) {
                return Err(unsafe_path(
                    &relative.to_string_lossy(),
                    "출력 루트를 벗어납니다",
                ));
            }
            break;
        }
        match probe.parent() {
            Some(p) => probe = p.to_path_buf(),
            None => break,
        }
    }
    Ok(target)
}

/// canonicalize 실패(경로 미존재) 시 최소 절대화
fn absolutize(p: &Path) -> PathBuf {
    if p.is_absolute() {
        return p.to_path_buf();
    }
    let mut out = std::env::current_dir().unwrap_or_default();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(name: &str) -> String {
        sanitize_for(name, true)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/")
    }

    #[test]
    fn 상대화한다() {
        assert_eq!(s("a/b/c.txt"), "a/b/c.txt");
        assert_eq!(s("a\\b\\c.txt"), "a/b/c.txt");
        assert_eq!(s("/etc/passwd"), "etc/passwd");
        assert_eq!(s("C:\\Windows\\notepad.exe"), "Windows/notepad.exe");
        assert_eq!(s("\\\\server\\share\\f.txt"), "server/share/f.txt");
        assert_eq!(s("./a/./b"), "a/b");
    }

    #[test]
    fn 상위경로_탈출은_거부한다() {
        for bad in ["../evil", "a/../../evil", "..\\evil", ".."] {
            let e = sanitize_for(bad, true).unwrap_err();
            assert_eq!(e.code, "unsafe_path", "거부되지 않음: {bad}");
        }
    }

    #[test]
    fn windows_금지문자와_예약어() {
        assert_eq!(s("a<b>c.txt"), "a_b_c.txt");
        assert_eq!(s("CON"), "_CON");
        assert_eq!(s("con.txt"), "_con.txt");
        assert_eq!(s("이름 ."), "이름"); // 끝 점, 공백 제거
    }

    #[test]
    fn 비windows에서는_금지문자를_보존한다() {
        let p = sanitize_for("a:b?c.txt", false).unwrap();
        assert_eq!(p.to_string_lossy(), "a:b?c.txt");
    }

    #[test]
    fn nul과_빈경로는_거부한다() {
        assert!(sanitize_for("a\0b", true).is_err());
        assert!(sanitize_for("", true).is_err());
        assert!(sanitize_for("///", true).is_err());
    }

    /// 조상 탐색이 루트 위로 → 루트의 부모가 실재 조상 → 전부 거부
    #[test]
    fn 루트가_아직_없어도_통과한다() {
        let root = std::env::temp_dir()
            .join(format!("zipmania_ru_{}", std::process::id()))
            .join("out");
        let _ = std::fs::remove_dir_all(&root);
        let rel = sanitize("a/b/c.txt").unwrap();
        let got = resolve_under(&root, &rel).expect("루트가 없다고 거부하면 안 된다");
        assert!(got.ends_with("a/b/c.txt") || got.ends_with(r"a\b\c.txt"), "{got:?}");
    }

    #[test]
    fn 루트_아래로만_떨어진다() {
        let root = std::env::temp_dir().join(format!("zipmania_ru2_{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let rel = sanitize("sub/f.txt").unwrap();
        let got = resolve_under(&root, &rel).unwrap();
        assert!(got.starts_with(root.canonicalize().unwrap()), "{got:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn 한글_이름은_그대로_둔다() {
        assert_eq!(s("사진/2026/겨울.png"), "사진/2026/겨울.png");
    }
}
