//! 업데이트 확인(앱 시작 1회), 로직은 klib_update, klib_dialog, 여기는 시점, 부모 창, 동의 후 동작만
//! 자동 교체 아님
//! 시작 → 서버 조회 → 다이얼로그 → 예: 브라우저 + 종료 / 아니오: 없음 / notify: 상태줄 배지(update:notify)

use std::sync::Mutex;

use tauri::{Emitter, Manager};

/// ZipMania 전용 조회 주소, 앱마다 다르다 — 없는 앱은 서버가 404 를 낸다
const UPDATE_URL: &str = "https://report.kilho.net/status/zipmania";

/// 상태줄 배지에 실어 보내는 정보
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotifyPayload {
    url: String,
    text: String,
}

/// 화면 준비 전 도착한 공지 보관, 화면은 get_update_notify 로 1회 조회 후 이벤트 수신
#[derive(Default)]
pub struct PendingNotify(Mutex<Option<NotifyPayload>>);

/// 보관된 공지 반환(비우지 않음)
#[tauri::command]
pub fn get_update_notify(state: tauri::State<'_, PendingNotify>) -> Option<NotifyPayload> {
    state.0.lock().ok().and_then(|v| v.clone())
}

/// 백그라운드 업데이트 확인(실패는 무시), 스레드 생성 + 다이얼로그도 그 스레드에서
pub fn spawn(app: &tauri::AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let lang = language(&app);
        let mut cfg = klib_update::UpdateConfig::new(UPDATE_URL, version());
        cfg.language = lang.clone();
        cfg.debug = cfg!(debug_assertions);

        // 오프라인의 network 오류는 정상, 앱 실행 무방해
        let info = match klib_update::check(&cfg) {
            Ok(info) => info,
            Err(e) => {
                eprintln!("[update] {e}");
                return;
            }
        };
        if let Some(err) = &info.server_error {
            eprintln!("[update] 서버 응답: {err}");
        }

        // 공지는 다이얼로그 없이 상태줄에만 띄운다
        if let Some(url) = info.notify.clone() {
            let payload = NotifyPayload {
                url,
                text: klib_update::text::notify(&lang).to_string(),
            };
            // 선보관, 화면이 늦게 떠도 즉시 회수 가능하도록
            if let Ok(mut slot) = app.state::<PendingNotify>().0.lock() {
                *slot = Some(payload.clone());
            }
            let _ = app.emit("update:notify", payload);
        }

        let Some(update_url) = info.update.clone() else {
            return;
        };

        let Some(answer) = ask(&app, &info, &lang) else {
            return;
        };

        // 다시 보지 않기 = 예/아니오와 무관하게 전송(델파이도 동일)
        if answer.checked {
            let _ = klib_update::send_command(&cfg, "hold");
        }

        if answer.yes {
            let _ = open_url(&update_url);
            // 종료 결정은 앱 몫
            app.exit(0);
        }
    });
}

/// 서버에 보낼 버전 문자열, 서버는 문자열 동일 여부만 확인, ZIPMANIA_UPDATE_VERSION 으로 덮어쓰기 가능
fn version() -> String {
    std::env::var("ZIPMANIA_UPDATE_VERSION")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string())
}

/// 다이얼로그를 띄운다, 띄우지 못하면 None
fn ask(
    app: &tauri::AppHandle,
    info: &klib_update::UpdateInfo,
    lang: &str,
) -> Option<klib_dialog::Answer> {
    let text = klib_update::text::dialog(lang);

    let mut dialog = klib_dialog::Dialog {
        title: "ZipMania",
        instruction: text.instruction,
        content: text.content,
        icon: klib_dialog::Icon::Warning,
        buttons: klib_dialog::Buttons::YesNo,
        parent: parent_hwnd(app),
        stay_on_top: info.stay_on_top,
        ..Default::default()
    };

    // 서버가 style 을 지정했을 때만 요소를 붙인다, 빈 확인란, 빈 링크 금지
    match info.style {
        klib_update::DialogStyle::Hold => dialog.verification = info.text.as_deref(),
        klib_update::DialogStyle::Note => {
            if let (Some(t), Some(u)) = (info.text.as_deref(), info.url.as_deref()) {
                dialog.footer_link = Some((t, u));
            }
        }
        klib_update::DialogStyle::None => {}
    }

    match klib_dialog::show(&dialog) {
        Ok(a) => Some(a),
        Err(e) => {
            eprintln!("[update] 다이얼로그 실패: {e}");
            None
        }
    }
}

/// 메인 창이 보일 때만 부모로 삼는다, 숨은 창을 부모로 주면 다이얼로그가 안 보인다
fn parent_hwnd(app: &tauri::AppHandle) -> Option<isize> {
    #[cfg(windows)]
    {
        let win = app.get_webview_window("main")?;
        if !win.is_visible().unwrap_or(false) {
            return None;
        }
        return win.hwnd().ok().map(|h| h.0 as isize);
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        None
    }
}

/// 안내 문구 언어 코드, 설정 언어 또는 OS 언어, 앱 UI 언어에 맞추기 금지 — klib_update::text 는 9개 언어(U)
pub fn language(app: &tauri::AppHandle) -> String {
    language_from(&crate::settings::load(app).language)
}

/// 설정값(system/ko/en) → 언어 코드, AppHandle 없는 곳(설치/제거 훅)에서도 사용
pub fn language_from(configured: &str) -> String {
    let configured = configured.to_string();
    let raw = if configured == "system" || configured.is_empty() {
        os_language()
    } else {
        configured
    };
    // 델파이는 ko 처럼 두 자리 전송, ko-KR 에서 앞부분만 사용
    raw.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase()
}

#[cfg(windows)]
fn os_language() -> String {
    use windows::Win32::Globalization::GetUserDefaultLocaleName;
    let mut buf = [0u16; 85]; // LOCALE_NAME_MAX_LENGTH
    let len = unsafe { GetUserDefaultLocaleName(&mut buf) };
    if len <= 1 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..(len - 1) as usize])
}

#[cfg(not(windows))]
fn os_language() -> String {
    std::env::var("LANG").unwrap_or_default()
}

/// 상태줄 배지를 클릭했을 때 공지 주소를 연다
#[tauri::command]
pub fn open_update_url(url: String) -> Result<(), String> {
    // 프런트 값이므로 http(s) 만 허용
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("http(s) 주소만 열 수 있습니다.".into());
    }
    open_url(&url)
}

/// 기본 브라우저로 주소를 연다(델파이 NewIE)
fn open_url(url: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("explorer")
            .raw_arg(format!("\"{url}\""))
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        let _ = url;
        Err("미지원".into())
    }
}
