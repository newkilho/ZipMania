// Svelte store — 앱 전역 상태 + 아카이브 열람, 백엔드 호출은 api.js 위임
// 컴포넌트는 @tauri-apps 직접 사용 금지

import { writable, derived, get } from "svelte/store";
import { t, errText } from "./i18n.js";
import {
  openArchive,
  startTest as startTestApi,
  startScan as startScanApi,
  startEdit as startEditApi,
  pickInputFiles,
  deleteFile,
  revealFile,
  confirmAction,
  extract as extractApi,
  cancelJob as cancelJobApi,
  openCompressWindow,
  openExtractWindow,
  openEntry,
  openArchiveWindow,
  readEntryPreview,
  onJobProgress,
  onJobDone,
  onJobError,
  onJobStarted,
  onTestReport,
  onScanReport,
} from "./api.js";

/**
 * 아카이브 인식 확장자(드롭 라우팅), 정본 READ_EXTS 의 사본, ext_tests 가 대조, (D3.8)
 */
const ARCHIVE_EXTS = new Set([
  "7z", "zip", "zipx", "jar", "rar", "r00", "arj", "lzh", "lha", "cab",
  "tar", "ova", "gz", "gzip", "tgz", "tpz", "bz2", "bzip2", "tbz", "tbz2",
  "xz", "txz", "zst", "tzst", "z", "taz", "lzma",
  "iso", "img", "udf", "wim", "swm", "esd", "dmg", "squashfs",
  "msi", "msp", "msm", "cpio", "rpm", "deb", "xar", "pkg", "chm", "nsis",
  "001", "cbz", "cbr", "cb7", "egg", "alz",
]);

/** 편집 가능 포맷(UpdateItems 지원) = 7z/zip/tar */
const EDITABLE_EXTS = new Set(["7z", "zip", "tar"]);

/** 경로 → 확장자(소문자, 점 없음), 없으면 "" */
function extOf(path) {
  const norm = String(path).replace(/\\/g, "/");
  const base = norm.substring(norm.lastIndexOf("/") + 1);
  const dot = base.lastIndexOf(".");
  return dot > 0 ? base.slice(dot + 1).toLowerCase() : "";
}

// ─── 기본 상태 ─────────────────────────────────────────────

/** 7z 엔진 버전 문자열(상태줄) */
export const engineVersion = writable("");

/** 열린 아카이브 절대 경로, null = 빈 상태 */
export const archivePath = writable(null);

/** 전체 엔트리, path = / 정규화 */
export const entries = writable([]);

/** 현재 탐색 중인 내부 폴더 경로, "" = 루트 */
export const currentPath = writable("");

/** 평면 보기, true = 전체 파일을 경로 포함 한 목록으로 */
export const flatView = writable(false);

/** 정렬 기준: 'name' | 'size' | 'date' */
export const sortKey = writable("name");
/** 정렬 방향: 'asc' | 'desc' */
export const sortDir = writable("asc");

/** 선택된 행의 path 집합 */
export const selectedPaths = writable(new Set());

/**
 * 좌측 하단 미리보기, null = 표시 안 함, { name, dataUri?, loading?, error? }
 */
export const previewImage = writable(null);

/** 미리보기를 지원하는 이미지 확장자(소문자, 점 없음), */
const IMAGE_EXTS = new Set([
  "jpg", "jpeg", "jpe", "jfif", "png", "gif", "bmp",
  "webp", "ico", "svg", "tif", "tiff", "avif",
]);

/** 미리보기로 메모리에 올릴 최대 크기(그 이상은 로드하지 않는다), */
const MAX_PREVIEW_BYTES = 32 * 1024 * 1024;

/** 경로가 미리보기 가능한 이미지 확장자인지, */
export function isImagePath(path) {
  return IMAGE_EXTS.has(extOf(path));
}

/** 인라인 오류 알림, { code, message } | null */
export const uiError = writable(null);

/** 목록 조회 진행 여부 (로딩 표시) */
export const loading = writable(false);

/**
 * 암호 다이얼로그, { open, error, mode }, mode = open(아카이브 열기) | extract(해제 재시도)
 */
export const passwordState = writable({ open: false, error: null, mode: "open" });

/**
 * 무결성 테스트 결과 다이얼로그, { open, phase: running|done, percent, currentFile, entries, error }
 */
export const testState = writable({
  open: false,
  phase: "running",
  percent: 0,
  currentFile: "",
  entries: [],
  error: null,
});

/**
 * 바이러스 검사 결과 다이얼로그, { open, phase: running|done, percent, currentFile, entries, error }
 */
