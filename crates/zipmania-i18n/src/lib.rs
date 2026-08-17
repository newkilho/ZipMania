//! UI 텍스트 정본(strings.rs)과 프런트/셸 확장용 생성물 만들기
//!
//! strings 정본 표(LANGS, LANG_LABELS, STRINGS)
//! text/lang_index Rust 쪽 조회
//! locales_json/shell_header 생성물 본문, 파일로 쓰는 것은 gen-strings

mod strings;

pub use strings::{LANGS, LANG_LABELS, STRINGS};

/// 모르는 언어가 떨어질 자리, LANGS 안의 영어 위치
const FALLBACK: usize = 1;

/// 프런트가 읽는 사전, 정본에서 생성
pub const LOCALES_JSON_PATH: &str = "src/locales/strings.json";

/// 셸 확장이 포함하는 헤더, 정본에서 생성
pub const SHELL_HEADER_PATH: &str = "shellext/strings.generated.h";

/// 언어 코드 → LANGS 인덱스, 모르는 언어는 영어
pub fn lang_index(lang: &str) -> usize {
    LANGS.iter().position(|&c| c == lang).unwrap_or(FALLBACK)
}

/// 키와 언어로 번역 하나, 없는 키는 키 문자열 그대로
pub fn text(key: &'static str, lang: &str) -> &'static str {
    let i = lang_index(lang);
    STRINGS
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v[i])
        .unwrap_or(key)
}

