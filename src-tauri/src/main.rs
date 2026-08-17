// Windows 릴리스 빌드의 콘솔 창 억제
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod amsi;
mod assoc_icon;
mod assoc_picker;
mod cli;
mod commands;
mod jobs;
mod models;
mod settings;
mod file_assoc;
mod maintenance;
mod shell_reg;
mod shelldrag;
mod sysicon;
mod update;
mod wintheme;

use jobs::JobManager;
use tauri::Manager;

fn main() {
    // /inst, /uninst 는 창 없이 처리하고 끝낸다, Tauri 를 띄우기 전에 가로챌 것
    let argv: Vec<String> = std::env::args().collect();
    if let Some(code) = maintenance::run(&argv) {
        std::process::exit(code);
    }

    tauri::Builder::default()
        // 탐색기 통합: 파일당 argv 를 첫 인스턴스로 포워딩, single-instance 를 먼저 등록
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            cli::handle(app, argv);
        }))
        // 파일/폴더 선택 다이얼로그 플러그인 (툴바 [열기], 해제 대상 폴더 선택)
        .plugin(tauri_plugin_dialog::init())
        // 메인 창이 닫히면 압축 창도 닫는다(부모-소유로 자동 파괴되지만 중복 안전)
        .setup(|app| {
            // 셸에서 기능 창(압축/풀기)만 띄우려는 실행이면 메인 창을 숨긴 채 둔다
            let argv: Vec<String> = std::env::args().collect();
            let cli_mode = cli::is_function_launch(&argv);
            app.manage(cli::CliMode::new(cli_mode));

            if let Some(main_window) = app.get_webview_window("main") {
                // 다크 캡션 강제(숨겨서 시작)
                wintheme::apply_window_chrome(&main_window);
                // 기능 창 전용 실행(cli_mode)에서는 메인 창을 표시하지 않는다
                if !cli_mode {
                    // 인자 없는 실행 = 화면 가운데, 탐색기 열기는 위치를 Windows 에 위임(겹침 방지)
                    if argv.len() <= 1 {
                        let _ = main_window.center();
                    }
                    let _ = main_window.show();
                    let _ = main_window.set_focus();
                }
                let app_handle = app.handle().clone();
                main_window.on_window_event(move |event| {
                    if matches!(
                        event,
                        tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed
                    ) {
                        if let Some(compress) = app_handle.get_webview_window("compress") {
                            let _ = compress.destroy();
                        }
                        if let Some(extract) = app_handle.get_webview_window("extract") {
                            let _ = extract.destroy();
                        }
                        if let Some(settings) = app_handle.get_webview_window("settings") {
                            let _ = settings.destroy();
                        }
                    }
                });
            }
            // 첫 인스턴스 argv 도 같은 라우터로, 포워딩 argv 와 동일 경로
            cli::handle_startup(app.handle(), argv);
            // 업데이트 확인(앱 시작마다 1회), 백그라운드 스레드에서 돌아 시작을 지연시키지 않는다
            update::spawn(app.handle());
            // 셸 확장 등록 상태를 설정과 동기화(exe, DLL 경로 변경, 포터블 이동 대응)
            {
                let handle = app.handle();
                let dll = commands::shellext_dll_path(handle);
                let (s, trusted) = settings::load_checked(handle);
                // 설정을 읽지 못했으면 건드리지 않는다, 빈 목록 = 모른다이지 끄겠다가 아니다
                if trusted {
                    shell_reg::sync(s.shell_integration, &dll);
                    // 파일 연결도 재기록 — 포터블 이동 시 등록 exe 경로 어긋남
                    let _ = file_assoc::sync(&s.file_assoc, &update::language(handle));
                }
            }
            Ok(())
        })
        // 작업 관리자 등록
        .manage(JobManager::new())
        // 새 압축 창으로 넘길 초기 입력 목록 보관소
        .manage(commands::PendingCompressInputs::default())
        // 창별 세션 id(소유 판단), label 은 재사용 대상
        .manage(commands::WindowSessions::default())
        // 새 압축 풀기 창으로 넘길 초기 컨텍스트(아카이브, 선택 항목) 보관소
        .manage(commands::PendingExtractContext::default())
        // 시작 시 --open 으로 받은, 메인 창이 열어야 할 아카이브 보관소
        .manage(commands::PendingStartupOpen::default())
        // 중첩 아카이브를 여는 독립 뷰어 창들의 label 번호 + 창별 초기 경로 보관소
        .manage(commands::ViewerWindows::default())
        // 세션 아카이브 암호 보관소(모든 창, 드래그가 공유, 재입력 방지)
        .manage(commands::SessionPassword::default())
        // 세션 임시 루트(%TEMP%\Ara_<랜덤>), 프로그램 시작 시 1회 생성, 종료 시 통째로 삭제
        .manage(commands::TempRoot::new())
        // 탐색기 통합: 파일당 실행의 argv 를 모으는 취합 버퍼
        .manage(cli::Aggregator::default())
        // 화면이 뜨기 전에 도착한 업데이트 공지 보관소
        .manage(update::PendingNotify::default())
        // IPC command 등록
        .invoke_handler(tauri::generate_handler![
            commands::sevenzip_version,
            commands::open_archive,
            commands::check_conflicts,
            commands::extract,
            commands::start_test,
            commands::start_scan,
            commands::start_edit,
            commands::create_archive,
            commands::cancel_job,
            commands::file_icon,
            commands::read_entry_preview,
            commands::stat_paths,
            commands::list_folder_files,
            commands::list_dir_children,
            commands::list_quick_access,
            commands::create_directory,
            commands::open_compress_window,
            commands::lease_compress_launch,
            commands::dispatch_compress_launch,
            commands::ack_compress_launch,
            commands::window_session,
            commands::peek_compress_standalone,
            commands::plan_each_compress,
            commands::open_extract_window,
            commands::take_extract_context,
            commands::take_startup_open,
            commands::open_archive_window,
            commands::take_viewer_archive,
            commands::delete_file,
            commands::open_folder,
            commands::open_entry,
            commands::open_default_apps,
            commands::open_default_app_picker,
            commands::finish_default_app_picker,
            commands::sync_file_assoc,
            commands::file_assoc_status,
            commands::default_assoc_exts,
            commands::reveal_file,
            commands::begin_shell_drag,
            commands::get_settings,
            commands::save_settings,
            commands::sync_shell_integration,
            commands::open_settings_window,
            update::open_update_url,
            update::get_update_notify
        ])
        .build(tauri::generate_context!())
        .expect("ZipMania 실행 중 오류가 발생했습니다")
        // 이벤트 루프 종료 시 세션 임시 루트(Ara_<랜덤>) 통째 삭제
        .run(|app_handle, event| {
            if let tauri::RunEvent::Exit = event {
                // 워커를 끊고 나가지 않는다, 신원 스냅샷과 취소는 한 호출(begin_shutdown)
                let running = app_handle.state::<JobManager>().begin_shutdown();
                let retired = app_handle
                    .state::<JobManager>()
                    .wait_retired(std::time::Duration::from_secs(5));
                if !retired {
                    // 미퇴장 → 종료는 미차단 + 파일 기록, 배포본은 콘솔 부재
                    eprintln!("종료: 작업이 제때 물러나지 않아 임시 파일이 남았을 수 있습니다.");
                    // 남은 임시 파일의 정확한 경로 병기(그 목록이 곧 잔존물)
                    note_shutdown_timeout(&running, &zipmania_archive::outfile::live_temp_paths());
                }
                commands::cleanup_temp_root(app_handle);
            }
        });
}

