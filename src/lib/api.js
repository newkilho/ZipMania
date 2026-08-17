// Tauri 백엔드 호출 단일 창구
// UI 는 @tauri-apps/api 직접 import 금지, 이 파일만 경유

import { invoke } from "@tauri-apps/api/core";
import { listen, emit } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { LogicalSize, LogicalPosition } from "@tauri-apps/api/dpi";
import { Menu, MenuItem, PredefinedMenuItem } from "@tauri-apps/api/menu";
import {
  open as openFileDialog,
  save as saveFileDialog,
  confirm as confirmDialog,
} from "@tauri-apps/plugin-dialog";

/**
 * 7z 버전 문자열 조회
 * @returns {Promise<string>} 예: "7-Zip 26.02 (x64)"
 */
export async function sevenzipVersion() {
  return await invoke("sevenzip_version");
}

/**
 * 아카이브 항목 전체 목록, 실패 = { code, message } throw
 * @param {string} path 아카이브 절대 경로
 * @param {string} password 파일명 암호화 아카이브 재시도용 암호
 * @returns {Promise<Array>} ArchiveEntry 배열 (camelCase: path/size/packedSize/modified/isDir/crc)
 */
export async function openArchive(path, password) {
  // password 없으면 인자 자체를 넘기지 않는다(백엔드 None)
  const args = password != null ? { path, password } : { path };
  return await invoke("open_archive", args);
}

/**
 * 무결성 테스트 job 시작 → job_id 즉시 반환, 진행률 job:progress, 결과 test:report, 오류 job:error
 * @param {string} archive 아카이브 절대 경로
 * @param {string} password 암호(옵션), 부재 시 백엔드가 세션 암호로 폴백
 * @returns {Promise<string>} job_id
 */
export async function startTest(archive, password) {
  const args = password != null ? { archive, password } : { archive };
  args.session = await windowSession();
  return await invoke("start_test", args);
}

/**
 * test:report 구독, payload = { jobId, entries: [{ path, isDir, expectedCrc, actualCrc, ok }] }
 * @param {(payload: object) => void} handler
 * @returns {Promise<() => void>} 구독 해제 함수
 */
export async function onTestReport(handler) {
  return await listen("test:report", (e) => handler(e.payload));
}

/**
 * AMSI 바이러스 검사 job 시작 → job_id 즉시 반환, 진행률 job:progress, 결과 scan:report, 오류 job:error
 * @param {string} archive 아카이브 절대 경로
 * @param {string} password 암호(옵션), 부재 시 백엔드가 세션 암호로 폴백
 * @returns {Promise<string>} job_id
 */
export async function startScan(archive, password) {
  const args = password != null ? { archive, password } : { archive };
  args.session = await windowSession();
  return await invoke("start_scan", args);
}

/**
 * scan:report 구독, payload = { jobId, entries: [{ path, isDir, size, status }] }, status = clean|malware|error|skipped
 * @param {(payload: object) => void} handler
 * @returns {Promise<() => void>} 구독 해제 함수
 */
export async function onScanReport(handler) {
  return await listen("scan:report", (e) => handler(e.payload));
}

/**
 * 확장자와 폴더 여부 → Windows 시스템 아이콘 PNG 데이터 URI, 목록 UI 의 탐색기 동일 16x16
 * @param {string} ext 확장자(소문자, 점 없음, 예: "txt"), 폴더/확장자 없음이면 빈 문자열
 * @param {boolean} isDir 폴더 아이콘 여부
 * @returns {Promise<string|null>} "data:image/png;base64,..." 데이터 URI, 실패 시 null
 */
export async function fileIcon(ext, isDir) {
  return await invoke("file_icon", { ext, isDir });
}

/**
 * 내부 이미지 1개 → 메모리 → data URI, 미리보기 패널용
 * @param {string} archive 열린 아카이브 절대 경로
 * @param {string} innerPath 내부 파일 경로("/" 정규화)
 * @param {string} password 파일 암호화 아카이브용 암호
 * @returns {Promise<string>} "data:image/...;base64,..." 데이터 URI
 */
export async function readEntryPreview(archive, innerPath, password) {
  const args = { archive, innerPath };
  if (password != null) args.password = password;
  return await invoke("read_entry_preview", args);
}

/**
 * 파일 선택 다이얼로그로 아카이브 경로 선택
 * @returns {Promise<string|null>} 선택한 경로, 취소 시 null
 */
