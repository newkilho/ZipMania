<script>
  // 루트 컴포넌트 — 전체 레이아웃 구성 + 전역 이벤트(버전 조회, 드래그&드롭) 연결
  import { onMount, onDestroy } from "svelte";
  import { get } from "svelte/store";
  import {
    currentWindowLabel,
    sevenzipVersion,
    onFileDrop,
    onOpenArchive,
    takeStartupOpen,
    takeViewerArchive,
    setCurrentWindowTitle,
  } from "./lib/api.js";
  import { t } from "./lib/i18n.js";
  import {
    engineVersion,
    archivePath,
    uiError,
    openDroppedPaths,
    openArchiveByPath,
    closeError,
    initJobEvents,
  } from "./lib/stores.js";

  import Toolbar from "./components/Toolbar.svelte";
  import AssocBanner from "./components/AssocBanner.svelte";
  import FolderTree from "./components/FolderTree.svelte";
  import PreviewPane from "./components/PreviewPane.svelte";
  import FileTable from "./components/FileTable.svelte";
  import StatusBar from "./components/StatusBar.svelte";
  import EmptyState from "./components/EmptyState.svelte";
  import ProgressPanel from "./components/ProgressPanel.svelte";
  import PasswordDialog from "./components/dialogs/PasswordDialog.svelte";
  import TestResultDialog from "./components/dialogs/TestResultDialog.svelte";
  import ScanResultDialog from "./components/dialogs/ScanResultDialog.svelte";

  // 스킨 미리보기 = 레이아웃만 렌더, Tauri 창과 이벤트 연결 생략
  export let preview = false;

  // 파일 연결 안내는 메인 창 전용, 뷰어 창(viewer-N)도 이 컴포넌트 사용
  const isMainWindow = currentWindowLabel() === "main";

  // 메인 창 제목 = 현재 언어의 앱 이름, 언어 변경 시 재적용
  // 뷰어 창(viewer-N)은 제목이 아카이브 파일명이라 제외
  $: if (!preview && isMainWindow) {
    setCurrentWindowTitle($t("app.name")).catch(() => {});
  }

  // WebView 기본 우클릭 메뉴 차단, 목록, 트리는 각자 시스템 네이티브 메뉴를 띄운다
  function onGlobalContextMenu(e) {
    e.preventDefault();
  }

  let unlistenDrop = null;
  let unlistenJobs = null;
  let unlistenOpen = null;

  // ── 폴더/파일 목록 사이 크기 조절(스플리터) ──
  const LEFT_MIN = 160; // 좌측(폴더 트리) 최소 너비
  const RIGHT_MIN = 320; // 우측(파일 목록) 최소 너비
  let leftWidth = 240; // 좌측 열 현재 너비(px)
  let workspaceEl; // 너비 계산 기준 컨테이너
  let dragging = false;

  function startDrag(e) {
    e.preventDefault();
    dragging = true;
    window.addEventListener("mousemove", onDrag);
    window.addEventListener("mouseup", endDrag);
  }

  function onDrag(e) {
    if (!workspaceEl) return;
    const rect = workspaceEl.getBoundingClientRect();
    let w = e.clientX - rect.left; // 좌측 열 = 컨테이너 좌단 ~ 커서
    const max = rect.width - RIGHT_MIN; // 우측 최소 너비 보장
    if (w < LEFT_MIN) w = LEFT_MIN;
    if (w > max) w = Math.max(LEFT_MIN, max);
    leftWidth = w;
  }

  function endDrag() {
    dragging = false;
    window.removeEventListener("mousemove", onDrag);
    window.removeEventListener("mouseup", endDrag);
  }

  /** 시작 시 --open 으로 지정된 아카이브가 있으면 연다, 회수는 1회라 두 번 열리지 않는다, */
  async function drainStartupOpen() {
    try {
      const archive = await takeStartupOpen();
      if (archive) await openArchiveByPath(archive);
    } catch (err) {
      console.error("시작 아카이브 회수 실패:", err);
    }
  }

  onMount(async () => {
    if (preview) return;

    // 최우선 개시, 신호와 mount 회수 병용, 메인 창만 회수(회수는 전역 1회)
    if (isMainWindow) {
      unlistenOpen = await onOpenArchive(() => drainStartupOpen());
      await drainStartupOpen();
    }

    // 시작 시 7z 엔진 버전을 받아 상태줄에 표시 (IPC 관통 검증)
    try {
      engineVersion.set(await sevenzipVersion());
    } catch (err) {
      engineVersion.set(get(t)("app.engineLoadFailed"));
      console.error("7z 버전 조회 실패:", err);
    }

    // 창 전체 드래그&드롭 구독 — 어디에 드롭해도 아카이브를 연다
    unlistenDrop = await onFileDrop((paths) => openDroppedPaths(paths));

    // 해제 작업 이벤트(job:progress/done/error) 구독
    unlistenJobs = await initJobEvents();

    // 뷰어 창(viewer-N)이면 자기 아카이브를 받아 연다, 메인 창은 항상 null
    try {
      const nested = await takeViewerArchive();
      if (nested) await openArchiveByPath(nested);
    } catch {
      /* 뷰어 창이 아니거나 이미 회수됨 */
    }
  });

  onDestroy(() => {
    if (unlistenDrop) unlistenDrop();
    if (unlistenJobs) unlistenJobs();
    if (unlistenOpen) unlistenOpen();
    endDrag(); // 혹시 드래그 중 언마운트되면 전역 리스너 정리
  });
