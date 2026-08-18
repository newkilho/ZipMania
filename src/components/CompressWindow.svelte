<script>
  // 새 압축 창(label compress), 입력, 옵션을 자체 상태로 보유, 진행률/완료도 이 창에서 표시
  // 백엔드 호출은 api.js 만 경유
  import { onMount, onDestroy, tick } from "svelte";
  import { get } from "svelte/store";
  import { t, errText } from "../lib/i18n.js";
  import {
    pickInputFiles,
    pickInputFolders,
    pickSaveArchive,
    statPaths,
    listFolderFiles,
    createArchive,
    cancelJob,
    leaseCompressLaunch,
    dispatchCompressLaunch,
    ackCompressLaunch,
    peekCompressStandalone,
    planEachCompress,
    onCompressTakeInputs,
    onFileDrop,
    onJobProgress,
    onJobDone,
    onJobError,
    closeCurrentWindow,
    resizeCurrentWindow,
    setCurrentWindowTitle,
  } from "../lib/api.js";
  import { formatSize } from "../lib/format.js";
  import { FORM_DEFAULTS, isFormDirty, batchIssueAfter, runPlan } from "../lib/compressPlan.js";
  import { createCoordinator } from "../lib/compressCoordinator.js";

  // 폼/진행 화면 창 크기, 폼 값 = Rust open_compress_window 의 inner_size 와 동일 필요
  const FORM_SIZE = [680, 620];
  const JOB_SIZE = [460, 240];

  // 포맷별 확장자
  const FORMAT_EXT = { "7z": "7z", zip: "zip", tar: "tar" };
  // 압축 레벨 선택지, 라벨은 번역 키, 템플릿에서 $t 로 렌더
  const LEVELS = [
    { value: 0, labelKey: "compress.level0" },
    { value: 1, labelKey: "compress.level1" },
    { value: 5, labelKey: "compress.level5" },
    { value: 7, labelKey: "compress.level7" },
    { value: 9, labelKey: "compress.level9" },
  ];

  // ── 자체 상태 ─────────────────────────────────────────────
  let inputs = []; // 압축할 원본 경로 목록(중복 제거해 관리)
  let output = "";
  let format = "zip"; // 기본 압축 형식
  let level = 5;
  let password = "";
  let encryptNames = false;
  let outputSuggested = false; // 출력 경로 자동 제안을 1회만 하기 위한 플래그

  // "각각 압축" 배치: 각 원본을 자기 이름의 아카이브로 순차 압축
  let batchMode = false;
  let batchItems = []; // [{ input, output }]
  let batchIndex = 0;
  // 배치에서 앞 항목이 실패, 경고로 끝났다는 표시, 마지막 항목의 성공이 덮지 못하게 누적
  let batchIssue = false;
  let eachMode = false; // 폼 체크박스 = 켜면 [압축 시작] 시 각 항목을 개별 압축(배치)
  let starting = false; // 압축 시작 요청 진행 중(중복 클릭 방지)
  let startError = ""; // 시작 실패(예: 이미 다른 작업 진행 중) 인라인 메시지

  // 작업 단계: "form"(설정) → "running"(압축 중) → "done"(완료/실패), 압축 창이 자체 표시
  let phase = "form";
  let jobId = null;

  // jobId 배정 전 도착한 이벤트 버퍼, createArchive 반환 전 백엔드 실패 가능
  let pendingEvents = [];
  let jobPercent = 0;
  let jobFile = "";
  let jobResult = null; // { status: "ok"|"warning"|"canceled"|"error", message }

  // 결과 상태별 아이콘, 색 클래스
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
  $: resultIcon =
    jobResult == null
      ? ""
      : jobResult.status === "ok"
        ? "✓"
        : jobResult.status === "canceled"
          ? "⨯"
          : "⚠";

  // UI 로컬 상태
  let showPasswordPanel = false; // 암호 설정 인라인 패널 표시 여부
  let selected = new Set(); // 목록에서 선택된 입력 경로들
  let statMap = {}; // path -> { name, path, size, isDir }
  let statToken = 0; // 최신 statPaths 요청만 반영하기 위한 토큰
  let folderFiles = {}; // 폴더 path -> [{ rel, size }] (내부 파일 펼침용)
  let ffToken = 0; // 최신 폴더 목록 요청만 반영하기 위한 토큰

  let unlistenAdd = null;
  let unlistenDrop = null;
  // 진행 중 드롭 파일 — 한가해지면 목록에 추가(그 사이 추가 시 실행 인자 변동)
  let droppedWhileBusy = [];
  let unlistenJobs = [];

  // 초기 회수, 최초 시작까지 끝났나, 이 전에는 새 요청을 처리하지 않는다(둘이 같은 상태를 덮는다)
  let initialized = false;
  // 조율자의 회수/적용 여부, 회수 = IPC 왕복, 배치는 대기 중 starting=false, phase=form
  // 이 표시 없으면 그 사이 [압축 시작] 으로 회수 중이던 입력이 빠진 채 압축
  let taking = false;

  /** 작업이 돌고 있나(시작 중 포함), 조율자에게 주는 값 — 회수 중은 조율자 자신의 상태다, */
  $: jobBusy = starting || phase === "running";
  /** 지금 새 요청 수용 가능 여부 — 회수 중, 시작 중, 압축 중은 불가(동시 1작업) */
  $: busy = taking || jobBusy;
  /** 폼에 아직 시작하지 않은 사용자 입력 유무(독립 요청이 지우면 안 되는 대상) */
  $: formDirty = isFormDirty({ phase, inputs });

  /** 요청 처리 조율자, 전이(idle/taking/blocked/applying)는 전부 창 밖, */
  const coord = createCoordinator({
    api: {
      lease: leaseCompressLaunch,
      dispatch: dispatchCompressLaunch,
      ack: ackCompressLaunch,
      peekStandalone: peekCompressStandalone,
    },
    host: {
      // 호출 시점의 현재 값 반환 — 조율자는 await 를 건넌 뒤에도 이것으로 재판정
      getState: () => ({ phase, inputs, busy: jobBusy }),
      apply: (plan) => applyPlan(plan),
    },
    onState: (s) => {
      taking = s !== "idle";
    },
    onError: (e) => console.error("압축 요청 처리 실패:", e),
  });

  // 창 상태 변화마다 통지(시작/완료, 입력 추가/삭제), 미뤄 둔 요청은 한가해지는 순간 처리
  $: if (initialized) {
    // 값 참조로 의존성 확보(값 자체는 getState 가 읽음)
    void jobBusy;
    void formDirty;
    coord.poke();
  }

  // 진행 중 드롭 파일을 한가해질 때 목록에 추가(위 반응문보다 뒤에 배치)
  $: if (initialized && !busy && droppedWhileBusy.length > 0) {
    const queued = droppedWhileBusy;
    droppedWhileBusy = [];
    addInputs(queued);
  }

  // 창 제목 = 현재 언어, Rust 초기 제목 덮어쓰기, 언어 변경 시 재적용
  $: setCurrentWindowTitle($t("compress.windowTitle")).catch(() => {});

  onMount(async () => {
    // 리스너를 회수보다 먼저 단다(회수는 IPC 왕복), 초기화 전이므로 기억만 해 둔다
    unlistenAdd = await onCompressTakeInputs(() => {
      coord.signal();
    });
    // 이 창의 드롭 파일도 목록에 추가 — 진행 중이면 버퍼 경유
    unlistenDrop = await onFileDrop((paths) => {
      if (busy) droppedWhileBusy = [...droppedWhileBusy, ...(paths || [])];
      else addInputs(paths);
    });

    // 이 창의 압축 작업 진행률/완료 표시, 초기 회수보다 먼저 등록 — 회수가 곧바로 작업 시작
    const offP = await onJobProgress((p) => dispatch("progress", p));
    const offD = await onJobDone((d) => dispatch("done", d));
    const offE = await onJobError((e) => dispatch("error", e));
    unlistenJobs = [offP, offD, offE];

    // 새 창 표시 시 보관 요청 회수 후 적용(배치/자동 모드면 여기서 시작)
    try {
      coord.signal();
      await coord.settled();
    } finally {
      // 최초 시작 판단까지 끝난 뒤 외부 요청을 받는다, 실패해도 반드시 세울 것
      initialized = true;
    }
  });

  /** 폼을 새 작업 기준으로 초기화(앞 작업의 입력, 출력, 옵션 미승계) */
  function resetForm() {
    // 진행 화면에서 줄인 창을 폼 크기로 복원, 미복원 시 460×240 폼
    if (phase !== "form") resizeCurrentWindow(...FORM_SIZE).catch(() => {});
    phase = "form";
    jobResult = null;
    jobPercent = 0;
    jobFile = "";
    jobId = null;
    startError = "";
    pendingEvents = [];
    inputs = [];
    selected = new Set();
    statMap = {};
    folderFiles = {};
    output = "";
    outputSuggested = false;
    // 옵션도 초기화, 잔존 시 즉시 zip 암호화 + eachMode 가 단일 출력 무시
    format = FORM_DEFAULTS.format;
    level = FORM_DEFAULTS.level;
    password = FORM_DEFAULTS.password;
    encryptNames = FORM_DEFAULTS.encryptNames;
    eachMode = FORM_DEFAULTS.eachMode;
    showPasswordPanel = FORM_DEFAULTS.showPasswordPanel;
  }

  /**
   * 계획대로 상태 적용 + 자동, 배치면 시작, 화살표 한 줄로 둘 것(값을 버릴 자리를 없앤다)
   * 순서와 성패 전달은 창 밖 runPlan 담당(D3.5)
   */
  const applyPlan = (plan) => runPlan(plan, planActions);

  /** runPlan 이 부르는 창 쪽 동작, 상태 변경은 전부 여기서, */
  const planActions = {
    resetForm: () => resetForm(),
    // 탐색기 즉시 zip = 포맷/출력 사전 확정 → 자동 제안 차단 후 그대로 사용
    setFormat: (f) => {
      format = f;
    },
    clearBatchIssue: () => {
      batchIssue = false;
    },
    // "각각 압축": 원본마다 자기 아카이브로 순차 배치
    setBatch: (items) => {
      batchMode = true;
      batchItems = items;
      batchIndex = 0;
      outputSuggested = true; // 자동 제안 억제
    },
    clearBatch: () => {
      batchMode = false;
      batchItems = [];
      batchIndex = 0;
    },
    setOutput: (out) => {
      output = out;
      outputSuggested = true; // 자동 제안이 덮어쓰지 않게
    },
    addInputs: (paths) => addInputs(paths),
    settle: () => tick(),
    runBatch: () => {
      resizeCurrentWindow(...JOB_SIZE).catch(() => {});
      return runCompressItem();
    },
    startAuto: () => onStart({ internal: true }),
  };

  /**
   * 작업 이벤트 처리, jobId 배정 전 도착분은 버퍼 → drainPendingEvents 가 같은 경로로 재투입
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
      onJobFinished(ev);
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

  /** 작업 완료 처리, */
  function onJobFinished(d) {
    // 배치("각각 압축"): 다음 원본으로 넘어가거나, 마지막이면 완료
    if (batchMode && d.status !== "canceled") {
        // ok 아닌 것은 전부 흠집, warning 을 세지 않으면 마지막 항목의 성공이 덮는다
      batchIssue = batchIssueAfter(batchIssue, d.status);
      if (batchIndex < batchItems.length - 1) {
        batchIndex++;
        runCompressItem();
        return;
      }
      phase = "done";
      jobPercent = 100;
      // 앞 항목의 실패를 마지막 항목의 성공으로 덮지 않는다
      jobResult = {
        status: batchIssue ? "warning" : "ok",
        message: get(t)("compress.doneOk"),
      };
      return;
    }
    phase = "done";
    // 성공 시 진행률 100 마감, 작은 아카이브는 진행률 콜백이 한 번도 오지 않는다
    if (d.status !== "canceled") jobPercent = 100;
    // 완료/취소 메시지 = 상태 기반 번역(백엔드 원문은 한국어 고정)
    const tr = get(t);
    const msg = d.status === "canceled" ? tr("compress.doneCanceled") : tr("compress.doneOk");
    jobResult = { status: d.status, message: msg };
  }

  /** 작업 실패 처리, */
  function onJobErrored(e) {
    // 배치: 이 원본 실패는 건너뛰고 다음으로(다만 전체 결과는 ok 가 아니다)
    if (batchMode) {
      batchIssue = true;
      if (batchIndex < batchItems.length - 1) {
        batchIndex++;
        runCompressItem();
      } else {
        phase = "done";
        jobResult = { status: "warning", message: get(t)("compress.doneOk") };
      }
      return;
    }
    phase = "done";
    jobResult = { status: "error", message: errText(get(t), e.code, e.message) };
  }

  /** 배치의 현재 원본을 자기 이름의 zip 으로 압축 */
  async function runCompressItem() {
    const item = batchItems[batchIndex];
    // 직전 항목 id 제거, 남기면 이 항목 이벤트가 jobId 불일치로 버려진다
    jobId = null;
    // job 등록까지 진행 중 유지, 여기서 내리면 새 요청이 batchItems 를 갈아치운다
    starting = true;
    try {
      const id = await createArchive({
        output: item.output,
        inputs: [item.input],
        format,
        level,
        password: supportsPassword ? password : "",
        encryptNames: supportsEncryptNames && encryptNames,
      });
      jobId = id;
      jobPercent = 0;
      jobFile = fileNameOf(item.input);
      jobResult = null;
      phase = "running";
      starting = false; // 등록 완료 — 이후 진행률/완료 이벤트가 이 작업을 견인
      drainPendingEvents();
      return true;
    } catch (err) {
      // 시작 실패 → 다음 항목으로(전체 결과는 ok 가 아니다)
      batchIssue = true;
      if (batchIndex < batchItems.length - 1) {
        batchIndex++;
        return await runCompressItem(); // 다음 항목이 starting 을 이어받음
      }
      phase = "done";
      jobResult = { status: "warning", message: get(t)("compress.doneOk") };
      starting = false;
      // 하나도 시작하지 못했다, 조율자가 이 값을 보고 마감을 미룬다
      return false;
    }
  }

  /** 경로에서 파일명만, */
  function fileNameOf(p) {
    const s = String(p ?? "");
    const i = Math.max(s.lastIndexOf("\\"), s.lastIndexOf("/"));
    return i >= 0 ? s.slice(i + 1) : s;
  }

  onDestroy(() => {
    if (unlistenAdd) unlistenAdd();
    if (unlistenDrop) unlistenDrop();
    for (const off of unlistenJobs) off && off();
  });

  // 입력 최초 생성 시 출력 경로 1회 제안(이후 사용자 편집 존중)
  $: if (!outputSuggested && inputs.length > 0) {
    output = suggestOutput(inputs, format);
    outputSuggested = true;
  }

  // 입력 목록 변경 시 크기/폴더 여부 재조회
  $: refreshStats(inputs);

  // 포맷 특성
  $: supportsPassword = format !== "tar";
  $: supportsEncryptNames = format === "7z";
  // zip = 7z 엔진 제약으로 비ASCII(한글 등) 암호 거부
  $: zipNonAsciiPw = format === "zip" && password !== "" && /[^\x00-\x7F]/.test(password);
  // 암호가 실제로 설정되어 유효한 상태인지(버튼 표기용)
  $: passwordSet = supportsPassword && password !== "" && !zipNonAsciiPw;

  // 각각 압축 = 항목마다 출력 계산 → 단일 출력 경로 불필요
  // canRun = 값의 문제(이 폼으로 압축 가능한가), canStart = 타이밍(지금 눌러도 되나), 자동 시작은 canRun 만(D3.5)
  $: canRun = inputs.length > 0 && (eachMode || !!output) && !zipNonAsciiPw;
  $: canStart = canRun && !busy;

  // 시작 불가 사유(인라인 안내), $t 참조로 locale 변경 추적
  $: startHint =
    inputs.length === 0
      ? $t("compress.hintAddFiles")
      : !eachMode && !output
        ? $t("compress.hintOutput")
        : zipNonAsciiPw
          ? $t("compress.hintZipPw")
          : "";

  /** 비교용 경로 정규화(구분자 통일 + 끝 구분자 제거 + 소문자 — Windows 는 대소문자 무시), */
  function normPath(p) {
    return String(p ?? "")
      .replace(/\\/g, "/")
      .replace(/\/+$/, "")
      .toLowerCase();
  }

  /**
   * 경로 추가(순서 유지 + 포함관계 중복 제거)
   * 기존 항목이나 추가된 폴더 안이면 건너뛰기, 새 폴더면 그 안의 기존 개별 항목 제거
   */
  function addInputs(paths) {
    if (!paths || paths.length === 0) return;
    let next = [...inputs];
    for (const raw of paths) {
      if (!raw) continue;
      const np = normPath(raw);
      // 이미 있거나 기존 항목(폴더) 안에 포함되면 스킵
      const covered = next.some((e) => {
        const ne = normPath(e);
        return ne === np || np.startsWith(ne + "/");
      });
      if (covered) continue;
      // 새 항목 안에 포함된 기존 개별 항목은 제거(새 폴더가 대체)
      next = next.filter((e) => !normPath(e).startsWith(np + "/"));
      next.push(raw);
    }
    inputs = next;
  }

  /** 목록에서 특정 경로 제거, */
  function removeInput(path) {
    inputs = inputs.filter((p) => p !== path);
  }

  /**
   * 입력 목록의 크기, 폴더 여부, 하위 개수 → statMap, 모르는 경로만 조회, 빠진 것은 제거
   */
  async function refreshStats(list) {
    if (!list || list.length === 0) {
      statMap = {};
      return;
    }
    // 기존 값 유지, 빠진 것 제거, 모르는 것만 조회 대상
    const kept = {};
    const missing = [];
    for (const p of list) {
      if (statMap[p]) kept[p] = statMap[p];
      else missing.push(p);
    }
    statMap = kept; // 제거를 즉시 반영
    if (missing.length === 0) return;

    const token = ++statToken;
    try {
      const infos = await statPaths(missing);
      if (token !== statToken) return; // 더 새로운 요청이 있으면 폐기
      const next = { ...statMap };
      for (const it of infos) next[it.path] = it;
      statMap = next;
    } catch {
      // 조회 실패 시 크기는 미표시로 둔다(치명적 아님)
    }
  }

  /**
   * 폴더 입력의 내부 파일 목록 → folderFiles(모르는 경로만), 파일 경로는 빈 목록이라 안전
   */
  async function refreshFolderFiles(list) {
    if (!list || list.length === 0) {
      folderFiles = {};
      return;
    }
    const kept = {};
    const missing = [];
    for (const p of list) {
      if (folderFiles[p]) kept[p] = folderFiles[p];
      else missing.push(p);
    }
    folderFiles = kept;
    if (missing.length === 0) return;

    const token = ++ffToken;
    try {
      const results = await Promise.all(missing.map((p) => listFolderFiles(p).then((f) => [p, f])));
      if (token !== ffToken) return;
      const next = { ...folderFiles };
      for (const [p, files] of results) next[p] = files;
      folderFiles = next;
    } catch {
      // 조회 실패는 치명적이지 않다(내부 목록만 미표시)
    }
  }

  // 입력이 바뀌면 내부 파일 목록도 갱신
  $: refreshFolderFiles(inputs);

  // 표시용 행 목록, inputs, statMap, folderFiles 중 하나라도 바뀌면 재계산
  // 템플릿에서 함수 직접 호출 금지(변화 추적 안 됨), 폴더는 헤더 한 줄로만 표시
  $: rows = buildRows(inputs, statMap, folderFiles);

  function buildRows(list, sm, ff) {
    return list.map((p) => {
      const info = sm[p];
      const known = !!info;
      const isDir = known ? info.isDir : false;
      if (!isDir) {
        // 파일 (또는 아직 종류 미상)
        return {
          kind: "file",
          path: p,
          name: baseName(p),
          size: known ? info.size : 0,
          known,
        };
      }
      // 폴더 — 내부 파일 목록에서 개수와 합계 크기 산출(미전개)
      const files = ff[p]; // [{rel,size}] 또는 미조회(undefined)
      const total = files ? files.reduce((a, f) => a + (f.size || 0), 0) : 0;
      return {
        kind: "folder",
        path: p,
        name: baseName(p),
        count: files ? files.length : null,
        size: total,
        known: !!files,
      };
    });
  }

  /** 크기 칸 표시 문자열, 미조회는 "…", 그 외 사람이 읽는 크기(폴더는 하위 합계), */
  function sizeText(row) {
    if (!row.known) return "…";
    return formatSize(row.size);
  }

  /** 경로를 현재 OS 형식으로 통일(Windows 형식이면 역슬래시), 혼합 구분자 방지 */
  function toOsPath(p) {
    const s = String(p ?? "");
    // 드라이브 문자(C:) 나 역슬래시가 있으면 Windows 경로로 보고 역슬래시로 통일
    const winLike = /^[A-Za-z]:/.test(s) || s.includes("\\");
    return winLike ? s.replace(/\//g, "\\") : s.replace(/\\/g, "/");
  }

  /** 첫 입력의 폴더/이름 기준 출력 경로 제안(OS 형식) */
  function suggestOutput(list, fmt) {
    if (!list || list.length === 0) return "";
    const raw = String(list[0]);
    // 양쪽 구분자 기준으로 마지막 조각(이름) 분리
    const norm = raw.replace(/[\\/]+/g, "/");
    const sepIdx = norm.lastIndexOf("/");
    const dir = sepIdx >= 0 ? norm.slice(0, sepIdx) : "";
    let base = sepIdx >= 0 ? norm.slice(sepIdx + 1) : norm;
    const dot = base.lastIndexOf(".");
    if (dot > 0) base = base.slice(0, dot);
    const name = `${base}.${FORMAT_EXT[fmt]}`;
    const full = dir ? dir + "/" + name : name;
    // 전체 경로를 OS 구분자로 통일(혼합 구분자 방지)
    return toOsPath(full);
  }

  /** 포맷을 바꾸면 출력 파일의 확장자도 맞춰 바꾼다, */
  function onFormatChange() {
    if (output) {
      const ext = FORMAT_EXT[format];
      // 알려진 확장자면 치환, 아니면 덧붙인다
      const m = output.match(/\.(7z|zip|tar)$/i);
      output = m ? output.slice(0, m.index) + "." + ext : output + "." + ext;
    }
    // tar 로 바뀌면 암호 관련 값은 의미 없으므로 정리
    if (format === "tar") {
      password = "";
      showPasswordPanel = false;
    }
    if (format !== "7z") encryptNames = false;
  }

  /** 표시용 파일명(경로의 마지막 조각), */
  function baseName(path) {
    const norm = String(path).replace(/\\/g, "/");
    return norm.substring(norm.lastIndexOf("/") + 1) || path;
  }

  let anchorIndex = -1; // Shift+클릭 범위 선택 기준 인덱스

  /** 행 선택 토글, additive(Ctrl/⌘) 이면 누적, 아니면 단일 선택, */
  function toggleSelect(path, additive) {
    const next = new Set(additive ? selected : []);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    selected = next;
  }

  /** 누르는 즉시(마우스 다운) 선택, 좌클릭만, Ctrl 누적 / Shift 범위 지원, */
  function onRowMouseDown(e, path, idx) {
    if (e.button !== 0) return;
    if (e.shiftKey) {
      const a = anchorIndex >= 0 && anchorIndex < rows.length ? anchorIndex : idx;
      const lo = Math.min(a, idx);
      const hi = Math.max(a, idx);
      selected = new Set(rows.slice(lo, hi + 1).map((r) => r.path));
    } else if (e.ctrlKey || e.metaKey) {
      toggleSelect(path, true);
      anchorIndex = idx;
    } else {
      toggleSelect(path, false);
      anchorIndex = idx;
    }
  }

  function onRowKey(e, path, idx) {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      toggleSelect(path, e.ctrlKey || e.metaKey);
      anchorIndex = idx;
    }
  }

  async function onAddFiles() {
    const picked = await pickInputFiles();
    if (picked.length > 0) addInputs(picked);
  }

  /** 폴더 선택 대화상자(Windows 폴더 전용 = FOS_PICKFOLDERS)로 폴더 추가 */
  async function onAddFolders() {
    const picked = await pickInputFolders();
    if (picked.length > 0) addInputs(picked);
  }

  /** 선택 행 목록에서 제거 */
  function onRemoveSelected() {
    if (selected.size === 0) return;
    for (const p of selected) removeInput(p);
    selected = new Set();
  }

  async function onPickOutput() {
    const picked = await pickSaveArchive(output);
    if (picked) {
      output = picked;
      outputSuggested = true; // 사용자가 직접 지정했으므로 자동 제안 중단
      // 저장 다이얼로그에서 고른 확장자에 맞춰 포맷을 동기화
      const m = picked.match(/\.(7z|zip|tar)$/i);
      if (m) {
        const ext = m[1].toLowerCase();
        if (ext !== format) {
          format = ext;
          if (format === "tar") {
            password = "";
            showPasswordPanel = false;
          }
          if (format !== "7z") encryptNames = false;
        }
      }
    }
  }

  /** [압축 시작] — createArchive 호출 후 이 창에서 진행률/완료 표시(창 유지) */
  async function onStart(opts) {
    // 조율자의 자동 시작(internal)은 busy 미참조 — 그 busy 가 이 요청 자신
    if (opts?.internal ? !canRun : !canStart) return false;
    // 실행 인자 고정 지점, 아래 await 사이의 파일/암호 변경이 함께 실려 가는 것 방지
    const req = {
      output,
      inputs: [...inputs],
      format,
      level,
      password: supportsPassword ? password : "",
      encryptNames: supportsEncryptNames && encryptNames,
      eachMode,
    };
    starting = true;
    startError = "";
    try {
      // 각각 압축 = 항목별 출력 경로 계산 후 순차 배치 압축(폼 옵션 반영)
      if (req.eachMode) {
        const items = await planEachCompress(req.inputs, req.format);
        if (items.length === 0) {
          startError = get(t)("compress.startFailed");
          starting = false;
          return false;
        }
        batchItems = items;
        batchIndex = 0;
        batchIssue = false;
        batchMode = true;
        resizeCurrentWindow(...JOB_SIZE).catch(() => {});
        return await runCompressItem();
      }
      const id = await createArchive({
        output: req.output,
        inputs: req.inputs,
        format: req.format,
        level: req.level,
        password: req.password,
        encryptNames: req.encryptNames,
      });
      // 작업이 시작됨 → 진행 화면으로 전환, 진행률/완료는 job 이벤트로 이 창에 표시
      jobId = id;
      // job 등록 순간 starting 해제, 유지 시 busy 가 안 내려가 미뤄 둔 요청 영구 잔존
      starting = false;
      jobPercent = 0;
      jobFile = "";
      jobResult = null;
      phase = "running";
      // 진행/완료 화면은 설정 폼보다 훨씬 작으므로 창을 컴팩트하게 줄인다
      resizeCurrentWindow(...JOB_SIZE).catch(() => {});
      // jobId 확정 시점, 그 사이 먼저 도착한 이벤트 처리
      drainPendingEvents();
      return true;
    } catch (err) {
      // 즉시 실패 시 설정 화면 유지 + 인라인 안내, 자동 시작 실패면 폼 크기 복원
      startError = errText(get(t), err && err.code, get(t)("compress.startFailed"));
      starting = false;
      resizeCurrentWindow(...FORM_SIZE).catch(() => {});
      // 시작하지 못했다, 조율자는 이 값을 보고 요청을 마감하지 않는다
      return false;
    }
  }

  /** 압축 중 [취소] — 백엔드 작업 취소(완료는 job:done status="canceled") */
  async function onCancelJob() {
    if (!jobId) return;
    try {
      await cancelJob(jobId);
    } catch (err) {
      console.error("작업 취소 실패:", err);
    }
  }

  /** 창 닫기(설정 화면의 [취소]/[✕], 완료 화면의 [닫기]), */
  async function onCancel() {
    await closeCurrentWindow();
  }
