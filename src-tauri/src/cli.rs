//! 탐색기 통합 CLI 라우팅, ZipMania.exe --<스위치> "<경로>" → 압축/해제, 열기
//! 다중 선택 = 파일당 프로세스 → single-instance 포워딩 → ~300ms 디바운스로 1회 개창(D3.7)

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use tauri::{AppHandle, Manager};

/// 셸 기능 창 전용 실행인가(managed state), 참 = 메인 창 숨김, 그 창이 닫히면 종료
pub struct CliMode(AtomicBool);

impl CliMode {
    pub fn new(v: bool) -> Self {
        Self(AtomicBool::new(v))
    }
    pub fn get(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// argv 가 기능 창 셸 실행인가(--open, 일반 실행 = false)
pub fn is_function_launch(argv: &[String]) -> bool {
    match parse(argv) {
        Some((v, paths)) if !paths.is_empty() => matches!(
            v,
            Verb::CompressZip
                | Verb::Compress
                | Verb::CompressEach
                | Verb::ExtractHere
                | Verb::ExtractSmart
                | Verb::ExtractNewFolder
                | Verb::Extract
                | Verb::ExtractEachSmart
                | Verb::ExtractEachNewFolder
        ),
        _ => false,
    }
}

/// 취합 디바운스(ms)
const DEBOUNCE_MS: u64 = 300;

/// 탐색기 메뉴 동작, 스위치 대응은 parse 참조
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verb {
    CompressZip,
    Compress,
    ExtractHere,
    ExtractSmart,
    ExtractNewFolder,
    Extract,
    Open,
    CompressEach,
    ExtractEachSmart,
    ExtractEachNewFolder,
}

impl Verb {
    fn from_switch(s: &str) -> Option<Verb> {
        match s {
            "--compress-zip" => Some(Verb::CompressZip),
            "--compress" => Some(Verb::Compress),
            "--extract-here" => Some(Verb::ExtractHere),
            "--extract-smart" => Some(Verb::ExtractSmart),
            "--extract-newfolder" => Some(Verb::ExtractNewFolder),
            "--extract" => Some(Verb::Extract),
            "--open" => Some(Verb::Open),
            "--compress-each" => Some(Verb::CompressEach),
            "--extract-each-smart" => Some(Verb::ExtractEachSmart),
            "--extract-each-newfolder" => Some(Verb::ExtractEachNewFolder),
            _ => None,
        }
    }
}

/// argv → (동작, 경로들), 스위치 뒤 비-플래그 인자가 경로, 없으면 None
pub fn parse(argv: &[String]) -> Option<(Verb, Vec<String>)> {
    let mut verb = None;
    let mut paths = Vec::new();
    for a in argv.iter().skip(1) {
        if let Some(v) = Verb::from_switch(a) {
            verb = Some(v);
        } else if verb.is_some() && !a.starts_with("--") {
            paths.push(a.clone());
        }
    }
    verb.map(|v| (v, paths))
}

/// 취합 버퍼(managed state), 동작별 경로 수집 + 세대 카운터로 디바운스 판정
#[derive(Default)]
pub struct Aggregator(Mutex<Agg>);

#[derive(Default)]
struct Agg {
    verb: Option<Verb>,
    paths: Vec<String>,
    generation: u64,
    startup: bool,
}

/// 앱을 시작시킨 argv 처리(첫 인스턴스 setup 에서 1회)
pub fn handle_startup(app: &AppHandle, argv: Vec<String>) {
    handle_inner(app, argv, true);
}

/// 포워딩된 argv 처리(두 번째 이후 실행)
pub fn handle(app: &AppHandle, argv: Vec<String>) {
    handle_inner(app, argv, false);
}

/// argv 1개 처리, 파싱 → 버퍼링 → 디바운스 예약
fn handle_inner(app: &AppHandle, argv: Vec<String>, startup: bool) {
    let Some((verb, paths)) = parse(&argv) else {
        // 인자 없는 재실행(단일 인스턴스 두 번째 실행 등) → 메인 창만 앞으로
        focus_main(app);
        return;
    };
    if paths.is_empty() {
        focus_main(app);
        return;
    }

    let agg = app.state::<Aggregator>();
    let my_gen = {
        let mut g = agg.0.lock().unwrap_or_else(|e| e.into_inner());
        // 버퍼에 다른 동작이 남아 있으면 먼저 내보낸다(동작이 섞이면 즉시 분리 처리)
        if let Some(cur) = g.verb {
            if cur != verb {
                let drained = std::mem::take(&mut g.paths);
                let was_startup = std::mem::take(&mut g.startup);
                dispatch(app, cur, drained, was_startup);
            }
        }
        g.verb = Some(verb);
        g.paths.extend(paths);
        // 시작 argv 가 하나라도 섞이면 시작으로 판정
        g.startup |= startup;
        g.generation += 1;
        g.generation
    };

    // 디바운스: DEBOUNCE_MS 뒤 세대가 그대로면 flush, 타이머는 별도 스레드, 창 생성은 dispatch
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(DEBOUNCE_MS));
        let agg = app.state::<Aggregator>();
        let (verb, paths, startup) = {
            let mut g = agg.0.lock().unwrap_or_else(|e| e.into_inner());
            if g.generation != my_gen {
                return; // 그 사이 새 유입 → 그쪽 타이머가 flush
            }
            (
                g.verb.take(),
                std::mem::take(&mut g.paths),
                std::mem::take(&mut g.startup),
            )
        };
        if let (Some(v), false) = (verb, paths.is_empty()) {
            dispatch(&app, v, paths, startup);
        }
    });
}

