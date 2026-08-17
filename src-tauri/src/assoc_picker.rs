//! 확장자 1개짜리 [기본 앱 선택] 창, 임시 파일 속성 창 → 화면에서 지움 → WM_COMMAND 0x3363(비문서화)
//! 실패 시 false → 호출부가 기본 앱 설정 열기로 폴백, (F13)
//! 전용 스레드 필수 — SetWinEventHook 콜백은 호출 스레드 큐로 도착, Tauri command 스레드에는 루프 부재

#![cfg(windows)]

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CLOAK};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_INVOKEIDLIST, SHELLEXECUTEINFOW};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, EnumWindows, GetClassNameW, GetWindowLongW, GetWindowTextW,
    GetWindowThreadProcessId, IsWindow, MoveWindow, PeekMessageW, PostMessageW,
    SetLayeredWindowAttributes, SetWindowLongW, TranslateMessage, EVENT_OBJECT_CREATE,
    EVENT_OBJECT_SHOW, GWL_EXSTYLE, LWA_ALPHA, MSG, PM_REMOVE, SW_HIDE, WINEVENT_OUTOFCONTEXT,
    WM_CLOSE, WM_COMMAND, WS_EX_LAYERED,
};

/// 속성 창의 '연결 프로그램: 변경' 명령 ID, 비문서화다
const IDM_CHANGE_ASSOC: usize = 0x3363;
/// 속성 시트의 창 클래스(공용 대화상자)
const DIALOG_CLASS: &str = "#32770";

// 훅 콜백은 인자를 못 받으므로 상태는 전역, 선택 창은 한 번에 하나

/// 확장자를 뗀 임시 파일 이름(소문자), 속성 창 제목 대조용
static SHEET_NAME: Mutex<String> = Mutex::new(String::new());
/// 숨긴 속성 창을 놓아 둘 좌표(우리 창 가운데)
static SHEET_ANCHOR: Mutex<(i32, i32)> = Mutex::new((0, 0));
/// 찾아낸 속성 창(HWND 를 isize 로)
static SHEET_HWND: AtomicIsize = AtomicIsize::new(0);
/// 나중에 지울 임시 파일
static TEMP_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);
/// 진행 중 표시 — 두 번 눌러도 창 이중 표시 방지
static RUNNING: AtomicBool = AtomicBool::new(false);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    // 잠금 오염돼도 값은 그대로 사용
    m.lock().unwrap_or_else(|e| e.into_inner())
}