export async function pickArchiveFile() {
  const selected = await openFileDialog({
    multiple: false,
    directory: false,
    title: "아카이브 열기",
    filters: [
      {
        // 정본은 crates/zipmania-archive 의 READ_EXTS (D3.8)
        // 어긋나면 열기필터_목록이_정본과_일치 실패
        name: "압축 파일",
        extensions: [
          "7z", "zip", "zipx", "jar", "rar", "r00", "arj", "lzh", "lha", "cab",
          "tar", "ova", "gz", "gzip", "tgz", "tpz", "bz2", "bzip2", "tbz", "tbz2",
          "xz", "txz", "zst", "tzst", "z", "taz", "lzma",
          "iso", "img", "udf", "wim", "swm", "esd", "dmg", "squashfs",
          "msi", "msp", "msm", "cpio", "rpm", "deb", "xar", "pkg", "chm", "nsis",
          "001", "cbz", "cbr", "cb7", "egg", "alz",
        ],
      },
      { name: "모든 파일", extensions: ["*"] },
    ],
  });
  // multiple:false 이면 문자열 또는 null
  return typeof selected === "string" ? selected : null;
}

/**
 * 창 전체 파일 드래그&드롭 구독(Tauri v2 onDragDropEvent), 창 어디에 드롭해도 호출
 * @param {(paths: string[]) => void} handler 드롭된 파일 경로 배열
 * @returns {Promise<() => void>} 구독 해제 함수
 */
export async function onFileDrop(handler) {
  return await getCurrentWebview().onDragDropEvent((event) => {
    if (event.payload.type === "drop") {
      handler(event.payload.paths ?? []);
    }
  });
}

/**
 * 폴더 선택 다이얼로그로 해제 대상 선택
 * @param {string} defaultPath 기본으로 열 경로
 * @returns {Promise<string|null>} 선택한 폴더 경로, 취소 시 null
 */
export async function pickFolder(defaultPath) {
  const selected = await openFileDialog({
    multiple: false,
    directory: true,
    title: "해제할 폴더 선택",
    defaultPath: defaultPath || undefined,
  });
  return typeof selected === "string" ? selected : null;
}

/**
 * 인라인 폴더 브라우저용 하위 디렉터리 목록, path 없으면 루트, 파일 제외
 * @param {string} path 자식을 조회할 폴더 경로(없으면 루트)
 * @returns {Promise<Array<{name:string, path:string, hasChildren:boolean}>>}
 */
export async function listDirChildren(path) {
  const args = path != null && path !== "" ? { path } : {};
  return await invoke("list_dir_children", args);
}

/**
 * 폴더 브라우저 바로가기(즐겨찾기 + 드라이브) 목록
 * @returns {Promise<Array<{kind:string, name:string, path:string}>>}
 *  kind: "desktop"|"documents"|"downloads"|"home"|"drive"
 */
export async function listQuickAccess() {
  return await invoke("list_quick_access");
}

/**
 * [새 폴더], parent 아래 name 생성 → 새 경로 반환
 * @param {string} parent 상위 폴더 경로
 * @param {string} name 새 폴더 이름
 * @returns {Promise<string>} 생성된 폴더의 절대 경로
 */
export async function createDirectory(parent, name) {
  return await invoke("create_directory", { parent, name });
}

/**
 * 압축 원본 파일 다중 선택 다이얼로그(폴더는 드래그&드롭으로)
 * @returns {Promise<string[]>} 선택한 파일 경로들(취소 시 빈 배열)
 */
export async function pickInputFiles() {
  const selected = await openFileDialog({
    multiple: true,
    directory: false,
    title: "압축할 파일 선택",
  });
  if (Array.isArray(selected)) return selected;
  return typeof selected === "string" ? [selected] : [];
}

/**
 * 압축 원본 폴더 다중 선택 다이얼로그
 * @returns {Promise<string[]>} 선택한 폴더 경로들(취소 시 빈 배열)
 */
export async function pickInputFolders() {
  const selected = await openFileDialog({
    multiple: true,
    directory: true,
    title: "압축할 폴더 선택",
  });
  if (Array.isArray(selected)) return selected;
  return typeof selected === "string" ? [selected] : [];
}

/**
 * 아카이브 저장 위치, 이름 선택 다이얼로그
 * @param {string} defaultPath 기본 경로/파일명
 * @returns {Promise<string|null>} 저장 경로(취소 시 null)
 */