export const scanState = writable({
  open: false,
  phase: "running",
  percent: 0,
  currentFile: "",
  entries: [],
  error: null,
});

/** 진행 중 작업, null = 유휴, shape: { jobId, percent, currentFile, startedAt } */
export const activeJob = writable(null);

/**
 * 마지막 작업 결과(인라인 알림), null = 없음, { status: ok|warning|canceled|error, message }
 */
export const jobResult = writable(null);

/** 작업 실행 중 여부 (열기, 해제 버튼 비활성용), */
export const jobRunning = derived(activeJob, ($j) => $j != null);

// 암호 재시도용 경로(모듈 내부)
let pendingPasswordPath = null;
// 해제 암호 재시도용 옵션(모듈 내부)
let pendingExtractOptions = null;
// 테스트 암호 재시도용 아카이브 경로(모듈 내부)
let pendingTestArchive = null;

// ─── 파생 상태 ─────────────────────────────────────────────

/** 아카이브 파일명 (경로의 마지막 조각) */
export const archiveName = derived(archivePath, ($p) => {
  if (!$p) return "";
  const norm = $p.replace(/\\/g, "/");
  return norm.substring(norm.lastIndexOf("/") + 1);
});

/** 현재 아카이브가 편집 가능 포맷인가 */
export const canEditArchive = derived(archivePath, ($p) =>
  !!$p && EDITABLE_EXTS.has(extOf($p))
);

/**
 * 화면에 보여줄 행 목록, 평면 = 전체 파일(경로 전체가 이름), 폴더 = 직속 자식, 폴더 우선 + 정렬
 */
export const visibleRows = derived(
  [entries, currentPath, flatView, sortKey, sortDir],
  ([$entries, $currentPath, $flat, $key, $dir]) => {
    const rows = $flat ? flatRows($entries) : childrenOf($entries, $currentPath);
    return sortRows(rows, $key, $dir);
  }
);

/**
 * 우측 폴더 트리, 루트 노드 { name: 아카이브명, path: "", children } 하나 반환
 */
export const folderTree = derived([entries, archiveName], ([$entries, $name]) =>
  buildFolderTree($entries, $name || "아카이브")
);

/**
 * 아카이브에 폴더가 하나라도 있나(한눈에 버튼 활성 조건)
 */
export const hasFolders = derived(entries, ($entries) =>
  $entries.some((e) => e.isDir || e.path.includes("/"))
);

/**
 * 미리보기 대상, 이미지 1개만 선택됐을 때 그 행, 아니면 null
 */
const previewTarget = derived(
  [selectedPaths, entries, archivePath],
  ([$sel, $entries, $archive]) => {
    if (!$archive || $sel.size !== 1) return null;
    const path = [...$sel][0];
    const ent = $entries.find((e) => e.path === path);
    if (!ent || ent.isDir || !isImagePath(ent.path)) return null;
    const name = ent.path.substring(ent.path.lastIndexOf("/") + 1);
    const tooBig = ent.size != null && ent.size > MAX_PREVIEW_BYTES;
    return { path: ent.path, name, tooBig };
  }
);

// 선택된 단일 이미지 → 메모리 → previewImage, 중복 로드 방지, 늦은 응답은 token 으로 폐기
let previewToken = 0;
let lastPreviewPath = null;

previewTarget.subscribe(async (target) => {
  const path = target ? target.path : null;
  // 같은 대상이면(둘 다 null 포함) 아무 것도 하지 않는다(불필요한 재로드 방지)
  if (path === lastPreviewPath) return;
  lastPreviewPath = path;

  const token = ++previewToken;

  if (!target) {
    previewImage.set(null);
    return;
  }
  if (target.tooBig) {
    previewImage.set({ name: target.name, error: get(t)("errors.previewTooBig") });
    return;
  }

  previewImage.set({ name: target.name, loading: true });
  try {
    const dataUri = await readEntryPreview(get(archivePath), target.path);
    if (token !== previewToken) return; // 더 최신 선택 존재 시 이 응답 폐기
    previewImage.set({ name: target.name, dataUri });
  } catch (err) {
    if (token !== previewToken) return;
    previewImage.set({
      name: target.name,
      error: errText(get(t), err && err.code, get(t)("errors.previewFailed")),
    });
  }
});

// ─── 순수 헬퍼 (테스트, 재사용 가능) ────────────────────────

