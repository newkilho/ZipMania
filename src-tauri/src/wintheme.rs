//! 창 제목 표시줄 다크(DWM immersive dark), 캡션은 항상 다크, WebView 안은 CSS(data-theme)
//! Windows 11 22000+ 지원, 이하, 비-Windows 는 no-op

/// 현재 창의 제목 표시줄 다크 강제, 비-Windows 는 no-op
pub fn apply_window_chrome(window: &tauri::WebviewWindow) {
    #[cfg(windows)]
    {
        use windows::Win32::Graphics::Dwm::{
            DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE,
        };
        use windows::Win32::UI::WindowsAndMessaging::{
            SetWindowPos, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
        };

        if let Ok(hwnd) = window.hwnd() {
            let dark: i32 = 1; // 제목 표시줄 항상 다크(4바이트 BOOL 대체)
            unsafe {
                let _ = DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_USE_IMMERSIVE_DARK_MODE,
                    &dark as *const i32 as *const core::ffi::c_void,
                    std::mem::size_of::<i32>() as u32,
                );
                // 캡션 즉시 재도색을 위한 프레임 변경 통지
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
                );
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = window;
    }
}