export async function pickSaveArchive(defaultPath) {
  const selected = await saveFileDialog({
    title: "아카이브 저장",
    defaultPath: defaultPath || undefined,
    filters: [
      { name: "7z 아카이브", extensions: ["7z"] },
      { name: "ZIP 아카이브", extensions: ["zip"] },
      { name: "TAR 아카이브", extensions: ["tar"] },
    ],
  });
  return typeof selected === "string" ? selected : null;
}

/**
 * 경로들의 표시용 메타데이터(이름, 크기, 폴더 여부), 디렉터리는 재귀 없이 size=0, isDir=true
 * @param {string[]} paths 조회할 절대 경로 목록
 * @returns {Promise<Array<{name:string, path:string, size:number, isDir:boolean}>>}
 *  입력 순서, 개수를 보존한 메타데이터 목록
 */
export async function statPaths(paths) {
  return await invoke("stat_paths", { paths: paths ?? [] });
}

/**
 * 폴더 하위 파일 목록(재귀) → 상대 경로, 크기, 파일 경로 = 빈 배열
 * @param {string} path 폴더 절대 경로
 * @returns {Promise<Array<{rel:string, size:number}>>}
 */
export async function listFolderFiles(path) {
  return await invoke("list_folder_files", { path });
}

/**
 * 아카이브 생성 → job_id 즉시 반환, 진행 = job:progress / job:done / job:error
 * @param {object} opts
 * @param {string} opts.output 생성할 아카이브 경로
 * @param {string[]} opts.inputs 압축할 원본 절대 경로 목록
 * @param {string} opts.format "7z" | "zip" | "tar"
 * @param {number} opts.level 압축 레벨 (0/1/3/5/7/9)
 * @param {string} opts.password 암호(옵션, TAR 은 무시)
 * @param {boolean} [opts.encryptNames] 파일명 암호화(7z + 암호일 때만)
 * @returns {Promise<string>} job_id
 */
export async function createArchive({ output, inputs, format, level, password, encryptNames }) {
  const args = {
    output,
    inputs: inputs ?? [],
    format,
    level,
    encryptNames: !!encryptNames,
  };
  if (password != null && password !== "") args.password = password;
  args.session = await windowSession();
  return await invoke("create_archive", args);
}

/**
 * 해제 전 충돌 검사 → 대상 폴더에 이미 있는 내부 경로 목록
 * @param {object} opts
 * @param {string} opts.archive 아카이브 경로
 * @param {string} opts.dest 대상 폴더
 * @param {boolean} opts.keepPaths 경로 유지 여부
 * @param {string[]} opts.selected 선택 내부 경로 목록(빈 배열=전체)
 * @param {string} opts.password 암호(옵션)
 * @returns {Promise<string[]>} 충돌하는 내부 경로 목록
 */
export async function checkConflicts({ archive, dest, keepPaths, selected, password }) {
  const args = { archive, dest, keepPaths, selected: selected ?? [] };
  if (password != null) args.password = password;
  return await invoke("check_conflicts", args);
}

/**
 * 아카이브 해제 → job_id 즉시 반환, 진행 = job:progress / job:done / job:error
 * @param {object} opts
 * @param {string} opts.archive 아카이브 경로
 * @param {string} opts.dest 대상 폴더
 * @param {string[]} opts.selected 선택 내부 경로 목록(빈 배열=전체)
 * @param {boolean} opts.keepPaths 경로 유지(true) / 평면(false)
 * @param {string} opts.overwrite 기본 정책 "overwrite" | "skip" | "rename"
 * @param {Record<string,string>} opts.decisions 파일별 정책(내부 경로 → 위 문자열)
 *  지정된 파일은 기본 정책 대신 이 값을 따른다
 * @param {string} opts.password 암호(옵션)
 * @returns {Promise<string>} job_id
 */
export async function extract({ archive, dest, selected, keepPaths, overwrite, decisions, password }) {
  const args = {
    archive,
    dest,
    selected: selected ?? [],
    keepPaths,
    overwrite,
  };
  if (decisions && Object.keys(decisions).length > 0) args.decisions = decisions;
  if (password != null) args.password = password;
  args.session = await windowSession();
  return await invoke("extract", args);
}