/// (동작, 경로들) → 실제 흐름 연결
fn dispatch(app: &AppHandle, verb: Verb, paths: Vec<String>, startup: bool) {
    match verb {
        Verb::CompressZip => {
            // 즉시 zip: 출력 경로 선계산 → 압축 창 자동 모드(폼 생략)
            let output = zip_output_path(&paths);
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let _ = crate::commands::open_compress_window(
                    app,
                    paths,
                    Some("zip".to_string()),
                    output,
                    Some(true),
                    None,
                )
                .await;
            });
        }
        Verb::Compress => {
            // 포맷, 옵션 선택 폼
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let _ = crate::commands::open_compress_window(
                    app,
                    paths,
                    None,
                    None,
                    Some(false),
                    None,
                )
                .await;
            });
        }
        Verb::CompressEach => {
            // 각각 압축: 원본마다 자기 이름 zip(충돌 회피) → 배치
            let items: Vec<crate::models::CompressBatchItem> = paths
                .iter()
                .filter_map(|p| {
                    zip_output_path(std::slice::from_ref(p)).map(|output| {
                        crate::models::CompressBatchItem {
                            input: p.clone(),
                            output,
                        }
                    })
                })
                .collect();
            if items.is_empty() {
                return;
            }
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let _ = crate::commands::open_compress_window(
                    app,
                    paths,
                    Some("zip".to_string()),
                    None,
                    Some(false),
                    Some(items),
                )
                .await;
            });
        }
        Verb::ExtractHere => {
            // 여기에 풀기: 각 아카이브를 자기 부모 폴더에, 다중 = 순차 배치
            if paths.len() > 1 {
                let items = paths
                    .iter()
                    .map(|a| crate::models::ExtractBatchItem {
                        archive: a.clone(),
                        dest: parent_dir(a),
                    })
                    .collect();
                open_extract_batch(app, items);
            } else if let Some(a) = paths.into_iter().next() {
                let dest = parent_dir(&a);
                open_extract_single(app, a, Some(dest), true);
            }
        }
        Verb::ExtractNewFolder => {
            // {이름}에 풀기: 단일=<부모>/<이름>, 다중=모두 <부모>/<현재폴더명> 한 폴더에
            if paths.len() > 1 {
                let dest = current_folder_dest(&paths[0]);
                let items = paths
                    .iter()
                    .map(|a| crate::models::ExtractBatchItem {
                        archive: a.clone(),
                        dest: dest.clone(),
                    })
                    .collect();
                open_extract_batch(app, items);
            } else if let Some(a) = paths.into_iter().next() {
                let dest = new_folder_dest(&a);
                open_extract_single(app, a, Some(dest), true);
            }
        }
        Verb::Extract => {
            // 대상 폴더 선택 폼(첫 아카이브)
            if let Some(a) = paths.into_iter().next() {
                open_extract_single(app, a, None, false);
            }
        }
        Verb::ExtractSmart => {
            // (메뉴에서 제거됨 — 혹시 호출되면 단일 스마트로 처리)
            if let Some(a) = paths.into_iter().next() {
                let dest = match archive_root_count(app, &a) {
                    Some(1) => parent_dir(&a),
                    _ => new_folder_dest(&a),
                };
                open_extract_single(app, a, Some(dest), true);
            }
        }
        Verb::ExtractEachNewFolder | Verb::ExtractEachSmart => {
            // 각각 풀기: 아카이브마다 자기 <이름> 폴더로(순차 배치)
            let items = paths
                .iter()
                .map(|a| crate::models::ExtractBatchItem {
                    archive: a.clone(),
                    dest: new_folder_dest(a),
                })
                .collect();
            open_extract_batch(app, items);
        }
        Verb::Open => {
            // 경로마다 창 1개, 시작 클릭이면 첫 아카이브만 메인 창, 아니면 전부 새 창
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let mut first = startup;
                for archive in paths {
                    crate::commands::open_from_shell(&app, archive, first).await;
                    first = false;
                }
            });
        }
    }
}