/** 평면 보기 행: 전체 엔트리 중 파일만, 이름은 전체 경로, */
export function flatRows(all) {
  return all
    .filter((e) => !e.isDir)
    .map((e) => ({
      name: e.path,
      path: e.path,
      isDir: false,
      size: e.size,
      packedSize: e.packedSize,
      modified: e.modified,
      crc: e.crc,
    }));
}

/**
 * 내부 경로의 직속 자식 행, 중간 폴더가 엔트리에 없어도 경로에서 유도
 */
export function childrenOf(all, currentPath) {
  const prefix = currentPath ? currentPath + "/" : "";
  const map = new Map(); // name -> row

  for (const e of all) {
    const p = e.path;
    if (prefix) {
      if (!p.startsWith(prefix)) continue;
    }
    const rel = prefix ? p.slice(prefix.length) : p;
    if (rel === "") continue;

    const slash = rel.indexOf("/");
    if (slash === -1) {
      // 직속 자식(명시된 엔트리), 파생 폴더 선행 시에도 실제 메타로 덮어쓰기
      map.set(rel, {
        name: rel,
        path: p,
        isDir: e.isDir,
        size: e.size,
        packedSize: e.packedSize,
        modified: e.modified,
        crc: e.crc,
      });
    } else {
      // 더 깊은 경로 → 첫 조각을 폴더로 유도 (명시 엔트리가 없을 때만)
      const folderName = rel.slice(0, slash);
      if (!map.has(folderName)) {
        map.set(folderName, {
          name: folderName,
          path: prefix + folderName,
          isDir: true,
          size: null,
          packedSize: null,
          modified: null,
          crc: null,
          derived: true,
        });
      }
    }
  }
  return [...map.values()];
}

/**
 * 엔트리 → 폴더 중첩 트리, 명시 폴더 + 경로 유도 폴더 포함
 * 반환 = 루트 노드 { name, path: "", children }
 */
export function buildFolderTree(all, rootName) {
  const folderSet = new Set();
  for (const e of all) {
    const parts = e.path.split("/").filter((p) => p !== "");
    // 파일이면 부모들까지, 폴더면 자기 경로까지가 폴더다
    const upto = e.isDir ? parts.length : parts.length - 1;
    let acc = "";
    for (let i = 0; i < upto; i++) {
      acc = acc ? acc + "/" + parts[i] : parts[i];
      folderSet.add(acc);
    }
  }

  const root = { name: rootName || "아카이브", path: "", children: [] };
  const nodeByPath = new Map([["", root]]);

  // 부모가 자식보다 먼저 오도록 경로 정렬 후 노드 연결
  for (const fp of [...folderSet].sort()) {
    const slash = fp.lastIndexOf("/");
    const parentPath = slash >= 0 ? fp.slice(0, slash) : "";
    const name = slash >= 0 ? fp.slice(slash + 1) : fp;
    const node = { name, path: fp, children: [] };
    nodeByPath.set(fp, node);
    (nodeByPath.get(parentPath) || root).children.push(node);
  }

  // 각 단계 자식을 한국어 이름 순으로 정렬
  const sortRec = (n) => {
    n.children.sort((a, b) => a.name.localeCompare(b.name, "ko"));
    n.children.forEach(sortRec);
  };
  sortRec(root);
  return root;
}

/** 폴더 우선 + 정렬 기준 적용, */
export function sortRows(rows, key, dir) {
  const sign = dir === "desc" ? -1 : 1;
  const cmp = (a, b) => {
    // 폴더는 항상 먼저
    if (a.isDir !== b.isDir) return a.isDir ? -1 : 1;
    let r = 0;
    if (key === "size") {
      r = (a.size ?? 0) - (b.size ?? 0);
    } else if (key === "date") {
      r = String(a.modified ?? "").localeCompare(String(b.modified ?? ""));
    } else {
      r = String(a.name).localeCompare(String(b.name), "ko");
    }
    // 값이 같으면 이름으로 안정 정렬
    if (r === 0 && key !== "name") {
      r = String(a.name).localeCompare(String(b.name), "ko");
      return r; // 이름 보조 정렬은 방향 영향 안 받게 그대로
    }
    return r * sign;
  };
  return [...rows].sort(cmp);
}

// ─── 액션 ──────────────────────────────────────────────────

/**
 * 아카이브 열기, 암호 필요, 오류를 상태로 반영
 * @param {string} path
 * @param {string} password
 * @returns {Promise<boolean>} 성공 여부
 */