/**
 * 아카이브 편집(추가/삭제) job → job_id 즉시 반환, job:progress / job:done / job:error
 * @param {string} archive 아카이브 절대 경로
 * @param {string[]} add 추가할 원본 절대 경로(파일/폴더), 없으면 []
 * @param {string[]} remove 삭제할 내부 경로, 없으면 []
 * @param {string} password 암호(옵션), 부재 시 백엔드가 세션 암호로 폴백
 * @returns {Promise<string>} job_id
 */
export async function startEdit(archive, add, remove, password) {
  const args = { archive, add: add ?? [], remove: remove ?? [] };
  if (password != null) args.password = password;
  args.session = await windowSession();
  return await invoke("start_edit", args);
}

/**
 * 작업 취소, 완료는 job:done(status=canceled)
 * @param {string} jobId
 * @returns {Promise<void>}
 */
export async function cancelJob(jobId) {
  return await invoke("cancel_job", { jobId });
}

/**
 * 작업 진행률 구독
 * @param {(payload: {jobId:string, percent:number, currentFile:string}) => void} handler
 * @returns {Promise<() => void>} 구독 해제 함수
 */
/**
 * 풀기 창이 열린 상태의 새 해제 요청 신호, 받으면 takeExtractContext() 로 회수해 재시작
 * @param {() => void} handler
 * @returns {Promise<() => void>} 구독 해제 함수
 */
export async function onExtractContext(handler) {
  return await listen("extract:context", () => handler());
}

export async function onJobProgress(handler) {
  return await listen("job:progress", (event) => handler(event.payload));
}

/**
 * 작업 완료 구독
 * @param {(payload: {jobId:string, status:string, message:string}) => void} handler
 * @returns {Promise<() => void>} 구독 해제 함수
 */
export async function onJobDone(handler) {
  return await listen("job:done", (event) => handler(event.payload));
}

/**
 * 작업 오류 구독
 * @param {(payload: {jobId:string, code:string, message:string}) => void} handler
 * @returns {Promise<() => void>} 구독 해제 함수
 */
export async function onJobError(handler) {
  return await listen("job:error", (event) => handler(event.payload));
}

/**
 * 작업 시작 구독, create/extract 가 job_id 반환 직전 전 창에 발행, 다른 창의 작업도 메인이 표시
 * @param {(payload: {jobId:string, kind:string}) => void} handler kind: "compress"|"extract"
 * @returns {Promise<() => void>} 구독 해제 함수
 */
export async function onJobStarted(handler) {
  return await listen("job:started", (event) => handler(event.payload));
}

// ─── 별도 압축 창(WebviewWindow "compress") ────────────────

/**
 * 압축 다이얼로그를 별도 네이티브 창으로 연다, 이미 열려 있으면 입력만 추가 + 포커스
 * @param {string[]} inputs 새 창으로 넘길 초기 입력 경로들
 * @returns {Promise<void>}
 */
export async function openCompressWindow(inputs) {
  return await invoke("open_compress_window", { inputs: inputs ?? [] });
}

/**
 * 이 창의 세션 토큰(compress#3), 작업 시작 호출마다 실어 보낸다, 1회 조회 후 캐시
 * @returns {Promise<string|null>}
 */
let sessionPromise = null;
export function windowSession() {
  if (!sessionPromise) {
    sessionPromise = invoke("window_session").catch(() => {
      sessionPromise = null;
      return null;
    });
  }
  return sessionPromise;
}

/**
 * 보관된 요청 1개 대여(큐에서 빼는 것은 ackCompressLaunch), more 참이면 한가해질 때 재호출
 * @returns {Promise<{id:number, launch:({inputs:string[], format:(string|null), output:(string|null), autoStart:boolean, batch:Array<{input:string,output:string}>}|null), more:boolean}>}
 */
export async function leaseCompressLaunch() {
  return await invoke("lease_compress_launch", { session: await windowSession() });
}

/**
 * 대여 요청 마감(창에 반영한 뒤 호출), 결과 = ok / already(응답 유실 재시도) / stale(우리 것 아님)
 * @param {number} id 빌릴 때 받은 번호
 * @param {number} gen 빌릴 때 받은 세대(죽은 창의 늦은 마감을 가리는 값)
 * @returns {Promise<"ok"|"already"|"stale">}
 */
export async function ackCompressLaunch(id, gen) {
  return await invoke("ack_compress_launch", { id, gen });
}

