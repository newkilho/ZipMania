<script>
  // 압축 풀기 창(label extract), 대상 폴더, 범위, 하위폴더, 성공 후 삭제 선택 + 진행률/완료 표시
  // 백엔드 호출은 api.js 만 경유
  import { onMount, onDestroy, tick } from "svelte";
  import { get } from "svelte/store";
  import { t, errText } from "../lib/i18n.js";
  import FolderPicker from "./FolderPicker.svelte";
  import {
    takeExtractContext,
    checkConflicts,
    extract as extractApi,
    cancelJob,
    deleteFile,
    openFolder,
    createDirectory,
    onJobProgress,
    onJobDone,
    onJobError,
    onExtractContext,
    closeCurrentWindow,
    centerCurrentWindow,
    setCurrentWindowTitle,
    getSettings,
    saveSettings,
    emitSettingsChanged,
  } from "../lib/api.js";

  // ── 컨텍스트/입력 상태 ────────────────────────────────────
  let archive = ""; // 풀 대상 아카이브 경로
  let selectedInner = []; // 메인에서 선택돼 있던 내부 경로들
  let dest = ""; // 대상(상위) 폴더
  let ready = false; // 컨텍스트 로드 완료(폴더 브라우저를 올바른 초기 경로로 마운트하기 위함)
  /** 작업 중 도착한 새 해제 요청의 보류 상태 — 작업 종료 시 회수 */
  let pendingContext = false;
  /**
   * 컨텍스트 세대, await 앞에서 적고 돌아온 뒤 다르면 아무것도 하지 않는다
   * archive, finalDest, conflicts 를 쓰는 비동기 흐름 전부 적용
   */
  let generation = 0;
  /** 최초 loadContext() 가 끝났나, 그 전에 restart 를 돌리면 둘이 같은 상태를 덮는다, */
  let initialized = false;
  /** restart 진행 여부, 중복 시 오래된 쪽이 새 쪽을 덮어씀 */
  let restarting = false;
  /** 완료 처리(자동 닫기, 폴더 열기) 진행 여부 — 그 사이 새 작업 시작 금지 */
  let finishing = false;
  let initialDest = ""; // 폴더 브라우저가 마운트 시 드러낼 초기 경로
  let autoMode = false; // 빠른 해제 = 폼 생략 후 해제 과정부터 표시
  let folderPicker; // FolderPicker 인스턴스(새 폴더 생성 후 revealTo 호출용)

  // 새 폴더 인라인 입력 상태('압축 풀 파일' 라인 우측)
  let newFolderMode = false;
  let newFolderName = "";
  let creatingFolder = false;
  let newFolderError = "";

  let scope = "all"; // 'all' | 'selected'
  let createSubfolder = true; // 대상 폴더 하위에 '압축파일명' 폴더 생성
  let deleteAfter = false; // 성공 후 압축 파일 삭제

  // "각각 풀기" 배치: 각 아카이브를 자기 대상 폴더로 순차 해제
  let batchMode = false;
  let batchItems = []; // [{ archive, dest }]
  let batchIndex = 0;
  let batchLog = []; // 배치 진행 로그
  // 배치 중 경고/오류/건너뜀 여부, 마지막 항목 상태로 전체를 덮지 않음 — ok = 원본 삭제 허용
  let batchIssue = false;

  // ── 작업 단계: "form" → "running" → "done" ────────────────
  let phase = "form";
  let jobId = null;
  let jobPercent = 0;
  let jobFile = "";
  let jobResult = null; // { status, message }
  let starting = false;
  let startError = "";

  // 진행/완료 로그 및 시간
  let logLines = []; // 진행 상황 로그(시작/현재 파일/완료/소요시간)
  let startedAt = 0; // 작업 시작 시각(ms)
  let elapsedSec = 0; // 소요 시간(초)
  // 완료 후 동작, settings.toml 에 즉시 저장되고 이번 작업에는 완료 시점 값이 쓰인다
  let autoClose = false; // 성공 시 창닫기
  let openFolderAfter = false; // 성공 시 대상 폴더 열기
  let loadedSettings = null; // 저장 시 다른 항목을 보존하려고 원본을 들고 있는다

  // 충돌 확인 — 겹치는 파일을 하나씩 질의
  let checking = false;
  let conflicts = []; // 대상 폴더에 이미 있는 내부 경로 목록
  // 충돌 검사 성공 여부, 실패 시 빈 conflicts 는 충돌 없음이 아니라 모른다 — 구분 안 하면 조용한 덮어쓰기
  let conflictsChecked = false;
  let showConflict = false;
  let conflictIndex = 0; // 지금 묻고 있는 파일
  let conflictChoice = "overwrite"; // "overwrite" | "skip" | "rename"
  let conflictApplyAll = false; // 남은 파일에도 같은 선택 적용
  let decisions = {}; // 내부 경로 → 정책, 백엔드가 파일별로 적용

  // 암호 재시도
  let needPassword = false; // 암호 입력 표시
  let password = "";
  let passwordError = "";

  let centeredOnce = false; // 창 중앙 정렬을 1회만 하기 위한 플래그
  let unlistenJobs = [];

  // jobId 배정 전 도착한 이벤트 버퍼, 즉시 실패 시 job:error 가 jobId 대입보다 선행
  let pendingEvents = [];

  // 완료 로그 박스의 상태색 클래스(성공=연초록 / 경고=연노랑 / 오류=연빨강)
  $: resultCls =
    jobResult == null
      ? ""
      : jobResult.status === "ok"
        ? "ok"
        : jobResult.status === "warning"
          ? "warn"
          : jobResult.status === "canceled"
            ? "cancel"
            : "err";

  /**
   * 지금 새 요청 수용 가능 여부, 충돌 검사/암호 창/충돌 창까지 전부 진행 중
   */
  $: busy =
    checking ||
    starting ||
    restarting ||
    finishing ||
    phase === "running" ||
    showConflict ||
    needPassword;

  /**
   * 미뤄 둔 요청을 한가해지는 순간 회수, 최초 로드 전에는 미동작
   */
  $: if (initialized && !busy && pendingContext) {
    pendingContext = false;
    restartFromContext();
  }

  $: selectedCount = selectedInner.length;
  // 선택 항목 없으면 선택된 파일 사용 불가 → 전체로 복원
  $: if (selectedCount === 0 && scope === "selected") scope = "all";

  onMount(async () => {
    // 창 제목을 현재 언어로 설정(Rust 초기 제목 덮어쓰기)
    setCurrentWindowTitle(get(t)("extract.windowTitle")).catch(() => {});

    // 리스너를 회수보다 먼저 단다(loadContext 는 IPC 여러 왕복), 표시만 남기고 처리는 언제나 아래 $:
    // 컴포넌트의 ready 와 다른 값(그쪽은 폴더 브라우저 준비)
    const offC = await onExtractContext(() => {
      pendingContext = true;
    });

    // 이 창의 풀기 작업 진행률/완료 표시, 초기 회수, 최초 시작보다 먼저 달 것
    const offP = await onJobProgress((p) => dispatch("progress", p));
    const offD = await onJobDone((d) => dispatch("done", d));
    const offE = await onJobError((e) => dispatch("error", e));
    unlistenJobs = [offP, offD, offE, offC];

    try {
      await loadContext();

      // 리스너 등록 후 즉시 시작
      if (batchMode) {
        await tick();
        await runBatchItem();
      } else if (autoMode) {
        // 빠른 해제: 폼은 건너뛰지만 충돌 확인은 건너뛰지 않는다, tick() 으로 finalDest 재계산 대기
        await tick();
        await confirmThenExtract();
      }
    } finally {
      // 최초 시작 판단까지 끝난 뒤 외부 요청을 받는다, 실패해도 반드시 세울 것
      initialized = true;
    }
  });

  /**
   * 작업 이벤트 처리, jobId 배정 전 도착분은 버퍼 → drainPendingEvents 가 재투입
   */
  function dispatch(kind, ev) {
    if (jobId === null) {
      // 아직 어떤 작업이 내 것인지 모른다 — 판단 가능해질 때까지 보관
      if (pendingEvents.length < 64) pendingEvents.push([kind, ev]);
      return;
    }
    if (ev.jobId !== jobId) return;
    if (kind === "progress") {
      jobPercent = ev.percent;
      jobFile = ev.currentFile;
    } else if (kind === "done") {
      onJobFinished(ev.status, ev.message);
    } else {
      onJobErrored(ev);
    }
  }

  /** jobId 확정 직후, 그 사이 도착한 이벤트를 순서대로 처리 */
  function drainPendingEvents() {
    const buffered = pendingEvents;
    pendingEvents = [];
    for (const [kind, ev] of buffered) dispatch(kind, ev);
  }

  /** 작업 실패 처리, */
  function onJobErrored(e) {
    // 배치 = 이 아카이브 실패를 로그로 남기고 다음으로
    if (batchMode && e.code !== "password_required" && e.code !== "wrong_password") {
      batchLog = [...batchLog, "  ⚠ " + errText(get(t), e.code, e.message)];
      batchIssue = true;
      if (batchIndex < batchItems.length - 1) {
        batchIndex++;
        runBatchItem();
      } else {
        phase = "done";
        jobResult = { status: "warning", message: "" };
        logLines = [...batchLog, get(t)("extract.doneEnd")];
      }
      return;
    }
    // 암호 필요/오류 시 진행 화면 유지 + 암호 창 재시도(폼 미복귀)
    if (e.code === "password_required" || e.code === "wrong_password") {
      jobId = null;
      starting = false;
      phase = "running";
      needPassword = true;
      password = "";
      passwordError = e.code === "wrong_password" ? get(t)("extract.wrongPassword") : "";
      return;
    }
    phase = "done";
    jobResult = { status: "error", message: errText(get(t), e.code, e.message) };
  }

  /**
   * 보관된 해제 컨텍스트를 1회 회수해 화면 상태 결정, 첫 mount 와 restartFromContext 공용
   * @param preTaken 이미 회수한 컨텍스트(회수는 전역 1회)
   */
  async function loadContext(preTaken = null) {
    let defaults = null;
    try {
      defaults = await getSettings();
      loadedSettings = defaults;
    } catch {
      /* 설정 실패는 치명적이지 않다(하드코딩 기본값 유지), */
    }
    // 완료 후 동작은 모든 경로(폼, 빠른 해제, 배치)에 적용
    if (defaults) {
      autoClose = defaults.extract_auto_close ?? false;
      openFolderAfter = defaults.extract_open_folder ?? false;
    }

    let auto = false;
    try {
      const ctx = preTaken ?? (await takeExtractContext());
      if (ctx) {
        archive = ctx.archive || "";
        selectedInner = Array.isArray(ctx.selected) ? ctx.selected : [];
        const batch = Array.isArray(ctx.batch) ? ctx.batch : [];
        if (batch.length > 0) {
          // "각각 풀기": 아카이브마다 순차 배치 해제
          batchMode = true;
          batchItems = batch;
          auto = true;
        } else if (ctx.autoStart && ctx.dest) {
          // 빠른 해제(우클릭 '여기에 풀기' 등): 폼을 건너뛰고 즉시 해제
          auto = true;
          dest = ctx.dest; // 이미 최종 대상 폴더(하위폴더 반영됨)
          createSubfolder = false; // dest 를 그대로 사용
          scope = selectedInner.length ? "selected" : "all";
        }
      }
    } catch {
      // 회수 실패는 치명적이지 않다
    }

    if (auto) {
      autoMode = true;
      phase = "running";
    } else {
      if (defaults) {
        createSubfolder = defaults.extract_create_subfolder ?? true;
        deleteAfter = defaults.extract_delete_after ?? false;
      }
      dest = parentDir(archive);
      initialDest = dest;
      if (selectedCount > 0) scope = "selected";
      ready = true;
    }
  }

  /** 창이 열린 상태의 새 해제 요청 — 상태 초기화 후 시작(진행 중이면 무시) */
  async function restartFromContext() {
    // 작업 중만 미루면 부족, 충돌 검사/시작/암호 창/충돌 창에서 갈아엎으면 진행 중 흐름이 새 대상으로 이어짐
    // 어느 쪽이든 종료 후 회수 — 보관 컨텍스트는 Rust 쪽에 잔존
    if (busy || !initialized) {
      pendingContext = true;
      return;
    }
    restarting = true;
    try {
      await doRestartFromContext();
    } finally {
      restarting = false;
    }
  }

  /** restartFromContext 의 알맹이, 겹쳐 돌지 않는 것은 바깥에서 보장, */
  async function doRestartFromContext() {

    // 회수 대상 유무를 먼저 확인, 빈 값으로 초기화하면 batchMode, autoMode 해제로 일반 폼 전환
    const ctx = await takeExtractContext().catch(() => null);
    if (!ctx) return;

    // 지금부터가 새 시대다 — 앞 시대의 await 결과는 이제 반영되지 않는다
    generation += 1;

    // 이전 작업의 잔재를 지운다
    jobId = null;
    pendingEvents = [];
    jobPercent = 0;
    jobFile = "";
    jobResult = null;
    logLines = [];
    startError = "";
    needPassword = false;
    passwordError = "";
    password = "";
    showConflict = false;
    conflicts = [];
    decisions = {};
    batchMode = false;
    batchItems = [];
    batchIndex = 0;
    batchLog = [];
    batchIssue = false;
    autoMode = false;
    starting = false;
    phase = "form";
    ready = false;

    await loadContext(ctx);
    await tick();
    if (batchMode) {
      await runBatchItem();
    } else if (autoMode) {
      await confirmThenExtract();
    }
  }

  /** 배치의 현재 항목 해제(각 아카이브를 자기 대상 폴더로) */
  async function runBatchItem() {
    const item = batchItems[batchIndex];
    // 앞 항목의 job 은 끝났다, 남겨 두면 busy 판정과 이벤트 걸러내기가 앞 항목을 가리킨다
    jobId = null;
    starting = false;
    archive = item.archive;
    dest = item.dest;
    createSubfolder = false; // dest 를 그대로 최종 대상으로 사용
    scope = "all";
    selectedInner = [];
    password = "";
    conflictsChecked = false; // 앞 항목의 검사 결과를 물려받지 않는다
    decisions = {};
    batchLog = [...batchLog, `(${batchIndex + 1}/${batchItems.length}) ${fileNameOf(item.archive)}`];
    logLines = batchLog;
    await tick();
    // 항목마다 충돌 확인, 겹치면 질의, 취소 시 다음 항목으로
    await confirmThenExtract();
  }

  /** 경로에서 파일명만, */
  function fileNameOf(p) {
    const s = String(p ?? "");
    const i = Math.max(s.lastIndexOf("\\"), s.lastIndexOf("/"));
    return i >= 0 ? s.slice(i + 1) : s;
  }

  onDestroy(() => {
    for (const off of unlistenJobs) off && off();
  });

  // ── 경로 헬퍼 ─────────────────────────────────────────────

  /** 경로를 현재 OS 형식으로 통일(Windows 형식이면 역슬래시) */
  function toOsPath(p) {
    const s = String(p ?? "");
    const winLike = /^[A-Za-z]:/.test(s) || s.includes("\\");
    return winLike ? s.replace(/\//g, "\\") : s.replace(/\\/g, "/");
  }

  /** 경로의 상위 폴더, */
  function parentDir(p) {
    const s = String(p ?? "");
    const idx = Math.max(s.lastIndexOf("\\"), s.lastIndexOf("/"));
    return idx >= 0 ? s.slice(0, idx) : "";
  }

  /** 아카이브 파일명(확장자 제거), 예: backup.tar.gz → backup.tar (확장자 하나만 제거) */
  function baseNoExt(p) {
    const s = String(p ?? "");
    const idx = Math.max(s.lastIndexOf("\\"), s.lastIndexOf("/"));
    let name = idx >= 0 ? s.slice(idx + 1) : s;
    const dot = name.lastIndexOf(".");
    if (dot > 0) name = name.slice(0, dot);
    return name;
  }

  /** 경로 두 조각을 OS 구분자로 잇는다, */
  function joinPath(dir, name) {
    if (!dir) return name;
    const sep = /^[A-Za-z]:/.test(dir) || dir.includes("\\") ? "\\" : "/";
    const trimmed = dir.replace(/[\\/]+$/, "");
    return toOsPath(trimmed + sep + name);
  }

  /** 실제 풀 대상 경로(하위폴더 옵션 반영), */
  $: finalDest = createSubfolder && dest ? joinPath(dest, baseNoExt(archive)) : toOsPath(dest);

  // ── 액션 ──────────────────────────────────────────────────

  function currentSelected() {
    return scope === "selected" ? selectedInner : [];
  }

  // ── 새 폴더('압축 풀 파일' 라인 우측) ─────────────────────
  function startNewFolder() {
    if (!dest) return;
    newFolderError = "";
    newFolderName = "";
    newFolderMode = true;
    tick().then(() => document.getElementById("new-folder-input")?.focus());
  }

  function cancelNewFolder() {
    newFolderMode = false;
    newFolderName = "";
  }

  async function confirmNewFolder() {
    const name = newFolderName.trim();
    if (!name || creatingFolder) return;
    creatingFolder = true;
    newFolderError = "";
    try {
      const created = await createDirectory(dest, name);
      newFolderMode = false;
      newFolderName = "";
      // 만든 폴더를 선택 + 트리에 드러낸다
      if (folderPicker) folderPicker.revealTo(created);
      else dest = created;
    } catch (err) {
      newFolderError = (err && err.message) || String(err);
    } finally {
      creatingFolder = false;
    }
  }

  /**
   * 충돌 검사 후 겹치면 확인 창, 없으면 즉시 시작, 폼/빠른 해제/배치 모두 경유
   * startExtract("overwrite") 직접 호출 금지
   */
  async function confirmThenExtract() {
    if (starting) return;
    if (!finalDest) {
      // 대상 폴더 미확정 상태
      const msg = get(t)("extract.noDest");
      if (phase === "form") {
        startError = msg;
      } else {
        phase = "done";
        jobResult = { status: "error", message: msg };
        logLines = [...logLines, msg];
      }
      return;
    }

    // 이 결과가 어느 세대의 것인지 적어 둔다, 늦게 온 결과가 새 대상의 conflicts 를 덮으면 조용한 덮어쓰기
    const gen = generation;
    checking = true;
    let checkError = null;
    let checked = [];
    try {
      checked = await checkConflicts({
        archive,
        dest: finalDest,
        keepPaths: true,
        selected: currentSelected(),
        password: password || undefined,
      });
    } catch (e) {
      checkError = e;
    } finally {
      if (gen === generation) checking = false;
    }
    if (gen !== generation) return; // 지난 시대의 결과 — 지금 화면에 반영하지 않는다

    if (checkError) {
      conflicts = [];
      conflictsChecked = false;
    } else {
      conflicts = checked;
      conflictsChecked = true;
    }

    // 검사 실패의 덮어쓰기 진행 금지, 실패 시 conflicts = [] 의 뜻은 모름
    if (!conflictsChecked) {
      const code = checkError && checkError.code;
      // 암호가 없어 목록을 못 읽은 경우만 예외, 해제를 시작해 암호를 묻고 onSubmitPassword 가 다시 부른다
      if (code !== "password_required" && code !== "wrong_password") {
        const msg = errText(get(t), code, checkError && checkError.message);
        if (batchMode) {
          batchLog = [...batchLog, "  ⚠ " + msg];
          batchIssue = true;
          if (batchIndex < batchItems.length - 1) {
            batchIndex++;
            await runBatchItem();
            return;
          }
          phase = "done";
          jobResult = { status: "warning", message: "" };
          logLines = [...batchLog, get(t)("extract.doneEnd")];
          return;
        }
        phase = "done";
        jobResult = { status: "error", message: msg };
        logLines = [...logLines, msg];
        return;
      }
    }

    if (conflicts.length > 0) {
      // 첫 충돌 파일부터 순차 질의(배치 중이면 진행 화면 위에 표시)
      decisions = {};
      conflictIndex = 0;
      conflictChoice = "overwrite";
      conflictApplyAll = false;
      showConflict = true;
      return;
    }
    await startExtract("overwrite");
  }

  /** [확인] — 폼에서의 시작, */
  async function onConfirm() {
    await confirmThenExtract();
  }

  /**
   * 충돌 창 [확인], 선택 기록 후 다음 파일로, 모든 파일에 적용이면 남은 전부에 적용하고 시작
   */
  async function confirmConflictChoice() {
    const choice = conflictChoice;
    if (conflictApplyAll) {
      for (let i = conflictIndex; i < conflicts.length; i++) decisions[conflicts[i]] = choice;
      conflictIndex = conflicts.length;
    } else {
      decisions[conflicts[conflictIndex]] = choice;
      conflictIndex += 1;
    }

    if (conflictIndex < conflicts.length) {
      // 다음 파일 질의, 선택은 기본값으로 복원
      conflictChoice = "overwrite";
      return;
    }

    // 모든 충돌 결정 완료 → 시작, 기본 정책 overwrite 보다 decisions 우선
    showConflict = false;
    await startExtract("overwrite");
  }

  /** 충돌 확인 창의 [취소] — 배치면 이 항목만 건너뛰기, 단일 작업이면 폼 복귀 */
  function cancelConflict() {
    showConflict = false;
    if (!batchMode) {
      if (autoMode) {
        // 빠른 해제는 폼 부재 — 사용자 취소이므로 아무 동작 없이 마감
        phase = "done";
        jobResult = { status: "canceled", message: "" };
        logLines = [...logLines, get(t)("extract.doneCanceled")];
      }
      return;
    }
    batchLog = [...batchLog, "  ⚠ " + get(t)("extract.conflictSkipped", { name: fileNameOf(archive) })];
    batchIssue = true;
    if (batchIndex < batchItems.length - 1) {
      batchIndex++;
      runBatchItem();
    } else {
      phase = "done";
      jobResult = { status: "warning", message: "" };
      logLines = [...batchLog, get(t)("extract.doneEnd")];
    }
  }

  /** extract 호출 → 진행 화면으로 전환, */
  async function startExtract(overwrite) {
    starting = true;
    startError = "";
    try {
      const id = await extractApi({
        archive,
        dest: finalDest,
        selected: currentSelected(),
        keepPaths: true,
        overwrite,
        // 충돌 창의 파일별 선택(존재 시), 기본 정책보다 우선
        decisions,
        password: password || undefined,
      });
      jobId = id;
      // 시작 중 종료 지점, 유지 시 배치가 첫 항목에서 정지 + 미뤄 둔 요청 미처리
      starting = false;
      jobPercent = 0;
      jobFile = "";
      jobResult = null;
      needPassword = false;
      if (!batchMode) logLines = [get(t)("extract.logStart")];
      startedAt = Date.now();
      elapsedSec = 0;
      phase = "running";
      // 시작 시 창을 화면 중앙으로, 첫 시작에만 — 재시도마다 옮기면 화면이 튄다
      if (!centeredOnce) {
        centeredOnce = true;
        centerCurrentWindow().catch(() => {});
      }
      // jobId 확정 시점, 그 사이 먼저 도착한 이벤트 처리
      drainPendingEvents();
    } catch (err) {
      const msg = errText(get(t), err && err.code, get(t)("extract.startFailed"));
      starting = false;
      startError = msg;
      // 폼 없는 경로(빠른 해제, 배치)에는 startError 표시 부재 → 완료 화면에 오류로 마감
      if (phase !== "form") {
        phase = "done";
        jobResult = { status: "error", message: msg };
        logLines = [...logLines, msg];
      }
    }
  }

  /** 암호 입력 후 재시도 — 진행 화면 유지 + 같은 대상으로 재시작 */
  async function onSubmitPassword() {
    if (!password) return;
    needPassword = false;
    passwordError = "";
    // 암호 부재로 충돌 검사를 못 했으면 지금 재실행, 그냥 시작하면 묻기인데도 덮어쓰기
    if (!conflictsChecked) {
      await confirmThenExtract();
      return;
    }
    // 충돌 확인과 파일별 선택(decisions) 생존 → 해제만 재시작
    await startExtract("overwrite");
  }

  /** 암호 입력 취소 — 이 작업을 취소로 마감 */
  function onCancelPassword() {
    needPassword = false;
    passwordError = "";
    if (batchMode) {
      batchLog = [...batchLog, "  ⚠ " + get(t)("extract.passwordSkipped", { name: fileNameOf(archive) })];
      batchIssue = true;
      if (batchIndex < batchItems.length - 1) {
        batchIndex++;
        runBatchItem();
        return;
      }
      phase = "done";
      jobResult = { status: "warning", message: "" };
      logLines = [...batchLog, get(t)("extract.doneEnd")];
      return;
    }
    phase = "done";
    jobResult = { status: "canceled", message: "" };
    logLines = [...logLines, get(t)("extract.doneCanceled")];
  }

  /** 작업 완료 처리 — 로그 마감 + "압축 파일 삭제", "창닫기", "대상 폴더 열기" 옵션 반영, */
  async function onJobFinished(status, message) {
    // 마감이 끝날 때까지 busy 유지(await 여럿), 안 그러면 뒤늦은 자동 닫기가 새 작업의 창을 닫는다
    finishing = true;
    try {
      await finishJob(status, message);
    } finally {
      finishing = false;
    }
  }

  async function finishJob(status, message) {
    // 배치("각각 풀기"): 이 항목 마감 후 다음 항목으로 넘어가거나, 마지막이면 종료
    if (batchMode) {
      // 취소 = 배치 전체의 취소, 계속 풀면 멈추라고 한 일이 끝까지 진행
      if (status === "canceled") {
        batchLog = [...batchLog, "  ⨯ " + get(t)("extract.doneCanceled")];
        phase = "done";
        jobResult = { status: "canceled", message: "" };
        logLines = [...batchLog, get(t)("extract.doneCanceled")];
        return;
      }
      // 삭제는 ok 일 때만, warning = 빠진 항목 존재
      if (status === "ok" && deleteAfter) {
        try {
          await deleteFile(archive);
        } catch {
          /* 삭제 실패 무시 */
        }
      }
      if (status !== "ok") {
        batchIssue = true;
        if (message) batchLog = [...batchLog, "  ⚠ " + message];
      }
      if (batchIndex < batchItems.length - 1) {
        batchIndex++;
        await runBatchItem();
        return;
      }
      phase = "done";
      // 앞 항목의 경고, 오류는 마지막 항목이 성공해도 사라지지 않는다
      jobResult = { status: batchIssue ? "warning" : "ok", message: "" };
      logLines = [...batchLog, get(t)("extract.doneEnd")];
      return;
    }

    elapsedSec = startedAt ? (Date.now() - startedAt) / 1000 : 0;
    const ok = status === "ok" || status === "warning";

    // 성공 시 진행률 100 마감, 작은 아카이브는 진행률 콜백이 한 번도 오지 않는다, 취소, 오류는 그대로
    if (ok) jobPercent = 100;

    const tr = get(t);
    const lines = [...logLines];
    // 완료/취소 메시지 = 상태 기반 번역(백엔드 원문은 한국어 고정)
    lines.push(
      status === "canceled"
        ? tr("extract.doneCanceled")
        : status === "warning"
          ? tr("extract.doneWarn")
          : ok
            ? tr("extract.doneOk")
            : tr("extract.doneEnd"),
    );
    // 누락 내용은 백엔드 메시지에만 존재, 삼키면 사용자 확인 불가
    if (status === "warning" && message) lines.push(message);

    // "압축 파일 삭제" 옵션 — ok 일 때만(warning 은 일부 항목이 빠진 상태다)
    if (status === "ok" && deleteAfter) {
      try {
        await deleteFile(archive);
        lines.push(tr("extract.logDeleted"));
      } catch (err) {
        lines.push(tr("extract.logDeleteFailed") + ((err && err.message) || err));
      }
    }
    lines.push(tr("extract.logElapsed", { sec: elapsedSec.toFixed(1) }));
    logLines = lines;

    phase = "done";
    jobResult = { status, message };

    // 완료 후 옵션 처리
    if (ok && openFolderAfter) {
      try {
        await openFolder(finalDest);
      } catch {
        /* 열기 실패는 무시 */
      }
    }
    // 미뤄 둔 요청이 자동 닫기보다 먼저다, finishing 동안은 busy 로 둔다
    if (ok && autoClose && !pendingContext) {
      await onClose();
    }
  }

  /** 완료 후 동작 체크박스를 설정에 저장 — 소급 없음, 다음 해제부터 적용 */
  async function persistAfterOptions() {
    if (!loadedSettings) return;
    try {
      // 저장 직전 파일 재읽기 후 병합, 재읽기 실패 시 저장 안 함
      const next = {
        ...(await getSettings()),
        extract_auto_close: autoClose,
        extract_open_folder: openFolderAfter,
      };
      await saveSettings(next);
      // 저장 성공 시 방송 — 다른 창이 반영하는 유일한 경로
      await emitSettingsChanged(next);
      loadedSettings = next;
    } catch (err) {
      console.error("완료 후 동작 저장 실패:", err);
    }
  }

  /** [대상 폴더 열기] — 압축을 푼 폴더를 탐색기로 연다, */
  async function onOpenDest() {
    try {
      await openFolder(finalDest);
    } catch (err) {
      console.error("대상 폴더 열기 실패:", err);
    }
  }

  async function onCancelJob() {
    if (!jobId) return;
    try {
      await cancelJob(jobId);
    } catch (err) {
      console.error("작업 취소 실패:", err);
    }
  }

  async function onClose() {
    await closeCurrentWindow();
  }
</script>

<div class="win" data-ui="extract-window">
  <!-- 본문: 설정 폼 또는 진행 화면. 대화상자는 아래에서 위에 덮어 띄운다. -->
  {#if phase === "form"}
      <!-- 옵션 화면 -->
      <div class="pad body">
        <!-- 대상 폴더: 인라인 폴더 브라우저(주소줄 + 빠른위치 + 트리 + 새 폴더) -->
        {#if ready}
          <FolderPicker bind:this={folderPicker} bind:path={dest} initialPath={initialDest} />
        {/if}

        <!-- 범위 + (우측) 새 폴더 -->
        <fieldset class="row rad">
          <span class="lb">{$t("extract.scopeLabel")}</span>
          <label class="radio">
            <input type="radio" bind:group={scope} value="all" /> {$t("extract.scopeAll")}
          </label>
          <label class="radio" class:disabled={selectedCount === 0}>
            <input type="radio" bind:group={scope} value="selected" disabled={selectedCount === 0} />
            {$t("extract.scopeSelected")}{selectedCount > 0 ? " " + $t("common.countParen", { count: selectedCount }) : ""}
          </label>
          <div class="spacer"></div>
          {#if newFolderMode}
            <input
              id="new-folder-input"
              class="nf-input"
              type="text"
              bind:value={newFolderName}
              placeholder={$t("folderPicker.newFolderName")}
              on:keydown={(e) => (e.key === "Enter" ? confirmNewFolder() : e.key === "Escape" ? cancelNewFolder() : null)}
            />
            <button class="ghost small" on:click={confirmNewFolder} disabled={!newFolderName.trim() || creatingFolder}>{$t("common.confirm")}</button>
            <button class="ghost small" on:click={cancelNewFolder}>{$t("common.cancel")}</button>
          {:else}
            <button class="ghost small" on:click={startNewFolder} disabled={!dest}>{$t("folderPicker.newFolder")}</button>
          {/if}
        </fieldset>
        {#if newFolderError}
          <div class="nf-error" role="alert">⚠ {newFolderError}</div>
        {/if}

        <!-- 옵션 체크박스 -->
        <label class="check">
          <input type="checkbox" bind:checked={createSubfolder} />
          {$t("extract.createSubfolder")}
        </label>
        <label class="check">
          <input type="checkbox" bind:checked={deleteAfter} />
          {$t("extract.deleteAfter")}
        </label>

      </div>

      {#if startError}
        <div class="start-error" role="alert">⚠ {startError}</div>
      {/if}

      <!-- 하단 액션 -->
      <div class="actions">
        <div class="spacer"></div>
        <button class="ghost" on:click={onClose}>{$t("common.cancel")}</button>
        <button class="primary" on:click={onConfirm} disabled={!finalDest || checking || starting}>
          {checking ? $t("common.checking") : $t("common.confirm")}
        </button>
      </div>  {:else}
    <!-- 진행/완료 화면 (참고 이미지: 진행률 바 + 로그 영역 + 소요 시간) -->
    <div class="prog">
      <div class="prog-body">
        <div class="prog-label">{$t("extract.progressLabel", { pct: jobPercent })}</div>
        <div class="bar"><div class="fill" style="width: {jobPercent}%"></div></div>
        {#if phase === "running"}
          <div class="cur-file" title={jobFile}>{jobFile || $t("progress.preparing")}</div>
        {/if}
        {#if startError && phase !== "form"}
          <div class="start-error" role="alert">⚠ {startError}</div>
        {/if}
        <div class="log {resultCls}">
          {#each logLines as line}
            <div class="log-line">{line}</div>
          {/each}
        </div>
      </div>

      <div class="prog-actions">
        <div class="opts">
          <label class="check sm">
            <input type="checkbox" bind:checked={autoClose} on:change={persistAfterOptions} />
            {$t("extract.optClose")}
          </label>
          <label class="check sm">
            <input type="checkbox" bind:checked={openFolderAfter} on:change={persistAfterOptions} />
            {$t("extract.openDestFolder")}
          </label>
        </div>
        <div class="spacer"></div>
        {#if phase === "running"}
          <button class="ghost" on:click={onCancelJob}>{$t("common.cancel")}</button>
        {:else}
          <button class="ghost" on:click={onOpenDest}>{$t("extract.openDestFolder")}</button>
          <button class="primary" on:click={onClose}>{$t("common.close")}</button>
        {/if}
      </div>
    </div>
  {/if}

  <!--
    암호·충돌 대화상자는 본문을 갈아끼우지 않고 오버레이로 얹는다.
    갈아끼우면 DOM 이 통째로 교체되면서 화면이 한 번 깜빡인다.
  -->
  {#if needPassword}
    <div class="overlay">
    <!-- 암호 입력 — 진행 화면 위에 뜬다(폼으로 되돌아가지 않는다) -->
    <div class="pad conflict-pad">
      <h2>{$t("common.password")}</h2>
      <p class="conflict-file" title={archive}>{fileNameOf(archive)}</p>
      <p class="desc">{$t("extract.passwordAsk")}</p>

      <div class="pw-panel">
        <!-- svelte-ignore a11y-autofocus -->
        <input
          type="password"
          autofocus
          bind:value={password}
          placeholder={$t("extract.passwordPlaceholder")}
          on:keydown={(e) => e.key === "Enter" && onSubmitPassword()}
        />
        {#if passwordError}<p class="warn-text">{passwordError}</p>{/if}
      </div>

      <div class="actions">
        <div class="spacer"></div>
        <button class="primary" on:click={onSubmitPassword} disabled={!password}>
          {$t("common.confirm")}
        </button>
        <button class="ghost" on:click={onCancelPassword}>{$t("common.cancel")}</button>
      </div>
    </div>
    </div>
  {:else if showConflict}
    <div class="overlay">
    <!-- 충돌 확인 — 파일 하나씩 묻는다(진행 중에도 이 화면이 앞에 온다). -->
    <div class="pad conflict-pad">
      <h2>{$t("extract.conflictTitle")}</h2>
      <p class="conflict-file" title={conflicts[conflictIndex]}>
        {$t("extract.conflictExists", { name: conflicts[conflictIndex] ?? "" })}
      </p>
      <p class="desc">{$t("extract.conflictAsk")}</p>

      <div class="choices">
        <label class="check">
          <input type="radio" bind:group={conflictChoice} value="overwrite" />
          {$t("extract.overwrite")}
        </label>
        <label class="check">
          <input type="radio" bind:group={conflictChoice} value="skip" />
          {$t("extract.skip")}
        </label>
        <label class="check">
          <input type="radio" bind:group={conflictChoice} value="rename" />
          {$t("extract.rename")}
        </label>
      </div>

      <div class="actions">
        <div class="spacer"></div>
        <button class="primary" on:click={confirmConflictChoice}>{$t("common.confirm")}</button>
        <button class="ghost" on:click={cancelConflict}>{$t("common.cancel")}</button>
      </div>

      <!-- 구분선 아래: 남은 파일에도 같은 선택을 적용 -->
      <div class="conflict-foot">
        <label class="check sm">
          <input type="checkbox" bind:checked={conflictApplyAll} />
          {$t("extract.conflictApplyAll", { count: conflicts.length - conflictIndex })}
        </label>
      </div>
    </div>
    </div>
  {/if}
</div>

<style>
  .win {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg);
    color: var(--text);
    overflow: hidden;
  }
  .pad {
    padding: 16px 18px;
  }
  .body {
    flex: 1;
    min-height: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  h2 {
    margin: 0 0 12px;
    font-size: inherit;
    font-weight: 600;
  }
  .desc {
    margin: 0 0 12px;
    font-size: 13px;
    color: var(--text-muted);
  }
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    border: none;
    padding: 0;
    margin: 0;
  }
  .rad {
    flex-wrap: wrap;
  }
  .lb {
    flex: 0 0 76px;    color: var(--text-muted);
  }
  input[type="password"] {
    flex: 1 1 auto;
    min-width: 0;
    padding: 7px 9px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--btn-bg);
    color: var(--text);
    font-size: 13px;
  }
  .radio,
  .check {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
    color: var(--text);
    cursor: pointer;
  }
  .check {
    align-items: flex-start;
  }
  .radio.disabled {
    opacity: 0.5;
    cursor: default;
  }
  .pw-panel {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
    padding: 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--surface);
  }
  .warn-text {
    margin: 0;
    width: 100%;    color: var(--alert-text, #9b1c1c);
  }
  /* 대화상자 오버레이 — 본문(폼/진행) 위에 적층, 본문 교체가 없어 깜빡임 없음 */
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 20;
    background: var(--bg);
    display: flex;
    flex-direction: column;
  }
  .overlay > :global(.pad) {
    flex: 1;
    min-height: 0;
  }

  /* 충돌 확인 창 — 파일명 강조, 선택 라디오, 구분선 아래 "모든 파일에 적용" */
  .conflict-pad {
    display: flex;
    flex-direction: column;
    height: 100%;
  }
  .conflict-file {
    margin: 0 0 6px;
    font-size: 13px;
    font-weight: 600;
    word-break: break-all;
  }
  .choices {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin: 4px 0 16px;
  }
  .conflict-foot {
    margin-top: auto;
    padding-top: 10px;
    border-top: 1px solid var(--border);
  }
  .start-error {
    padding: 8px 18px;
    border-top: 1px solid var(--border);
    background: var(--alert-bg, #fde8e8);
    color: var(--alert-text, #9b1c1c);
    font-size: 13px;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 18px;
    border-top: 1px solid var(--border);
  }
  .spacer {
    flex: 1 1 auto;
  }
  button {
    padding: 7px 16px;
    border-radius: 6px;
    border: 1px solid var(--border);
    cursor: pointer;
    font-size: 13px;
  }
  button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  button.small {
    padding: 5px 10px;  }
  .nf-input {
    flex: 0 1 180px;
    min-width: 0;
    padding: 5px 8px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--btn-bg);
    color: var(--text);  }
  .nf-error {
    color: var(--alert-text, #9b1c1c);
    padding-left: 2px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ghost {
    background: var(--btn-bg);
    color: var(--text);
  }
  .primary {
    background: var(--accent);
    color: var(--accent-contrast);
    border-color: var(--accent);
  }
  /* 진행/완료 화면 (참고 이미지: 진행률 바 + 로그 영역) */
  .prog {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .prog-body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 14px 18px;
  }
  .prog-label {
    font-size: 13px;
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
  .bar {
    width: 100%;
    height: 16px;
    border-radius: 4px;
    background: var(--border);
    overflow: hidden;
  }
  .fill {
    height: 100%;
    background: var(--progress-success);
    transition: width 0.2s ease;
  }
  .cur-file {
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* 로그 박스 — 완료 시 상태색(성공=연초록/오류=연빨강)으로 배경을 물들인다, */
  .log {
    flex: 1;
    min-height: 80px;
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--btn-bg);
    padding: 8px 10px;    line-height: 1.7;
  }
  .log-line {
    white-space: pre-wrap;
    word-break: break-all;
  }
  .log.ok {
    background: var(--ok-bg, #e6f4ea);
    color: var(--ok-text, #1e6b34);
  }
  .log.warn {
    background: var(--warn-bg, #fdf3e0);
    color: var(--warn-text, #8a5a12);
  }
  .log.err {
    background: var(--alert-bg, #fde8e8);
    color: var(--alert-text, #9b1c1c);
  }
  .prog-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 18px;
    border-top: 1px solid var(--border);
  }
  .opts {
    display: flex;
    gap: 14px;
  }
  .check.sm {
    color: var(--text-muted);
  }
</style>