/// JSON 문자열 리터럴 하나, 제어문자는 \u 로
fn json_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// C 와이드 문자열 리터럴 하나
fn c_str(s: &str, out: &mut String) {
    out.push_str("L\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// 프런트 사전 본문, { langs: [{code,label}], strings: { 코드: { 키: 값 } } }
pub fn locales_json() -> String {
    let mut s = String::with_capacity(128 * 1024);
    s.push_str("{\n  \"langs\": [\n");
    for (i, code) in LANGS.iter().enumerate() {
        s.push_str("    { \"code\": ");
        json_str(code, &mut s);
        s.push_str(", \"label\": ");
        json_str(LANG_LABELS[i], &mut s);
        s.push_str(" }");
        if i + 1 < LANGS.len() {
            s.push(',');
        }
        s.push('\n');
    }
    s.push_str("  ],\n  \"strings\": {\n");
    for (i, code) in LANGS.iter().enumerate() {
        s.push_str("    ");
        json_str(code, &mut s);
        s.push_str(": {\n");
        for (j, (key, vals)) in STRINGS.iter().enumerate() {
            s.push_str("      ");
            json_str(key, &mut s);
            s.push_str(": ");
            json_str(vals[i], &mut s);
            if j + 1 < STRINGS.len() {
                s.push(',');
            }
            s.push('\n');
        }
        s.push_str("    }");
        if i + 1 < LANGS.len() {
            s.push(',');
        }
        s.push('\n');
    }
    s.push_str("  }\n}\n");
    s
}

/// 셸 확장 메뉴 문구, shell. 접두사를 뗀 열 개를 언어 순서로
pub fn shell_header() -> String {
    const FIELDS: [&str; 10] = [
        "shell.compressZipPre",
        "shell.compressZipPost",
        "shell.compress",
        "shell.compressEach",
        "shell.extractHere",
        "shell.extractToPre",
        "shell.extractToPost",
        "shell.extract",
        "shell.open",
        "shell.extractEach",
    ];

    let mut s = String::with_capacity(16 * 1024);
    s.push_str("// 생성물 — 손으로 고치지 말 것, 정본은 zipmania-i18n 의 strings.rs\n");
    s.push_str("// 갱신 = cargo run -p zipmania-i18n --bin gen-strings\n");
    s.push_str("#pragma once\n\n");
    s.push_str("// 메뉴 문구 한 벌, Pre/Post 는 이름을 사이에 끼우는 앞뒤 조각\n");
    s.push_str("struct MenuText\n{\n");
    for f in FIELDS {
        s.push_str("    const wchar_t* ");
        s.push_str(f.trim_start_matches("shell."));
        s.push_str(";\n");
    }
    s.push_str("};\n\n");
    s.push_str("// 언어별 메뉴 문구, 코드 순서는 LANGS 와 같다\n");
    s.push_str("static const struct\n{\n    const wchar_t* code;\n    MenuText text;\n} kMenuTexts[] = {\n");
    for (i, code) in LANGS.iter().enumerate() {
        s.push_str("    {");
        c_str(code, &mut s);
        s.push_str(",\n     {");
        for (j, f) in FIELDS.iter().enumerate() {
            if j > 0 {
                s.push_str(",\n      ");
            }
            c_str(text(f, code), &mut s);
        }
        s.push_str("}},\n");
        let _ = i;
    }
    s.push_str("};\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// 저장소 루트, 이 크레이트는 crates/zipmania-i18n 에 있다
    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf()
    }

    #[test]
    fn 폴백은_영어다() {
        assert_eq!(LANGS[FALLBACK], "en");
        assert_eq!(lang_index("de"), FALLBACK);
        assert_eq!(lang_index("ko"), 0);
    }

    /// 표기 개수가 어긋나면 선택 목록에 빈 항목이 생긴다
    #[test]
    fn 언어_표기가_코드와_짝이다() {
        assert_eq!(LANGS.len(), LANG_LABELS.len());
        for (i, label) in LANG_LABELS.iter().enumerate() {
            assert!(!label.trim().is_empty(), "{} 표기가 비었다", LANGS[i]);
        }
    }

    /// 빈 번역은 화면에 빈칸으로 나온다, 키 중복은 뒤엣것이 영영 안 불린다
    #[test]
    fn 모든_항목이_모든_언어를_채운다() {
        let mut seen: Vec<&str> = Vec::new();
        for (key, vals) in STRINGS {
            assert!(!seen.contains(key), "{key} 가 두 번 있다");
            seen.push(key);
            for (i, v) in vals.iter().enumerate() {
                assert!(!v.trim().is_empty(), "{key} 의 {} 번역이 비었다", LANGS[i]);
            }
        }
    }

    /// 치환자가 어긋나면 {count} 가 그대로 노출되거나 값이 사라진다
    #[test]
    fn 치환자가_언어마다_같다() {
        fn holders(s: &str) -> Vec<String> {
            let mut v: Vec<String> = Vec::new();
            let b: Vec<char> = s.chars().collect();
            let mut i = 0;
            while i < b.len() {
                if b[i] == '{' {
                    if let Some(end) = b[i + 1..].iter().position(|&c| c == '}') {
                        v.push(b[i + 1..i + 1 + end].iter().collect());
                        i += end + 2;
                        continue;
                    }
                }
                i += 1;
            }
            v.sort();
            v
        }
        for (key, vals) in STRINGS {
            let want = holders(vals[0]);
            for (i, v) in vals.iter().enumerate() {
                assert_eq!(holders(v), want, "{key} 의 {} 치환자가 다르다", LANGS[i]);
            }
        }
    }

    /// Rust 코드가 부르는 키가 빠지면 화면에 키 문자열이 나온다
    #[test]
    fn 코드가_쓰는_키가_표에_있다() {
        for key in ["assoc.typeName", "assoc.appName"] {
            assert!(
                STRINGS.iter().any(|(k, _)| *k == key),
                "{key} 가 표에 없다"
            );
        }
        assert!(text("assoc.typeName", "ko").contains("{ext}"));
    }

    /// 정본을 고치고 생성기를 돌리지 않으면 프런트와 셸 확장이 옛 문구로 남는다
    #[test]
    fn 생성물이_정본과_같다() {
        for (path, want) in [
            (LOCALES_JSON_PATH, locales_json()),
            (SHELL_HEADER_PATH, shell_header()),
        ] {
            let full = repo_root().join(path);
            let got = std::fs::read_to_string(&full)
                .unwrap_or_else(|e| panic!("{path} 를 읽지 못했다: {e}"));
            assert_eq!(
                got.replace("\r\n", "\n"),
                want.replace("\r\n", "\n"),
                "{path} 가 정본과 다르다 — cargo run -p zipmania-i18n --bin gen-strings"
            );
        }
    }
}
