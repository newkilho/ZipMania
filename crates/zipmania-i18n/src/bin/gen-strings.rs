//! 정본(strings.rs) → 프런트 사전 + 셸 확장 헤더
//!
//! cargo run -p zipmania-i18n --bin gen-strings

use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("저장소 루트를 찾지 못했다")
        .to_path_buf();

    for (path, body) in [
        (zipmania_i18n::LOCALES_JSON_PATH, zipmania_i18n::locales_json()),
        (zipmania_i18n::SHELL_HEADER_PATH, zipmania_i18n::shell_header()),
    ] {
        let full = root.join(path);
        if let Some(dir) = full.parent() {
            std::fs::create_dir_all(dir).expect("폴더를 만들지 못했다");
        }
        // 내용이 같으면 건드리지 않는다(빌드 시스템의 타임스탬프 판정 보호)
        if std::fs::read_to_string(&full).map(|s| s == body).unwrap_or(false) {
            println!("그대로 {path}");
            continue;
        }
        std::fs::write(&full, &body).expect("생성물을 쓰지 못했다");
        println!("생성 {path}");
    }

    println!(
        "항목 {} 개, 언어 {} 개",
        zipmania_i18n::STRINGS.len(),
        zipmania_i18n::LANGS.len()
    );
}