export async function openArchiveByPath(path, password) {
  loading.set(true);
  uiError.set(null);
  try {
    const list = await openArchive(path, password);
    const normalized = list.map((e) => ({
      ...e,
      path: e.path.replace(/\\/g, "/"),
    }));
    entries.set(normalized);
    archivePath.set(path);
    currentPath.set("");
    flatView.set(false);
    selectedPaths.set(new Set());
    passwordState.set({ open: false, error: null, mode: "open" });
    pendingPasswordPath = null;
    return true;
  } catch (err) {
    const code = err && err.code;
    if (code === "password_required") {
      pendingPasswordPath = path;
      passwordState.set({ open: true, error: null, mode: "open" });
    } else if (code === "wrong_password") {
      passwordState.set({ open: true, error: get(t)("errors.wrong_password"), mode: "open" });
    } else {
      uiError.set({
        code: code || "unknown",
        message: errText(get(t), code, get(t)("errors.openFailed")),
      });
    }
    return false;
  } finally {
    loading.set(false);
  }
}

/**
 * 메인 창 드롭 라우팅, 아카이브 1개 = 열기, 그 외 = 압축 창(입력 채움), 작업 중이면 무시
 */
export async function openDroppedPaths(paths) {
  if (!Array.isArray(paths) || paths.length === 0) return;
  if (get(jobRunning)) return;

  if (paths.length === 1 && ARCHIVE_EXTS.has(extOf(paths[0]))) {
    await openArchiveByPath(paths[0]);
  } else {
    await openCompressWindow(paths);
  }
}

/**
 * 암호 다이얼로그 제출 → mode 에 따라 열기(open) 또는 해제(extract) 재시도
 */
export async function submitPassword(pw) {
  const mode = get(passwordState).mode;
  if (mode === "extract") {
    if (!pendingExtractOptions) return;
    passwordState.set({ open: false, error: null, mode: "open" });
    await startExtract({ ...pendingExtractOptions, password: pw });
  } else if (mode === "test") {
    if (!pendingTestArchive) return;
    passwordState.set({ open: false, error: null, mode: "open" });
    await runTestJob(pendingTestArchive, pw);
  } else if (mode === "scan") {
    if (!pendingScanArchive) return;
    passwordState.set({ open: false, error: null, mode: "open" });
    await runScanJob(pendingScanArchive, pw);
  } else {
    if (!pendingPasswordPath) return;
    await openArchiveByPath(pendingPasswordPath, pw);
  }
}

/** 암호 입력 취소, */
export function cancelPassword() {
  passwordState.set({ open: false, error: null, mode: "open" });
  pendingPasswordPath = null;
  pendingExtractOptions = null;
  pendingTestArchive = null;
  pendingScanArchive = null;
}

// ─── 해제 ─────────────────────────────────────────────

/**
 * 기본 해제 대상 = 아카이브 폴더\아카이브명(확장자 제거), 예: D:\docs\backup.7z → D:\docs\backup
 */
export const defaultExtractDest = derived(archivePath, ($p) => {
  if (!$p) return "";
  // 마지막 구분자(\ 또는 /) 위치
  const sepIdx = Math.max($p.lastIndexOf("\\"), $p.lastIndexOf("/"));
  const dir = sepIdx >= 0 ? $p.slice(0, sepIdx) : "";
  let name = sepIdx >= 0 ? $p.slice(sepIdx + 1) : $p;
  // 확장자 하나 제거(.tar.gz 는 .gz 만 — 단순 규칙)
  const dot = name.lastIndexOf(".");
  if (dot > 0) name = name.slice(0, dot);
  const sep = $p.includes("\\") ? "\\" : "/";
  return dir ? dir + sep + name : name;
});

/**
 * 압축 풀기 창(label extract) 열기, 현재 아카이브 + 선택 경로 전달, 진행률/완료는 그 창이 표시
 */
export async function openExtract() {
  const archive = get(archivePath);
  if (!archive || get(jobRunning)) return;
  const selected = Array.from(get(selectedPaths));
  await openExtractWindow(archive, selected);
}

/**
 * 해제 시작, 옵션은 확정된 값
 * @param {object} opts
 * @param {string} opts.dest 대상 폴더
 * @param {string[]} opts.selected 선택 내부 경로(빈 배열=전체)
 * @param {boolean} opts.keepPaths 경로 유지 여부
 * @param {string} opts.overwrite 확정 정책 "overwrite" | "skip"
 * @param {string} opts.password 암호(옵션)
 */
