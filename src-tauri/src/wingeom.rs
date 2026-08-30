//! 메인 창/뷰어 창 크기 기억, 정본은 settings.toml, 기록은 종료 시 1회
//! 압축/해제 창 제외 — 진행↔폼 전환이 FORM_SIZE 로 크기를 되돌린다(D3.4)

use std::sync::{Arc, Mutex};

use tauri::{AppHandle, PhysicalSize, WebviewWindow};

use crate::settings::Settings;

/// 메인 창 기본 크기, tauri.conf.json 의 width/height 와 동일 — 아래 테스트가 대조
pub const DEF_W: f64 = 820.0;
pub const DEF_H: f64 = 600.0;

/// 메인 창 최소 크기, tauri.conf.json 의 minWidth/minHeight 와 동일
pub const MIN_W: f64 = 690.0;
pub const MIN_H: f64 = 420.0;

/// 메인 창의 마지막 비최대화 크기, width/height 의 0 = 아직 모름
#[derive(Default, Clone, Copy)]
pub struct Geom {
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
    pub dirty: bool,
}

/// 창 이벤트 스레드와 종료 훅이 함께 보는 자리
pub type GeomCell = Arc<Mutex<Geom>>;

/// 새 캐시
pub fn cell() -> GeomCell {
    Arc::new(Mutex::new(Geom::default()))
}

/// 저장값 → 실제로 적용할 논리 크기, 최소 크기 미만은 set_size 가 조용히 무시하므로 clamp
pub fn restore_size(app: &AppHandle, s: &Settings) -> (f64, f64) {
    // 둘은 항상 함께 기록되므로 함께 쓴다, 한쪽만 살리면 저장한 적 없는 비율이 나온다
    let (mut w, mut h) = if s.window_width > 0 && s.window_height > 0 {
        (s.window_width as f64, s.window_height as f64)
    } else {
        (DEF_W, DEF_H)
    };
    // 모니터가 바뀌어 화면보다 큰 값이 남아 있을 수 있다
    if let Ok(Some(m)) = app.primary_monitor() {
        let size = m.size().to_logical::<f64>(m.scale_factor());
        w = w.min(size.width);
        h = h.min(size.height);
    }
    (w.max(MIN_W), h.max(MIN_H))
}

/// Resized 이벤트 → 캐시 갱신, 최대화는 플래그만, 최소화(0)는 무시
/// 숨긴 창의 이벤트는 사용자 조작이 아니므로 버린다 — cli_mode 로 거르면 안 된다(그 세션도
/// 나중에 메인 창을 띄운다), 판정은 시작 시점이 아니라 이벤트 시점의 가시성
pub fn track(cell: &GeomCell, window: &WebviewWindow, size: PhysicalSize<u32>) {
    if !window.is_visible().unwrap_or(false) {
        return;
    }
    let maximized = window.is_maximized().unwrap_or(false);
    let Ok(mut g) = cell.lock() else { return };
    if maximized {
        g.maximized = true;
        g.dirty = true;
        return;
    }
    if size.width == 0 || size.height == 0 {
        return;
    }
    // Resized 는 물리 픽셀, inner_size/set_size 는 논리 크기 — 변환 누락 시 고DPI 에서 재시작마다 확대
    let logical = size.to_logical::<f64>(window.scale_factor().unwrap_or(1.0));
    g.width = logical.width.round() as u32;
    g.height = logical.height.round() as u32;
    g.maximized = false;
    g.dirty = true;
}

/// 종료 시 기록, 저장 직전 재읽기로 다른 창이 바꾼 필드 보존, 읽지 못하면 미저장
pub fn save(app: &AppHandle, cell: &GeomCell) {
    let Ok(g) = cell.lock() else { return };
    if !g.dirty {
        return;
    }
    let (mut s, trusted) = crate::settings::load_checked(app);
    if !trusted {
        return;
    }
    if g.width > 0 && g.height > 0 {
        s.window_width = g.width;
        s.window_height = g.height;
    }
    s.window_maximized = g.maximized;
    let _ = crate::settings::save(app, &s);
}

#[cfg(test)]
mod tests {
    /// 상수가 tauri.conf.json 과 어긋나면 첫 실행 크기와 clamp 기준이 갈린다
    #[test]
    fn 창_크기_상수가_tauri_conf_와_같다() {
        let conf = include_str!("../tauri.conf.json");
        for (key, want) in [
            ("width", super::DEF_W),
            ("height", super::DEF_H),
            ("minWidth", super::MIN_W),
            ("minHeight", super::MIN_H),
        ] {
            let needle = format!("\"{key}\": {}", want as u32);
            assert!(
                conf.contains(&needle),
                "tauri.conf.json 에 {needle} 이 없다 — wingeom 상수와 어긋났다"
            );
        }
    }
}