/// 단일 아카이브 해제 창(dest + 자동시작 여부)
fn open_extract_single(app: &AppHandle, archive: String, dest: Option<String>, auto: bool) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _ = crate::commands::open_extract_window(app, archive, Vec::new(), dest, Some(auto), None)
            .await;
    });
}

/// 여러 아카이브 순차 배치 해제 창
fn open_extract_batch(app: &AppHandle, items: Vec<crate::models::ExtractBatchItem>) {
    if items.is_empty() {
        return;
    }
    let first = items[0].archive.clone();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _ = crate::commands::open_extract_window(
            app,
            first,
            Vec::new(),
            None,
            Some(true),
            Some(items),
        )
        .await;
    });
}

/// 다중 선택 {현재폴더명}에 풀기 대상 = <부모>/<현재 폴더명>
fn current_folder_dest(archive: &str) -> String {
    let parent = parent_dir(archive);
    let folder = parent_folder_name(archive);
    if folder.is_empty() {
        parent
    } else {
        Path::new(&parent).join(folder).to_string_lossy().to_string()
    }
}

/// 경로가 든 폴더 이름, 없으면 빈 문자열
fn parent_folder_name(p: &str) -> String {
    Path::new(p)
        .parent()
        .and_then(|d| d.file_name())
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// 메인 창 복원, 표시, 포커스
fn focus_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// 경로의 부모 폴더, 없으면 빈 문자열
fn parent_dir(p: &str) -> String {
    Path::new(p)
        .parent()
        .map(|d| d.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// <부모>/<확장자 하나 제거한 이름> (예: backup.tar.gz → backup.tar)
fn new_folder_dest(p: &str) -> String {
    let path = Path::new(p);
    let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "extracted".to_string());
    parent.join(stem).to_string_lossy().to_string()
}

/// 즉시 zip 출력 경로, 단일 = <이름>.zip, 다중 = 공통 부모 폴더명, 충돌 시 이름 (2).zip
fn zip_output_path(inputs: &[String]) -> Option<String> {
    let first = inputs.first()?;
    let p = Path::new(first);
    let parent = p.parent()?.to_path_buf();
    let base = if inputs.len() == 1 {
        p.file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "archive".to_string())
    } else {
        parent
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "archive".to_string())
    };
    let mut candidate = parent.join(format!("{base}.zip"));
    let mut n = 2;
    while candidate.exists() {
        candidate = parent.join(format!("{base} ({n}).zip"));
        n += 1;
    }
    Some(candidate.to_string_lossy().to_string())
}

/// 아카이브 최상위 항목 수, 첫 경로 조각의 고유 개수, 열기 실패 = None
fn archive_root_count(app: &AppHandle, archive: &str) -> Option<usize> {
    let entries = crate::commands::list_archive(app, archive, None).ok()?;
    let mut roots: HashSet<String> = HashSet::new();
    for e in &entries {
        let norm = e.path.replace('\\', "/");
        if let Some(first) = norm.split('/').find(|s| !s.is_empty()) {
            roots.insert(first.to_string());
        }
    }
    Some(roots.len())
}