</script>

<div class="win" data-ui="compress-window">
  {#if phase === "form"}
  <!-- 본문 2단 -->
  <div class="body">
    <!-- ── 좌측 열 ─────────────────────────────── -->
    <div class="col left">
      <!-- 압축할 파일 목록 -->
      <div class="block list-block">
        <span class="block-title">{$t("compress.listTitle", { count: inputs.length })}</span>
        <div class="list-wrap">
          <table class="list">
            <thead>
              <tr>
                <th class="c-name">{$t("compress.colName")}</th>
                <th class="c-size">{$t("compress.colSize")}</th>
                <th class="c-path">{$t("compress.colPath")}</th>
              </tr>
            </thead>
            <tbody>
              {#if inputs.length === 0}
                <tr class="empty-row">
                  <td colspan="3">
                    {$t("compress.emptyList")}
                  </td>
                </tr>
              {:else}
                {#each rows as row, i (row.path)}
                  <tr
                    class="item"
                    class:sel={selected.has(row.path)}
                    role="button"
                    tabindex="0"
                    aria-pressed={selected.has(row.path)}
                    on:mousedown={(e) => onRowMouseDown(e, row.path, i)}
                    on:keydown={(e) => onRowKey(e, row.path, i)}
                  >
                    <td class="c-name" title={row.path}>
                      <span class="ic">{row.kind === "folder" ? "📁" : "📄"}</span>
                      <span class="txt"
                        >{row.name}{row.kind === "folder" && row.count != null
                          ? " " + $t("compress.folderCount", { count: row.count })
                          : ""}</span
                      >
                    </td>
                    <td class="c-size">{sizeText(row)}</td>
                    <td class="c-path" title={row.path}>{row.path}</td>
                  </tr>
                {/each}
              {/if}
            </tbody>
          </table>
        </div>
        <div class="list-actions">
          <button class="ghost small" on:click={onAddFiles}>{$t("compress.addFiles")}</button>
          <button class="ghost small" on:click={onAddFolders}>{$t("compress.addFolders")}</button>
          <button
            class="ghost small"
            on:click={onRemoveSelected}
            disabled={selected.size === 0}
          >{$t("compress.removeSelected")}</button>
        </div>
      </div>

      <hr class="sep" />

      <!-- 압축 파일 설정 -->
      <div class="block">
        <span class="block-title">{$t("compress.settingsTitle")}</span>

        <!-- 파일 이름(출력 경로) — "각각 압축" 이면 항목별로 계산되므로 비활성. -->
        <div class="row">
          <span class="lb">{$t("compress.fileName")}</span>
          <div class="grow path-row">
            <input type="text" bind:value={output} disabled={eachMode} />
            <button class="ghost small" title={$t("compress.pickOutputTitle")} on:click={onPickOutput} disabled={eachMode}>…</button>
          </div>
        </div>
        {#if eachMode}
          <p class="each-note">{$t("compress.eachNameNote")}</p>
        {/if}

        <!-- 압축 형식 + 암호 설정(누르면 오른쪽 옆으로 펼쳐진다) -->
        <div class="row">
          <span class="lb">{$t("compress.format")}</span>
          <div class="grow inline">
            <select bind:value={format} on:change={onFormatChange}>
              <option value="7z">7Z</option>
              <option value="zip">ZIP</option>
              <option value="tar">TAR</option>
            </select>
            <button
              class="ghost small"
              class:has-pw={passwordSet}
              disabled={!supportsPassword}
              title={supportsPassword ? $t("compress.passwordTitle") : $t("compress.passwordTitleTar")}
              on:click={() => (showPasswordPanel = !showPasswordPanel)}
            >{passwordSet ? $t("compress.passwordSet") : $t("compress.password")}</button>

            {#if showPasswordPanel && supportsPassword}
              <input type="password" class="pw-in" bind:value={password} placeholder={$t("common.password")} />
              {#if supportsEncryptNames}
                <label class="check pw-check" title={$t("compress.encryptNamesTitle")}>
                  <input type="checkbox" bind:checked={encryptNames} disabled={password === ""} />
                  {$t("compress.encryptNames")}
                </label>
              {/if}
            {/if}
          </div>
        </div>

        <!-- 암호 관련 경고(있을 때만, 행 아래) -->
        {#if showPasswordPanel && zipNonAsciiPw}
          <p class="warn-text">
            {$t("compress.zipNonAsciiWarn")}
          </p>
        {/if}

        <!-- 분할 압축 (Phase 2, 비활성) -->
        <div class="row">
          <span class="lb">{$t("compress.split")}</span>
          <div class="grow">
            <select class="full" disabled title={$t("common.comingSoon")}>
              <option>{$t("compress.splitNone")}</option>
            </select>
          </div>
        </div>

        <!-- 압축 방법 (실제 동작) -->
        <div class="row">
          <span class="lb">{$t("compress.method")}</span>
          <div class="grow">
            <select class="full" bind:value={level}>
              {#each LEVELS as lv}
                <option value={lv.value}>{$t(lv.labelKey)}</option>
              {/each}
            </select>
          </div>
        </div>

        <!-- 압축 후 처리 옵션 -->
        <div class="p2-checks">
          <label class="check dis" title={$t("common.comingSoon")}>
            <input type="checkbox" disabled /> {$t("compress.afterTest")}
          </label>
          <label class="check dis" title={$t("common.comingSoon")}>
            <input type="checkbox" disabled /> {$t("compress.afterDelete")}
          </label>
          <label class="check" title={$t("compress.eachNameTitle")}>
            <input type="checkbox" bind:checked={eachMode} /> {$t("compress.eachName")}
          </label>
        </div>
      </div>
    </div>
  </div>

  <!-- 시작 실패 인라인 알림 -->
  {#if startError}
    <div class="start-error" role="alert">⚠ {startError}</div>
  {/if}

  <!-- 하단 액션 바 -->
  <div class="actions">
    {#if !canStart && startHint}
      <span class="hint">{startHint}</span>
    {/if}
    <div class="spacer"></div>
    <button class="ghost" on:click={onCancel}>{$t("common.cancel")}</button>
    <button class="primary" on:click={() => onStart()} disabled={!canStart}>{$t("compress.start")}</button>
  </div>
  {:else if phase === "running"}
    <!-- 진행 화면 -->
    <div class="job-view">
      <div class="job-title">{$t("compress.running")}</div>
      <div class="bar"><div class="fill" style="width: {jobPercent}%"></div></div>
      <div class="job-meta">
        <span class="pct">{jobPercent}%</span>
        <span class="file" title={jobFile}>{jobFile || $t("progress.preparing")}</span>
      </div>
      <div class="job-actions">
        <button class="ghost" on:click={onCancelJob}>{$t("common.cancel")}</button>
      </div>
    </div>
  {:else}
    <!-- 완료/실패 화면 -->
    <div class="job-view">
      <div class="result {resultCls}">
        <span class="r-ic">{resultIcon}</span>
        <span class="r-msg">{jobResult ? jobResult.message : ""}</span>
      </div>
      <div class="job-actions">
        <button class="primary" on:click={onCancel}>{$t("common.close")}</button>
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
    /* 목록, 라벨 등에서 클릭-드래그 시 텍스트 선택/드래그 차단 */
    user-select: none;
    -webkit-user-select: none;
  }
  /* 단, 입력칸은 텍스트 선택/편집 허용 */
  .win input,
  .win select {
    user-select: text;
    -webkit-user-select: text;
  }

  /* 본문 2단 — 창을 꽉 채우고 내부 스크롤 */
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
    gap: 18px;
    padding: 16px 18px;
    overflow-y: auto;
  }
  .col {
    min-width: 0;
  }
  .col.left {
    flex: 1 1 300px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .block {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  /* 목록 블록이 남는 세로 공간 차지 → 테이블이 창 높이에 맞춰 신장 */
  .list-block {
    flex: 1 1 auto;
    min-height: 0;
  }
  .block-title {
    font-weight: 600;
  }
  .sep {
    border: none;
    border-top: 1px solid var(--border);
    margin: 4px 0;
    width: 100%;
  }

  /* 파일 목록 테이블 */
  .list-wrap {
    border: 1px solid var(--border);
    border-radius: 6px;
    flex: 1 1 auto;
    min-height: 140px;
    overflow: auto;
  }
  table.list {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }
  table.list th {
    position: sticky;
    top: 0;
    text-align: left;
    font-weight: 600;    color: var(--text-muted);
    background: var(--surface);
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
    white-space: nowrap;
  }
  table.list td {
    padding: 5px 10px;
    border-bottom: 1px solid var(--border);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 0;
  }
  .c-size {
    width: 84px;
    white-space: nowrap;
  }
  .c-path {
    color: var(--text-muted);
  }
  tr.item {
    cursor: pointer;
  }
  tr.item:hover td {
    background: var(--surface);
  }
  tr.item.sel td {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
  }
  tr.item:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  .c-name .ic {
    margin-right: 6px;
  }
  .empty-row td {
    text-align: center;
    color: var(--text-muted);
    padding: 24px 10px;
    white-space: normal;
  }
  .list-actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
  }

  /* 설정 행 */
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .lb {
    flex: 0 0 76px;    color: var(--text-muted);
  }
  .grow {
    flex: 1 1 auto;
    min-width: 0;
  }
  .inline {
    display: flex;
    gap: 8px;
    align-items: stretch; /* 콤보박스와 버튼 높이를 동일하게 맞춘다 */
  }
  /* 압축 형식 콤보박스와 [암호 설정] 버튼의 높이를 통일, */
  .inline select,
  .inline button {
    height: 32px;
    padding-top: 0;
    padding-bottom: 0;
  }
  .path-row {
    display: flex;
    gap: 8px;
  }

  input[type="text"],
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
  input:disabled,
  select:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  select {
    padding: 7px 9px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--btn-bg);
    color: var(--text);
    font-size: 13px;
  }
  select.full {
    width: 100%;
  }

  /* 암호 입력 — 형식 행 오른쪽에 200px 고정 폭(7Z 의 '파일명' 체크박스 공간 확보)
     전역 input[type=password]{flex:1 1 auto} 보다 특이성을 높여 100% 로 늘어나지 않게 한다. */
  .inline input.pw-in {
    flex: 0 0 200px;
    width: 200px;
    min-width: 0;
  }
  .pw-check {
    flex: 0 0 auto;    color: var(--text-muted);
    white-space: nowrap;
  }
  .has-pw {
    border-color: var(--accent);
    color: var(--accent);
  }

  /* 체크박스 */
  .check {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
    color: var(--text);
    cursor: pointer;
  }
  .check.dis {
    color: var(--text-muted);
    cursor: not-allowed;
  }
  .p2-checks {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 2px;
  }

  .warn-text {
    margin: 0;    color: var(--warn-text, #8a5a12);
  }
  .each-note {
    margin: 2px 0 0;
    color: var(--text-muted);
    font-size: 12px;
  }

  /* 시작 실패 알림 */
  .start-error {
    padding: 8px 18px;
    border-top: 1px solid var(--border);
    background: var(--alert-bg, #fde8e8);
    color: var(--alert-text, #9b1c1c);
    font-size: 13px;
  }

  /* 하단 */
  .actions {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 18px;
    border-top: 1px solid var(--border);
  }
  .hint {
    color: var(--text-muted);
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
  .ghost {
    background: var(--btn-bg);
    color: var(--text);
  }
  .primary {
    background: var(--accent);
    color: var(--accent-contrast);
    border-color: var(--accent);
  }
  /* 진행/완료 화면 */
  .job-view {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    padding: 16px;
    text-align: center;
  }
  .job-title {
    color: var(--text-muted);
  }
  .bar {
    width: min(420px, 80%);
    height: 10px;
    border-radius: 5px;
    background: var(--border);
    overflow: hidden;
  }
  .fill {
    height: 100%;
    background: var(--accent);
    transition: width 0.2s ease;
  }
  .job-meta {
    display: flex;
    gap: 10px;
    align-items: center;
    font-size: 13px;
    color: var(--text-muted);
    max-width: 80%;
  }
  .job-meta .pct {
    font-variant-numeric: tabular-nums;
    font-weight: 600;
    color: var(--text);
  }
  .job-meta .file {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .result {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 13px;
    padding: 14px 20px;
    border-radius: 8px;
  }
  .result .r-ic {  }
  .result.ok {
    background: var(--ok-bg, #e6f4ea);
    color: var(--ok-text, #1e6b34);
  }
  .result.warn {
    background: var(--warn-bg, #fdf3e0);
    color: var(--warn-text, #8a5a12);
  }
  .result.cancel {
    background: var(--surface);
    color: var(--text-muted);
  }
  .result.err {
    background: var(--alert-bg, #fde8e8);
    color: var(--alert-text, #9b1c1c);
  }
  .job-actions {
    display: flex;
    gap: 8px;
  }

</style>