unsafe fn window_text(h: HWND) -> String {
    let mut buf = [0u16; 260];
    let n = GetWindowTextW(h, &mut buf);
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

unsafe fn window_class(h: HWND) -> String {
    let mut buf = [0u16; 260];
    let n = GetClassNameW(h, &mut buf);
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// 우리가 띄운 속성 창인가, 확장자 뗀 이름으로 제목 비교, 제목 빈 값 = 후보(F13)
unsafe fn is_our_sheet(h: HWND) -> bool {
    if window_class(h) != DIALOG_CLASS {
        return false;
    }

    let title = window_text(h).to_lowercase();
    if !title.is_empty() {
        let name = lock(&SHEET_NAME).clone();
        return !name.is_empty() && title.contains(&name);
    }

    let mut pid = 0u32;
    GetWindowThreadProcessId(h, Some(&mut pid));
    pid != GetCurrentProcessId()
}

/// 창을 화면에서 지운다, layered alpha 0 + DWMWA_CLOAK + 크기 0. SW_HIDE, 화면 밖 이동 금지(F13)
unsafe fn vanish(h: HWND) {
    let (x, y) = *lock(&SHEET_ANCHOR);

    let ex = GetWindowLongW(h, GWL_EXSTYLE);
    SetWindowLongW(h, GWL_EXSTYLE, ex | WS_EX_LAYERED.0 as i32);
    let _ = SetLayeredWindowAttributes(h, COLORREF(0), 0, LWA_ALPHA);

    let cloak: i32 = 1;
    let _ = DwmSetWindowAttribute(
        h,
        DWMWA_CLOAK,
        &cloak as *const i32 as *const c_void,
        std::mem::size_of::<i32>() as u32,
    );

    let _ = MoveWindow(h, x, y, 0, 0, false);
}

/// 창 생성 통지, EVENT_OBJECT_CREATE 부터 받을 것 — EVENT_OBJECT_SHOW 는 늦다
unsafe extern "system" fn on_win_event(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _thread: u32,
    _time: u32,
) {
    if id_object != 0 || id_child != 0 || hwnd.0.is_null() {
        return; // OBJID_WINDOW 만
    }

    let known = SHEET_HWND.load(Ordering::Relaxed);
    if known != 0 {
        // 셸이 다시 표시하려 하면 또 지운다
        if known == hwnd.0 as isize {
            vanish(hwnd);
        }
        return;
    }

    if is_our_sheet(hwnd) {
        SHEET_HWND.store(hwnd.0 as isize, Ordering::Relaxed);
        vanish(hwnd);
    }
}

/// 훅이 놓쳤을 때의 보조 탐색
unsafe extern "system" fn enum_find(hwnd: HWND, _l: LPARAM) -> windows::core::BOOL {
    if SHEET_HWND.load(Ordering::Relaxed) == 0 && is_our_sheet(hwnd) {
        SHEET_HWND.store(hwnd.0 as isize, Ordering::Relaxed);
        return false.into(); // 찾았으니 멈춘다
    }
    true.into()
}

/// 메시지 1회 처리, 훅 콜백이 이 큐로 도착
fn pump() {
    unsafe {
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// 확장자 1개 [기본 앱 선택] 창 표시(성공 true), anchor = 숨긴 속성 창 좌표
pub fn show(ext: &str, anchor: (i32, i32)) -> bool {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return false; // 이미 1건 진행 중
    }
    // 앞서 띄운 것이 남아 있으면 먼저 치운다
    finish();

    let ext = ext.to_string();
    // 전용 스레드, 훅 콜백이 이 스레드 큐로 도착
    let ok = std::thread::spawn(move || unsafe { run(&ext, anchor) })
        .join()
        .unwrap_or(false);

    RUNNING.store(false, Ordering::SeqCst);
    ok
}

unsafe fn run(ext: &str, anchor: (i32, i32)) -> bool {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let temp = std::env::temp_dir().join(format!("zipmania-assoc-{stamp}.{ext}"));
    if std::fs::File::create(&temp).is_err() {
        return false;
    }

    *lock(&SHEET_NAME) = temp
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    *lock(&SHEET_ANCHOR) = anchor;
    SHEET_HWND.store(0, Ordering::Relaxed);

    // 훅 선등록, 창 생성 순간부터 수신해야 깜빡임 없음
    let hook = SetWinEventHook(
        EVENT_OBJECT_CREATE,
        EVENT_OBJECT_SHOW,
        None,
        Some(on_win_event),
        0,
        0,
        WINEVENT_OUTOFCONTEXT,
    );

    let path: Vec<u16> = temp
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let mut sei = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_INVOKEIDLIST,
        lpVerb: w!("properties"),
        lpFile: PCWSTR(path.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    if ShellExecuteExW(&mut sei).is_err() {
        let _ = UnhookWinEvent(hook);
        let _ = std::fs::remove_file(&temp);
        return false;
    }

    // 훅이 잡을 때까지 메시지 처리(최대 5초), EnumWindows 는 보조 수단
    let deadline = Instant::now() + Duration::from_secs(5);
    while SHEET_HWND.load(Ordering::Relaxed) == 0 && Instant::now() < deadline {
        pump();
        let _ = EnumWindows(Some(enum_find), LPARAM(0));
        std::thread::sleep(Duration::from_millis(5));
    }

    let raw = SHEET_HWND.load(Ordering::Relaxed);
    if raw == 0 {
        let _ = UnhookWinEvent(hook);
        let _ = std::fs::remove_file(&temp);
        return false; // 호출부가 설정 앱으로 폴백
    }
    let hwnd = HWND(raw as *mut c_void);

    let _ = PostMessageW(
        Some(hwnd),
        WM_COMMAND,
        WPARAM(IDM_CHANGE_ASSOC),
        LPARAM(0),
    );

    // 셸의 속성 창 재표시 — 훅 유지 상태로 잠깐 더 돌며 계속 제거
    let settle = Instant::now() + Duration::from_millis(700);
    while Instant::now() < settle {
        pump();
        vanish(hwnd);
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = UnhookWinEvent(hook);

    // 뒷정리는 나중에 — 선택 창의 종료 시점 불명(finish)
    *lock(&TEMP_PATH) = Some(temp);
    true
}

/// 숨긴 속성 창, 임시 파일 정리 + 셸 통지, 창 재활성화 시 호출
pub fn finish() {
    let raw = SHEET_HWND.swap(0, Ordering::Relaxed);
    let temp = lock(&TEMP_PATH).take();
    if raw == 0 && temp.is_none() {
        return;
    }

    unsafe {
        if raw != 0 {
            let h = HWND(raw as *mut c_void);
            if IsWindow(Some(h)).as_bool() {
                let _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
    }
    if let Some(p) = temp {
        let _ = std::fs::remove_file(p);
    }
    crate::file_assoc::notify_shell();
}
