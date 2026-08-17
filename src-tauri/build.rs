use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn main() {
    // tauri-build 는 아이콘 변경만으로 재실행 안 됨 → 옛 아이콘 박힘
    println!("cargo:rerun-if-changed=icons/icon.ico");

    let rc = assoc_icons_rc();
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .windows_attributes(tauri_build::WindowsAttributes::new().append_rc_content(rc)),
    )
    .expect("tauri 빌드 실패");
}

/// icon/ 폴더 → exe 아이콘 리소스, .rc 조각 + Rust 조회표 동시 생성, ID 는 이름순
/// 별도 .res 링크 금지, (D3.12)
fn assoc_icons_rc() -> String {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("icon");
    println!("cargo:rerun-if-changed={}", dir.display());

    let mut icons: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("아이콘 폴더를 읽지 못했습니다({}): {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map(|e| e.eq_ignore_ascii_case("ico")).unwrap_or(false))
        .collect();
    icons.sort();

    // 앱 아이콘(tauri 32512)보다 커야 함, (D3.12)
    const FIRST_ID: u16 = 40000;

    let mut rc = String::new();
    let mut table = String::from("// build.rs 가 생성한다. 손으로 고치지 않는다.\n");
    let _ = writeln!(table, "pub const ICON_IDS: &[(&str, u16)] = &[");

    for (i, path) in icons.iter().enumerate() {
        let id = FIRST_ID + i as u16;
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_lowercase();
        let _ = writeln!(rc, "{id} ICON \"{}\"", rc_path(path));
        let _ = writeln!(table, "    ({stem:?}, {id}),");
    }
    let _ = writeln!(table, "];");

    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    std::fs::write(out.join("assoc_icon_ids.rs"), table).expect("아이콘 ID 표 생성 실패");

    rc
}

/// .rc 문자열용 경로, 역슬래시 이스케이프
fn rc_path(p: &Path) -> String {
    p.display().to_string().replace('\\', "\\\\")
}