export async function startExtract(opts) {
  const archive = get(archivePath);
  if (!archive || get(jobRunning)) return;

  // 암호 재시도용 옵션 보관
  pendingExtractOptions = { ...opts };
  jobResult.set(null);

  try {
    const jobId = await extractApi({
      archive,
      dest: opts.dest,
      selected: opts.selected ?? [],
      keepPaths: opts.keepPaths,
      overwrite: opts.overwrite,
      password: opts.password,
    });
    // 작업 시작 상태로 전환(진행률 0, 경과시간 계산용 시작 시각)
    activeJob.set({ jobId, percent: 0, currentFile: "", startedAt: Date.now(), kind: "extract" });
  } catch (err) {
    // 즉시 실패(예: job_busy 또는 실행 실패)
    const message = errText(get(t), err && err.code, get(t)("errors.extractStartFailed"));
    jobResult.set({ status: "error", message });
  }
}

// ─── 압축 ─────────────────────────────────────────────
//
// 메인 창은 창 여는 트리거만, 입력, 옵션, createArchive 는 CompressWindow, job:started 로 메인 진행률 표시

/** 진행 중인 작업 취소 */
export async function cancelActiveJob() {
  const job = get(activeJob);
  if (!job) return;
  try {
    await cancelJobApi(job.jobId);
  } catch (err) {
    console.error("작업 취소 실패:", err);
  }
}

/**
 * job:progress/done/error 구독 → store 반영, App onMount 에서 1회
 * @returns {Promise<() => void>} 구독 해제 함수
 */
// 압축/해제 job = 각자의 창이 표시 → 메인은 무시, job:started 에서 id 수집 후 필터링
const externalJobIds = new Set();

export async function initJobEvents() {
  const offStarted = await onJobStarted((s) => {
    if (
      s.kind === "compress" ||
      s.kind === "extract" ||
      s.kind === "test" ||
      s.kind === "scan"
    ) {
      // 별도 창(압축/풀기) 또는 테스트, 검사 다이얼로그가 자체 표시 → 메인 진행률 패널 무시
      externalJobIds.add(s.jobId);
      // job id 를 여기서 먼저 받는다, IPC 응답을 기다리면 그 사이 진행률, 리포트가 버려진다
      if (s.kind === "test" && testAwaiting) testJobId = s.jobId;
      if (s.kind === "scan" && scanAwaiting) scanJobId = s.jobId;
      return;
    }
    activeJob.update((j) => {
      if (j && j.jobId === s.jobId) return j;
      return { jobId: s.jobId, kind: s.kind, percent: 0, currentFile: "", startedAt: Date.now() };
    });
    jobResult.set(null);
  });

  const offProgress = await onJobProgress((p) => {
    // 테스트/검사 job 의 진행률은 각 다이얼로그로 라우팅
    if (p.jobId === testJobId) {
      testState.update((s) => ({ ...s, percent: p.percent, currentFile: p.currentFile }));
      return;
    }
    if (p.jobId === scanJobId) {
      scanState.update((s) => ({ ...s, percent: p.percent, currentFile: p.currentFile }));
      return;
    }
    if (externalJobIds.has(p.jobId)) return;
    activeJob.update((j) => {
      if (!j || j.jobId !== p.jobId) return j;
      return { ...j, percent: p.percent, currentFile: p.currentFile };
    });
  });

  const offDone = await onJobDone((d) => {
    if (externalJobIds.has(d.jobId)) {
      externalJobIds.delete(d.jobId);
      return;
    }
    activeJob.set(null);
    pendingExtractOptions = null;
    // 완료/취소 메시지는 상태 기반으로 번역(백엔드 원문은 한국어 고정)
    const tr = get(t);
    const isEdit = d.jobId === editJobId;
    if (isEdit) editJobId = null;
    // 편집 성공 시 목록 새로고침(취소는 원본 미변경이라 제외)
    if (isEdit && d.status !== "canceled") {
      reloadEntries();
    }
    // 편집 성공은 목록에 바로 보이므로 토스트 생략
    if (isEdit && d.status !== "canceled") return;
    const msg = d.status === "canceled" ? tr("progress.canceled") : tr("progress.done");
    jobResult.set({ status: d.status, message: msg });
  });

  const offError = await onJobError((e) => {
    // 테스트/검사 job 오류(암호 필요/틀림/손상)는 각 흐름으로 라우팅
    if (e.jobId === testJobId) {
      externalJobIds.delete(e.jobId);
      handleTestError(e);
      return;
    }
    if (e.jobId === scanJobId) {
      externalJobIds.delete(e.jobId);
      handleScanError(e);
      return;
    }
    if (externalJobIds.has(e.jobId)) {
      externalJobIds.delete(e.jobId);
      return;
    }
    activeJob.set(null);
    pendingExtractOptions = null;
    jobResult.set({ status: "error", message: errText(get(t), e.code, e.message) });
  });

  // 테스트 결과(파일별 CRC) → 테스트 다이얼로그를 done 단계로 채운다
  const offTestReport = await onTestReport((r) => {
    if (r.jobId !== testJobId) return;
    externalJobIds.delete(r.jobId);
    testJobId = null;
    testAwaiting = false;
    testState.update((s) => ({ ...s, phase: "done", entries: r.entries || [], error: null }));
  });

  // 검사 결과(파일별 AMSI) → 검사 다이얼로그를 done 단계로 채운다
  const offScanReport = await onScanReport((r) => {
    if (r.jobId !== scanJobId) return;
    externalJobIds.delete(r.jobId);
    scanJobId = null;
    scanAwaiting = false;
    scanState.update((s) => ({ ...s, phase: "done", entries: r.entries || [], error: null }));
  });

  return () => {
    offStarted();
    offProgress();
    offDone();
    offError();
    offTestReport();
    offScanReport();
  };
}

