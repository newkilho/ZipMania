//! 오류 보고, 로직은 klib_except, 여기는 설치 시점과 앱 이름/버전/언어만
//! 패닉, 네이티브 크래시 모두 그 자리에서 백그라운드 전송, 파일도 화면도 없음
//! 앱이 잡아서 표시하는 일반 오류(ZipManiaError)는 보내지 않음

/// 크래시 핸들러 설치, Tauri 를 띄우기 전에 부를 것
pub fn install() {
    let mut cfg = klib_except::ExceptConfig::new("ZipMania", env!("CARGO_PKG_VERSION"));
    // 설정을 아직 읽을 수 없는 시점, 언어는 OS 것으로
    cfg.language = crate::update::language_from("system");

    if let Err(e) = klib_except::install(cfg) {
        eprintln!("[except] {e}");
    }
}