</script>

<svelte:window on:contextmenu={onGlobalContextMenu} />

<Toolbar />

{#if $uiError}
  <div class="alert" data-ui="alert" role="alert">
    <span class="alert-msg">⚠ {$uiError.message}</span>
    <button class="alert-close" on:click={closeError} title={$t("common.close")}>✕</button>
  </div>
{/if}

<!-- 오류(빨강) 아래에 둔다. 급한 것이 위다. -->
{#if !preview && isMainWindow}
  <AssocBanner />
{/if}

<main data-ui="app-main">
  {#if $archivePath}
    <div class="workspace" data-ui="workspace" class:dragging bind:this={workspaceEl}>
      <div class="left-pane" data-ui="sidebar" style="width: {leftWidth}px;">
        <FolderTree />
        <PreviewPane />
      </div>
      <!-- 폴더/파일 목록 사이 크기 조절 손잡이 -->
      <!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
      <div
        class="gutter"
        data-ui="splitter"
        role="separator"
        aria-orientation="vertical"
        title={$t("app.gutterTitle")}
        on:mousedown={startDrag}
      ></div>
      <FileTable />
    </div>
  {:else}
    <EmptyState />
  {/if}
</main>

<ProgressPanel />
<StatusBar {preview} />

<PasswordDialog />
<TestResultDialog />
<ScanResultDialog />

<style>
  main {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    min-height: 0;
  }
  .workspace {
    flex: 1;
    display: flex;
    flex-direction: row;
    overflow: hidden;
    min-height: 0;
  }
  .workspace :global(.table) {
    min-width: 0;
  }
  /* 좌측 열: 폴더 트리(위) + 이미지 미리보기(아래)를 세로로 쌓는다
     너비는 스플리터 드래그로 조절되며 인라인 style 로 지정된다. */
  .left-pane {
    display: flex;
    flex-direction: column;
    flex-shrink: 0;
    overflow: hidden;
  }
  /* 트리가 남는 높이 충전, 너비와 테두리는 좌측 열/스플리터 담당 */
  .left-pane :global(.folder-tree) {
    flex: 1;
    width: auto;
    min-width: 0;
    border-right: none;
  }
  /* 폴더/파일 목록 사이 크기 조절 손잡이(세로 스플리터)
     원래 5px 폭 그대로, 기본 색만 더 연하게. 마우스 오버/드래그 중엔 accent 강조. */
  .gutter {
    flex: 0 0 5px;
    cursor: col-resize;
    background: color-mix(in srgb, var(--border) 40%, transparent);
  }
  .gutter:hover,
  .workspace.dragging .gutter {
    background: var(--accent);
  }
  /* 드래그 중 텍스트 선택/커서 흔들림 방지, */
  .workspace.dragging {
    cursor: col-resize;
    user-select: none;
  }
  .alert {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    background: var(--alert-bg, #fde8e8);
    color: var(--alert-text, #9b1c1c);
    border-bottom: 1px solid var(--border);
    font-size: 13px;
  }
  .alert-msg {
    flex: 1;
  }
  .alert-close {
    border: none;
    background: transparent;
    color: inherit;
    cursor: pointer;
    font-size: 13px;
    padding: 2px 6px;
  }
</style>