/** 작업 결과 알림 닫기, */
export function closeJobResult() {
  jobResult.set(null);
}

// ─── 무결성 테스트 ─────────────────────────────────────────
//
// 열린 아카이브를 백그라운드 job 으로 검증 → 진행률, CRC 를 testState 에, 암호 문제면 mode test 로 재시도

// 진행 중인 테스트 job id (이벤트 라우팅용, 모듈 내부)
let testJobId = null;
// 시작 요청했지만 job id 미확정, job:started 수신 대상 판별용
let testAwaiting = false;

/** 현재 아카이브의 무결성 테스트 시작, 진행 중이면 무시 */
export async function openTest() {
  const archive = get(archivePath);
  if (!archive || get(jobRunning) || testJobId) return;
  await runTestJob(archive, undefined);
}

/** 내부: 테스트 job 시작 + 다이얼로그(running) 오픈, 진행률, 결과는 이벤트로 채워진다, */
async function runTestJob(archive, password) {
  pendingTestArchive = archive;
  testJobId = null;
  testAwaiting = true;
  testState.set({ open: true, phase: "running", percent: 0, currentFile: "", entries: [], error: null });
  try {
    const id = await startTestApi(archive, password);
    // 응답이 돌아오는 사이에 job 이 이미 끝났으면(리포트 처리 완료) 되살리지 않는다
    if (testAwaiting) {
      testJobId = id;
      testAwaiting = false;
    }
  } catch (err) {
    // 즉시 실패(예: job_busy)
    testJobId = null;
    testAwaiting = false;
    testState.update((s) => ({
      ...s,
      phase: "done",
      error: errText(get(t), err && err.code, get(t)("test.failed")),
    }));
  }
}

/** 테스트 job:error 처리 — 암호 문제면 암호 다이얼로그, 그 외는 오류 표시, */
function handleTestError(e) {
  testJobId = null;
  testAwaiting = false;
  const code = e && e.code;
  if (code === "password_required" || code === "wrong_password") {
    passwordState.set({
      open: true,
      error: code === "wrong_password" ? get(t)("errors.wrong_password") : null,
      mode: "test",
    });
  } else {
    testState.update((s) => ({
      ...s,
      phase: "done",
      error: errText(get(t), code, e && e.message),
    }));
  }
}

/** 테스트 다이얼로그 닫기, 진행 중이면 job 취소 */
export function closeTest() {
  if (testJobId) {
    cancelJobApi(testJobId).catch(() => {});
    testJobId = null;
  }
  testAwaiting = false;
  testState.set({ open: false, phase: "running", percent: 0, currentFile: "", entries: [], error: null });
}

// ─── 바이러스 검사(AMSI) ──────────────────────────────────
//
// 내부 파일(10MB 미만)을 AMSI 로 검사 → 진행률, 결과를 scanState 에, 암호 문제면 mode scan 으로 재시도

// 진행 중인 검사 job id (이벤트 라우팅용, 모듈 내부)
let scanJobId = null;
// 시작을 요청했지만 아직 job id 를 모르는 상태(테스트와 같은 이유)
let scanAwaiting = false;
// 검사 암호 재시도 시 다시 검사할 아카이브 경로
let pendingScanArchive = null;

/** 현재 아카이브의 바이러스 검사 시작, 진행 중이면 무시 */
export async function openScan() {
  const archive = get(archivePath);
  if (!archive || get(jobRunning) || scanJobId) return;
  await runScanJob(archive, undefined);
}