/**
 * 실행 개시 통지, 적용 전에 부른다 — 창 사망 시 되돌림/버림을 가른다(D3.5)
 * @param {number} id 빌릴 때 받은 번호
 * @param {number} gen 빌릴 때 받은 세대
 * @returns {Promise<"ok"|"already"|"stale">}
 */
export async function dispatchCompressLaunch(id, gen) {
  return await invoke("dispatch_compress_launch", { id, gen });
}

/**
 * 큐 맨 앞 요청의 독립 작업 여부, 꺼내지 않고 조회만 — 소유권은 백엔드 큐에 잔존
 * @returns {Promise<boolean>}
 */
export async function peekCompressStandalone() {
  return await invoke("peek_compress_standalone");
}

/**
 * 각각 압축 출력 경로 계산(포맷별 확장자, 충돌 회피), 셸 메뉴와 같은 규약, 계산은 백엔드
 * @param {string[]} inputs 압축할 원본 절대 경로 목록
 * @param {string} format "zip"|"7z"|"tar"
 * @returns {Promise<Array<{input:string, output:string}>>}
 */
export async function planEachCompress(inputs, format) {
  return await invoke("plan_each_compress", { inputs, format });
}

/**
 * 시작 --open 아카이브 회수(1회), 값은 이벤트가 아니라 백엔드 보관소(PendingStartupOpen)
 * @returns {Promise<string|null>} 열 아카이브 절대 경로, 없으면 null
 */
export async function takeStartupOpen() {
  return await invoke("take_startup_open");
}

/**
 * 탐색기 --open 신호 구독(payload 없음, 값은 takeStartupOpen), mount 때도 1회 회수라 유실, 중복 없음
 * @param {() => void} handler 신호 처리기
 * @returns {Promise<() => void>} 구독 해제 함수
 */
export async function onOpenArchive(handler) {
  return await listen("shell:open-archive", () => handler());
}

/**
 * 압축 창 새 요청 신호 구독(compress:take-inputs), 값은 takeCompressInputs() 로
 * @param {() => void} handler 신호 처리기
 * @returns {Promise<() => void>} 구독 해제 함수
 */
export async function onCompressTakeInputs(handler) {
  return await listen("compress:take-inputs", () => handler());
}

/**
 * 현재 창 닫기
 * @returns {Promise<void>}
 */
export async function closeCurrentWindow() {
  return await getCurrentWindow().close();
}

/**
 * 현재 창 제목 변경
 * @param {string} title
 * @returns {Promise<void>}
 */
export async function setCurrentWindowTitle(title) {
  return await getCurrentWindow().setTitle(title);
}

/**
 * 현재 창 크기 변경(논리 픽셀), 최소 크기도 함께 낮춘다
 * @param {number} width
 * @param {number} height
 * @returns {Promise<void>}
 */
export async function resizeCurrentWindow(width, height) {
  const win = getCurrentWindow();
  await win.setMinSize(new LogicalSize(width, height));
  await win.setSize(new LogicalSize(width, height));
  // 크기 변경은 좌상단 기준이라 다시 중앙으로
  await win.center();
}

/**
 * 현재 창을 화면 중앙으로
 * @returns {Promise<void>}
 */
export async function centerCurrentWindow() {
  return await getCurrentWindow().center();
}

// ─── 환경설정(settings.toml — localStorage 캐시 없이 그때그때 읽는다) ────────────

/**
 * 현재 설정 읽기(테마, 언어, 해제 기본값, 최근 파일)
 * @returns {Promise<{theme:string, language:string, extract_create_subfolder:boolean, extract_delete_after:boolean, recent_files:string[]}>}
 */
export async function getSettings() {
  return await invoke("get_settings");
}

/**
 * 설정 저장
 * @param {object} settings 저장할 전체 설정 객체
 * @returns {Promise<void>}
 */
export async function saveSettings(settings) {
  return await invoke("save_settings", { settings });
}

/**
 * 환경설정 창 열기, 이미 열려 있으면 포커스만
 * @returns {Promise<void>}
 */
export async function openSettingsWindow() {
  return await invoke("open_settings_window");
}

/**
 * 셸 확장 레지스트리 반영(ON=등록, OFF=해제), 플래그 저장은 saveSettings
 * @param {boolean} enabled
 * @returns {Promise<void>}
 */
export async function syncShellIntegration(enabled) {
  return await invoke("sync_shell_integration", { enabled });
}

