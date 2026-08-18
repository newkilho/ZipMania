use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn main() {
    // tauri-build 는 아이콘 변경만으로 재실행 안 됨 → 옛 아이콘 박힘
    println!("cargo:rerun-if-changed=icons/icon.ico");

    export_build_env();

    let rc = assoc_icons_rc();
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .windows_attributes(tauri_build::WindowsAttributes::new().append_rc_content(rc)),
    )
    .expect("tauri 빌드 실패");
}

/// 저장소 루트 .env → 컴파일 시점 환경 변수, 프로세스 환경이 있으면 그것이 이김(CI 재정의)
fn export_build_env() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join(".env");
    println!("cargo:rerun-if-changed={}", path.display());
    println!("cargo:rerun-if-env-changed=UPDATE_URL");

    let value = std::env::var("UPDATE_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| dotenv_value(&path, "UPDATE_URL"))
        .unwrap_or_else(|| {
            panic!("UPDATE_URL 이 없습니다. {} 에 넣거나 환경 변수로 주십시오", path.display())
        });

    // 오타는 여기서 잡는다, 실행 시점에 잡으면 이미 배포된 뒤
    if !(value.starts_with("https://") || value.starts_with("http://")) {
        panic!("UPDATE_URL 은 전체 URL(https://...)이어야 합니다: {value:?}");
    }
    println!("cargo:rustc-env=UPDATE_URL={value}");
}

/// .env 에서 KEY=VALUE 하나, # 주석과 빈 줄 무시, 값의 감싼 따옴표 제거
fn dotenv_value(path: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((k, v)) = line.split_once('=') else { continue };
        if k.trim() != key {
            continue;
        }
        let v = v.trim();
        let v = v.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(v);
        let v = v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')).unwrap_or(v);
        return Some(v.to_string());
    }
    None
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