/** 내부: 검사 job 시작 + 다이얼로그(running) 오픈, */
async function runScanJob(archive, password) {
  pendingScanArchive = archive;
  scanJobId = null;
  scanAwaiting = true;
  scanState.set({ open: true, phase: "running", percent: 0, currentFile: "", entries: [], error: null });
  try {
    const id = await startScanApi(archive, password);
    if (scanAwaiting) {
      scanJobId = id;
      scanAwaiting = false;
    }
  } catch (err) {
    scanJobId = null;
    scanAwaiting = false;
    scanState.update((s) => ({
      ...s,
      phase: "done",
      error: errText(get(t), err && err.code, get(t)("scan.failed")),
    }));
  }
}

/** 검사 job:error 처리 — 암호 문제면 암호 다이얼로그, 그 외는 오류 표시, */
function handleScanError(e) {
  scanJobId = null;
  scanAwaiting = false;
  const code = e && e.code;
  if (code === "password_required" || code === "wrong_password") {
    passwordState.set({
      open: true,
      error: code === "wrong_password" ? get(t)("errors.wrong_password") : null,
      mode: "scan",
    });
  } else {
    scanState.update((s) => ({
      ...s,
      phase: "done",
      error: errText(get(t), code, e && e.message),
    }));
  }
}

/** 검사 다이얼로그 닫기, 진행 중이면 job 취소 */
export function closeScan() {
  if (scanJobId) {
    cancelJobApi(scanJobId).catch(() => {});
    scanJobId = null;
  }
  scanAwaiting = false;
  scanState.set({ open: false, phase: "running", percent: 0, currentFile: "", entries: [], error: null });
}

// ─── 아카이브 편집(파일 추가/삭제) ─────────────────────────
//
// 아카이브에 파일 추가, 선택 항목 삭제, start_edit job, 진행률은 메인 패널, 완료 시 목록 새로고침
// 기존 항목 = 재압축 없이 복사, 7z/zip/tar 만

// 진행 중인 편집 job id (이벤트 라우팅용, 모듈 내부)
let editJobId = null;

/** 현재 아카이브가 편집 가능한 포맷인지, 아니면 안내를 띄우고 false, */
function ensureEditable(archive) {
  if (EDITABLE_EXTS.has(extOf(archive))) return true;
  jobResult.set({ status: "error", message: get(t)("edit.unsupported") });
  return false;
}

/** 파일 선택 후 현재 아카이브에 추가 */
export async function addFilesToArchive() {
  const archive = get(archivePath);
  if (!archive || get(jobRunning) || editJobId) return;
  if (!ensureEditable(archive)) return;
  const files = await pickInputFiles();
  if (!files.length) return;
  await runEditJob(archive, files, []);
}

/** 선택된 내부 항목을 아카이브에서 삭제 */
export async function removeSelectedFromArchive() {
  const archive = get(archivePath);
  if (!archive || get(jobRunning) || editJobId) return;
  if (!ensureEditable(archive)) return;
  const remove = Array.from(get(selectedPaths));
  if (!remove.length) return;
  await runEditJob(archive, [], remove);
}

/** 내부: 편집 job 시작, 진행률/완료는 initJobEvents 가 처리(메인 패널 + 새로고침), */
async function runEditJob(archive, add, remove) {
  jobResult.set(null);
  try {
    editJobId = await startEditApi(archive, add, remove, undefined);
  } catch (err) {
    editJobId = null;
    jobResult.set({
      status: "error",
      message: errText(get(t), err && err.code, get(t)("edit.failed")),
    });
  }
}

/** 목록 재로드(편집 후), 선택 초기화, 폴더, 평면 보기 유지, */
export async function reloadEntries() {
  const archive = get(archivePath);
  if (!archive) return;
  try {
    const list = await openArchive(archive);
    const normalized = list.map((e) => ({ ...e, path: e.path.replace(/\\/g, "/") }));
    entries.set(normalized);
    selectedPaths.set(new Set());
  } catch {
    // 재로드 실패(예: 암호) 시 기존 목록 유지
  }
}

// ─── 우클릭 컨텍스트 메뉴 액션 ─────────────────────────────

/** 아카이브 경로의 상위 폴더(OS 구분자 유지), */
function archiveDir(archive) {
  const idx = Math.max(archive.lastIndexOf("\\"), archive.lastIndexOf("/"));
  return idx >= 0 ? archive.slice(0, idx) : "";
}