/**
 * 설정 변경을 모든 창에 방송
 * @param {object} settings 변경된 전체 설정
 * @returns {Promise<void>}
 */
export async function emitSettingsChanged(settings) {
  return await emit("settings:changed", settings);
}

/**
 * 설정 변경 구독, 테마, 언어 재적용에 사용
 * @param {(settings: object) => void} handler
 * @returns {Promise<() => void>} 구독 해제 함수
 */
export async function onSettingsChanged(handler) {
  return await listen("settings:changed", (event) => handler(event.payload));
}

// ─── 별도 압축 풀기 창(WebviewWindow "extract") ────────────

/**
 * 해제 옵션 창 열기, 이미 열려 있으면 포커스만
 * @param {string} archive 풀 대상 아카이브 경로
 * @param {string[]} selected 메인 창에서 선택돼 있던 내부 경로들(‘선택된 파일’ 옵션용)
 * @param {string} dest 빠른 해제 시 최종 대상 폴더(폼을 건너뛰고 이 폴더로 즉시 해제)
 * @param {boolean} autoStart 참이면 폼(폴더 선택) 생략 후 해제 과정부터 표시
 * @returns {Promise<void>}
 */
export async function openExtractWindow(archive, selected, dest, autoStart) {
  const args = { archive, selected: selected ?? [] };
  if (dest != null) args.dest = dest;
  if (autoStart != null) args.autoStart = autoStart;
  return await invoke("open_extract_window", args);
}

/**
 * 해제 창 mount 시 초기 컨텍스트 1회 회수
 * @returns {Promise<{archive: string, selected: string[]}|null>}
 */
export async function takeExtractContext() {
  return await invoke("take_extract_context");
}

/**
 * 파일 1개 삭제(해제 후 원본 삭제 옵션용)
 * @param {string} path 삭제할 파일 경로
 * @returns {Promise<void>}
 */
export async function deleteFile(path) {
  return await invoke("delete_file", { path });
}

/**
 * 업데이트 공지 구독, 앱 시작 시 서버 조회 후 1회 발행
 * @param {(payload: {url: string, text: string}) => void} handler
 * @returns {Promise<() => void>} 구독 해제 함수
 */
export async function onUpdateNotify(handler) {
  return await listen("update:notify", (event) => handler(event.payload));
}

/**
 * 보관된 업데이트 공지 조회(없으면 null), 화면이 뜨자마자 1회 호출
 * @returns {Promise<{url: string, text: string} | null>}
 */
export async function getUpdateNotify() {
  return await invoke("get_update_notify");
}

/**
 * 업데이트 공지 주소를 기본 브라우저로
 * @param {string} url http(s) 주소
 * @returns {Promise<void>}
 */
export async function openUpdateUrl(url) {
  return await invoke("open_update_url", { url });
}

/**
 * 폴더를 탐색기로 열기
 * @param {string} path 열 폴더 경로
 * @returns {Promise<void>}
 */
export async function openFolder(path) {
  return await invoke("open_folder", { path });
}

/**
 * 가상 파일 Shell DnD 시작, 드롭 순간 추출
 * @param {string} archive 열린 아카이브 경로
 * @param {string[]} innerPaths 드래그할 내부 경로(파일/폴더)
 * @param {string} password 암호(옵션)
 * @returns {Promise<void>}
 */
export async function beginShellDrag(archive, innerPaths, password) {
  const args = { archive, innerPaths };
  if (password != null) args.password = password;
  return await invoke("begin_shell_drag", args);
}

/**
 * 파일 연결 등록/해제(HKCU), 목록에서 빠진 것은 원래 연결로 복원
 * @param {string[]} exts 점 없는 소문자 확장자 목록
 * @returns {Promise<void>}
 */
export async function syncFileAssoc(exts) {
  return await invoke("sync_file_assoc", { exts });
}

/**
 * 확장자 연결 상태, registered(등록 여부)와 ours(지금 우리로 열리나)는 다른 값, hard = UserChoice 차단
 * @param {string[]} exts 점 없는 소문자 확장자 목록
 * @returns {Promise<Array<{ext:string, registered:boolean, ours:boolean, other:string|null, hard:boolean}>>}
 */
export async function fileAssocStatus(exts) {
  return await invoke("file_assoc_status", { exts });
}