/// 종료 대기 초과를 설정 파일 옆 ZipMania-종료기록.log 에 기록
/// 작업 종류, 세션, 대상 경로 + 살아 있는 임시 파일 경로, 그 목록으로 아무것도 지우지 않는다(D3.5)
fn note_shutdown_timeout(running: &[(String, jobs::JobInfo)], live: &[std::path::PathBuf]) {
    use std::fmt::Write as _;
    use std::io::Write as _;
    let Ok(settings) = crate::settings::settings_path_headless() else {
        eprintln!("종료 기록: 설정 경로를 알 수 없어 남기지 못했습니다.");
        return;
    };
    let Some(dir) = settings.parent() else { return };

    // 메모장에서도 줄이 맞도록 CRLF 기록
    const NL: &str = "\r\n";
    let mut line = String::new();
    let _ = write!(line, "종료할 때 작업이 5초 안에 물러나지 않았습니다.{NL}");
    if running.is_empty() {
        let _ = write!(line, "  (진행 중이던 작업 정보를 얻지 못했습니다.){NL}");
    } else {
        let _ = write!(line, "  진행 중이던 작업:{NL}");
        for (id, info) in running {
            let _ = write!(
                line,
                "  - {id} ({}, 창 {}) 대상: {}{NL}",
                info.kind, info.owner, info.target
            );
        }
    }
    // 남은 임시 파일은 이 목록이 정확하다, 대상 경로만으로는 찾을 수 없어 그대로 적는다
    if live.is_empty() {
        let _ = write!(line, "  남은 `.zmtmp-` 임시 파일: 없음{NL}");
    } else {
        let _ = write!(line, "  남은 `.zmtmp-` 임시 파일(직접 지우셔도 됩니다):{NL}");
        for p in live {
            let _ = write!(line, "  - {}{NL}", p.display());
        }
    }

    let path = dir.join("ZipMania-종료기록.log");
    let wrote = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| f.write_all(line.as_bytes()));
    // 기록 실패 무시 금지, 남길 곳의 부재 자체가 정보
    if let Err(e) = wrote {
        eprintln!("종료 기록을 남기지 못했습니다({}): {e}", path.display());
    }
}