/** 현재 아카이브 파일 삭제(확인 후), 성공 시 빈 상태로 복귀 */
export async function deleteCurrentArchive() {
  const archive = get(archivePath);
  if (!archive || get(jobRunning)) return;
  const name = archive.replace(/\\/g, "/").split("/").pop();
  const ok = await confirmAction(get(t)("ctx.deleteConfirm", { name }), {
    title: get(t)("ctx.deleteArchive"),
    kind: "warning",
  });
  if (!ok) return;
  try {
    await deleteFile(archive);
    // 열린 아카이브가 사라졌으므로 빈 상태로 초기화
    archivePath.set(null);
    entries.set([]);
    currentPath.set("");
    flatView.set(false);
    selectedPaths.set(new Set());
    jobResult.set({ status: "ok", message: get(t)("ctx.deleteDone") });
  } catch (err) {
    jobResult.set({
      status: "error",
      message: errText(get(t), err && err.code, get(t)("ctx.deleteFailed")),
    });
  }
}

/** 현재 아카이브가 있는 폴더를 탐색기에서 연다(파일 선택 상태), */
export async function revealCurrentArchive() {
  const archive = get(archivePath);
  if (!archive) return;
  try {
    await revealFile(archive);
  } catch {
    // 탐색기 열기 실패는 조용히 무시
  }
}

/**
 * 선택 항목만 기본 폴더로 해제, 폼을 건너뛰고 진행 화면부터
 */
export async function extractSelectedQuick() {
  const archive = get(archivePath);
  if (!archive || get(jobRunning)) return;
  const selected = Array.from(get(selectedPaths));
  if (!selected.length) return;
  await openExtractWindow(archive, selected, get(defaultExtractDest), true);
}

/**
 * 전체를 아카이브 폴더로 해제, 폼을 건너뛰고 진행 화면부터
 */
export async function extractAllToArchiveFolder() {
  const archive = get(archivePath);
  if (!archive || get(jobRunning)) return;
  await openExtractWindow(archive, [], archiveDir(archive), true);
}

/**
 * 파일 1개를 %TEMP% 에 풀어 열기, 아카이브면 새 창(판정은 백엔드 정본), 임시 폴더는 종료 시 삭제
 */
export async function openInnerEntry(path) {
  const archive = get(archivePath);
  if (!archive) return;
  try {
    const nested = await openEntry(archive, path);
    if (nested) await openArchiveWindow(nested);
  } catch (err) {
    const message =
      typeof err === "string" && err ? err : get(t)("errors.openEntryFailed");
    jobResult.set({ status: "error", message });
  }
}

/** 화면에 보이는 모든 행 선택(전체 선택) */
export function selectAllVisible() {
  const paths = get(visibleRows).map((r) => r.path);
  selectedPaths.set(new Set(paths));
}

/** 내부 폴더로 이동, */
export function navigateTo(path) {
  currentPath.set(path);
  selectedPaths.set(new Set());
}

/** 폴더 진입 (더블클릭), */
export function enterFolder(folderPath) {
  navigateTo(folderPath);
}

/** 좌측 폴더 트리의 폴더 선택 — 평면 보기 해제 + 해당 폴더로 이동 */
export function navigateFromTree(folderPath) {
  flatView.set(false);
  navigateTo(folderPath);
}

/** 평면 보기 토글, */
export function toggleFlat() {
  flatView.update((v) => !v);
  selectedPaths.set(new Set());
}

/** 정렬 헤더 클릭: 같은 키면 방향 토글, 다른 키면 오름차순으로 전환, */
export function setSort(key) {
  if (get(sortKey) === key) {
    sortDir.set(get(sortDir) === "asc" ? "desc" : "asc");
  } else {
    sortKey.set(key);
    sortDir.set("asc");
  }
}

/**
 * 행 단일 선택(토글 아님), 키보드 이동처럼 항상 하나만 선택할 때
 * @param {string|null} path null 이면 선택 해제
 */
export function setSelection(path) {
  selectedPaths.set(new Set(path == null ? [] : [path]));
}

/**
 * 여러 경로 선택(기존 대체), Shift+클릭 범위 선택용
 * @param {string[]} paths
 */
export function setSelectionPaths(paths) {
  selectedPaths.set(new Set(paths ?? []));
}

/** 행 선택, additive(Ctrl) 이면 토글 누적, 아니면 단일 선택, */
export function selectRow(path, additive) {
  selectedPaths.update((s) => {
    const next = new Set(additive ? s : []);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    return next;
  });
}

/** 인라인 오류 닫기, */
export function closeError() {
  uiError.set(null);
}