/**
 * 파일 연결 대상 확장자(환경설정 순서), 정본 = Rust DEFAULT_ASSOC_EXTS, 프런트 사본은 SettingsWindow.svelte 하나
 * @returns {Promise<string[]>} 점 없는 소문자 확장자 목록
 */
export async function defaultAssocExts() {
  return await invoke("default_assoc_exts");
}

/**
 * 확장자 1개 [기본 앱 선택] 창, 최대 5초, false = 실패 → openDefaultApps() 폴백
 * @param {string} ext 점 없는 소문자 확장자
 * @returns {Promise<boolean>} 창을 띄웠는지
 */
export async function openDefaultAppPicker(ext) {
  return await invoke("open_default_app_picker", { ext });
}

/**
 * 숨긴 속성 창, 임시 파일 정리, 설정 창 재활성화 시 호출
 * @returns {Promise<void>}
 */
export async function finishDefaultAppPicker() {
  return await invoke("finish_default_app_picker");
}

/**
 * Windows 기본 앱 설정(ms-settings:defaultapps) 열기, 레지스트리로 못 빼앗는 확장자용
 * @returns {Promise<void>}
 */
export async function openDefaultApps() {
  return await invoke("open_default_apps");
}

/**
 * 아카이브를 새 독립 창에서 열기(중첩), 현재 창 유지
 * @param {string} path 열 아카이브 경로
 * @returns {Promise<void>}
 */
export async function openArchiveWindow(path) {
  return await invoke("open_archive_window", { path });
}

/**
 * 뷰어 창 mount 시 자기 아카이브 경로 회수(1회), 메인 창에서는 항상 null
 * @returns {Promise<string|null>}
 */
export async function takeViewerArchive() {
  return await invoke("take_viewer_archive");
}

/**
 * 탐색기에서 파일 선택 상태로 열기
 * @param {string} path 선택해서 보여줄 파일 경로
 * @returns {Promise<void>}
 */
export async function revealFile(path) {
  return await invoke("reveal_file", { path });
}

/**
 * 항목을 임시 폴더에 풀고 실행, 아카이브면 실행 대신 경로 반환, 아니면 기본 연결로 실행 후 null
 * @param {string} archive 열린 아카이브 경로
 * @param {string} innerPath 실행할 내부 파일 경로("/" 정규화)
 * @returns {Promise<string|null>} 아카이브면 풀린 임시 파일 경로, 아니면 null
 */
export async function openEntry(archive, innerPath) {
  return await invoke("open_entry", { archive, innerPath });
}

/**
 * 시스템 네이티브 컨텍스트 메뉴를 커서 위치에
 * @param {Array<{label?: string, onSelect?: () => void, enabled?: boolean, separator?: boolean}>} items
 * @param {number} x 창 기준 논리 x 좌표(생략 시 커서 위치)
 * @param {number} y 창 기준 논리 y 좌표
 * @returns {Promise<void>}
 */
export async function showContextMenu(items, x, y) {
  const built = [];
  for (const it of items) {
    if (it.separator) {
      built.push(await PredefinedMenuItem.new({ item: "Separator" }));
    } else {
      built.push(
        await MenuItem.new({
          text: it.label,
          enabled: it.enabled !== false,
          action: () => {
            if (it.onSelect) it.onSelect();
          },
        }),
      );
    }
  }
  const menu = await Menu.new({ items: built });
  const at = x != null && y != null ? new LogicalPosition(x, y) : undefined;
  await menu.popup(at);
}

/**
 * 네이티브 확인 대화상자(예/아니오)
 * @param {string} message 표시할 메시지
 * @param {{title?: string, kind?: "info"|"warning"|"error"}} opts
 * @returns {Promise<boolean>} 확인=true, 취소=false
 */
export async function confirmAction(message, opts) {
  return await confirmDialog(message, opts);
}

/**
 * 현재 창 label, main.js 가 창을 분기하는 데 사용, Tauri 준비 전이면 main 으로 간주
 * @returns {string} 창 label (예: "main" | "compress")
 */
export function currentWindowLabel() {
  try {
    return getCurrentWindow().label;
  } catch {
    return "main";
  }
}

/**
 * 현재 창 포커스 변화 구독, 상단 강조선 색 전환용
 * @param {(focused: boolean) => void} handler
 * @returns {Promise<() => void>} 구독 해제 함수
 */
export async function onWindowFocus(handler) {
  return await getCurrentWindow().onFocusChanged(({ payload }) => handler(payload));
}
