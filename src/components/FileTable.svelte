<script>
  // 파일 테이블 — 정렬 헤더, 폴더 더블클릭 진입, 외부 라이브러리 없는 가상 스크롤
  import { onDestroy } from "svelte";
  import {
    visibleRows,
    selectedPaths,
    archivePath,
    sortKey,
    sortDir,
    setSort,
    selectRow,
    setSelection,
    setSelectionPaths,
    enterFolder,
    openExtract,
    openInnerEntry,
    extractSelectedQuick,
    extractAllToArchiveFolder,
    addFilesToArchive,
    removeSelectedFromArchive,
    selectAllVisible,
    canEditArchive,
  } from "../lib/stores.js";
  import { formatSize, formatModified } from "../lib/format.js";
  import { beginShellDrag, showContextMenu } from "../lib/api.js";
  import { t } from "../lib/i18n.js";
  import FileIcon from "./FileIcon.svelte";

  const ROW_H = 28; // 한 행의 고정 높이(px), 가상 스크롤 계산 기준
  const OVERSCAN = 8; // 화면 밖 여유 행 수

  // 컬럼 너비(px), 이름, 크기, 압축크기, 종류는 고정폭, 수정일이 남는 폭 흡수(1fr)
  const COL_MIN = { name: 100, size: 60, packed: 60, kind: 56 }; // 각 컬럼 최소 너비
  const DATE_W = 168; // 수정일 최소 폭 — YYYY-MM-DD HH:MM:SS 전체가 들어가는 값, 줄이면 말줄임
  let colW = { name: 280, size: 94, packed: 110, kind: 90 };
  // 고정폭 4개 + '수정일' minmax(DATE_W,1fr), 각 컬럼은 넘치면 CSS 로 말줄임(…)
  $: colsCss = `${colW.name}px ${colW.size}px ${colW.packed}px ${colW.kind}px minmax(${DATE_W}px, 1fr)`;
  // 컬럼 합계 = 행의 최소 폭, 창이 좁아도 컬럼을 줄이지 않고 가로 스크롤
  $: rowMinW = colW.name + colW.size + colW.packed + colW.kind + DATE_W;

  let resizing = null; // { key, startX, startW }

  function startResize(e, key) {
    e.preventDefault();
    e.stopPropagation(); // 헤더 정렬(on:click)로 번지지 않게
    resizing = { key, startX: e.clientX, startW: colW[key] };
    window.addEventListener("mousemove", onResizeMove);
    window.addEventListener("mouseup", endResize);
  }
  function onResizeMove(e) {
    if (!resizing) return;
    const delta = e.clientX - resizing.startX;
    const min = COL_MIN[resizing.key] ?? 48;
    colW = { ...colW, [resizing.key]: Math.max(min, Math.round(resizing.startW + delta)) };
  }
  function endResize() {
    resizing = null;
    window.removeEventListener("mousemove", onResizeMove);
    window.removeEventListener("mouseup", endResize);
  }

  let scrollTop = 0;
  let viewportH = 0;
  let bodyEl; // 스크롤 뷰포트 — 키보드 포커스, 스크롤 조정 대상
  let headEl; // 헤더 클리핑 래퍼 — 본문의 scrollLeft 를 따라간다
  let anchorIndex = -1; // Shift+클릭 범위 선택의 기준(마지막 단일/Ctrl 클릭) 인덱스
  let pendingSinglePath = null; // 이미 선택된 다중 행을 눌렀을 때, 드래그 없이 떼면 단일선택할 경로

  // ── 마퀴(고무줄) 선택 — 빈 영역을 드래그해 사각형 안의 항목을 선택 ──
  let marquee = null; // { x, y, w, h } (콘텐츠 좌표) — 표시용, null=비활성
  let marqueeStart = null; // { x, y } 시작점(콘텐츠 좌표)
  let marqueeBase = []; // Ctrl 드래그 시 유지할 기존 선택

  $: rows = $visibleRows;
  $: total = rows.length;
  $: start = Math.max(0, Math.floor(scrollTop / ROW_H) - OVERSCAN);
  $: visibleCount = Math.ceil(viewportH / ROW_H) + OVERSCAN * 2;
  $: end = Math.min(total, start + visibleCount);
  $: slice = rows.slice(start, end);

  // 현재 활성(키보드 커서) 행 인덱스 — 선택된 것 중 첫 번째, 없으면 -1
  $: activeIndex = rows.findIndex((r) => $selectedPaths.has(r.path));

  function onScroll(e) {
    scrollTop = e.target.scrollTop;
    if (headEl) headEl.scrollLeft = e.target.scrollLeft; // 헤더 가로 동기화
  }

  function onRowMouseDown(e, row, idx) {
    // 마우스 다운 즉시 선택, 좌클릭만 처리
    if (e.button !== 0) return;
    e.stopPropagation(); // 빈 영역 마퀴 시작(onBodyMouseDown) 발동 차단
    pendingSinglePath = null;
    if (e.shiftKey) {
      // Shift+클릭: 기준(anchor)부터 현재 행까지 범위 선택
      const a = anchorIndex >= 0 && anchorIndex < total ? anchorIndex : idx;
      const lo = Math.min(a, idx);
      const hi = Math.max(a, idx);
      setSelectionPaths(rows.slice(lo, hi + 1).map((r) => r.path));
      // 기준 유지(연속 Shift+클릭으로 범위 증감 가능)
    } else if (e.ctrlKey || e.metaKey) {
      selectRow(row.path, true);
      anchorIndex = idx;
    } else if ($selectedPaths.has(row.path) && $selectedPaths.size > 1) {
      // 이미 다중 선택된 행 그냥 누름 → 선택 유지(드래그 대비), 드래그 없이 떼면 이 행 하나로 축소
      pendingSinglePath = row.path;
      anchorIndex = idx;
    } else {
      selectRow(row.path, false);
      anchorIndex = idx;
    }
    bodyEl?.focus(); // 이후 화살표 키가 목록에서 동작하도록 포커스를 뷰포트에 둔다
  }

  function onRowMouseUp() {
    // 드래그 없이 뗀 경우(클릭), 미뤄 둔 단일선택 확정
    if (pendingSinglePath != null) {
      setSelection(pendingSinglePath);
      pendingSinglePath = null;
    }
  }

  // 빈 영역에서 마우스 다운 → 마퀴 선택 시작(행에서는 stopPropagation 되어 오지 않는다)
  function onBodyMouseDown(e) {
    if (e.button !== 0 || !bodyEl) return;
    const rect = bodyEl.getBoundingClientRect();
    const x = e.clientX - rect.left + bodyEl.scrollLeft;
    const y = e.clientY - rect.top + bodyEl.scrollTop;
    marqueeStart = { x, y };
    marquee = { x, y, w: 0, h: 0 };
    // Ctrl 이면 기존 선택에 더하고, 아니면 빈 영역 클릭이므로 선택 해제부터 시작
    marqueeBase = e.ctrlKey || e.metaKey ? Array.from($selectedPaths) : [];
    if (!(e.ctrlKey || e.metaKey)) setSelectionPaths([]);
    bodyEl.focus();
    e.preventDefault();
    window.addEventListener("mousemove", onMarqueeMove);
    window.addEventListener("mouseup", onMarqueeUp);
  }

  function onMarqueeMove(e) {
    if (!marqueeStart || !bodyEl) return;
    const rect = bodyEl.getBoundingClientRect();
    const x = e.clientX - rect.left + bodyEl.scrollLeft;
    const y = e.clientY - rect.top + bodyEl.scrollTop;
    const left = Math.min(marqueeStart.x, x);
    const topPx = Math.min(marqueeStart.y, y);
    marquee = { x: left, y: topPx, w: Math.abs(x - marqueeStart.x), h: Math.abs(y - marqueeStart.y) };

    // 사각형과 세로로 겹치는 행들을 선택(행은 전체 폭이라 세로 겹침 기준)
    const bottomPx = topPx + marquee.h;
    const first = Math.max(0, Math.floor(topPx / ROW_H));
    const last = Math.min(total - 1, Math.floor(bottomPx / ROW_H));
    const set = new Set(marqueeBase);
    for (let j = first; j <= last && j >= 0; j++) set.add(rows[j].path);
    setSelectionPaths(Array.from(set));
  }

  function onMarqueeUp() {
    marquee = null;
    marqueeStart = null;
    marqueeBase = [];
    window.removeEventListener("mousemove", onMarqueeMove);
    window.removeEventListener("mouseup", onMarqueeUp);
  }

  onDestroy(() => {
    // 마퀴/컬럼 리사이즈 도중 언마운트되면 전역 리스너 정리
    window.removeEventListener("mousemove", onMarqueeMove);
    window.removeEventListener("mouseup", onMarqueeUp);
    window.removeEventListener("mousemove", onResizeMove);
    window.removeEventListener("mouseup", endResize);
  });

  function onRowDblClick(row) {
    // 폴더는 진입, 파일은 %TEMP% 에 풀어 연다(압축 파일이면 이 프로그램에서, 그 외는 기본 프로그램)
    if (row.isDir) enterFolder(row.path);
    else openInnerEntry(row.path);
  }

  // 파일 목록 컨텍스트 메뉴 항목 생성(행 우클릭, 빈 영역 공용)
  // sel: 이 메뉴가 대상으로 삼는 선택 경로 집합(빈 영역이면 빈 Set)
  function buildMenu(sel) {
    const tr = $t;
    const selArr = Array.from(sel);
    const hasSel = selArr.length > 0;
    // 파일 실행: 정확히 하나의 '파일'(폴더 아님)이 선택됐을 때만 활성
    let runPath = null;
    if (selArr.length === 1) {
      const r = rows.find((x) => x.path === selArr[0]);
      if (r && !r.isDir) runPath = r.path;
    }
    return [
      { label: tr("ctx.extract"), onSelect: openExtract },
      { label: tr("ctx.extractSelected"), onSelect: extractSelectedQuick, enabled: hasSel },
      { label: tr("ctx.extractAllHere"), onSelect: extractAllToArchiveFolder },
      { separator: true },
      { label: tr("ctx.runFile"), onSelect: () => openInnerEntry(runPath), enabled: !!runPath },
      { label: tr("ctx.addFiles"), onSelect: addFilesToArchive, enabled: $canEditArchive },
      {
        label: tr("ctx.deleteSelected"),
        onSelect: removeSelectedFromArchive,
        enabled: $canEditArchive && hasSel,
      },
      { label: tr("ctx.selectAll"), onSelect: selectAllVisible, enabled: total > 0 },
    ];
  }

  // 행 우클릭 → 그 행을 선택(미선택 시)하고 컨텍스트 메뉴를 띄운다
  function onRowContextMenu(e, row) {
    e.preventDefault();
    e.stopPropagation();
    if (!$selectedPaths.has(row.path)) setSelection(row.path);
    // setSelection 직후 $selectedPaths 는 미갱신 → 대상 선택을 직접 판정
    const sel = $selectedPaths.has(row.path) ? $selectedPaths : new Set([row.path]);
    showContextMenu(buildMenu(sel), e.clientX, e.clientY);
  }

  // 빈 영역 우클릭 → 선택 해제, 선택 의존 항목은 비활성 상태로 병기
  function onBodyContextMenu(e) {
    if (!$archivePath) return; // 열린 아카이브가 없으면 메뉴 없음
    e.preventDefault();
    setSelectionPaths([]); // 빈 영역이므로 선택 해제
    showContextMenu(buildMenu(new Set()), e.clientX, e.clientY);
  }

  // 행에서 드래그 시작 → WebView 자체 드래그를 취소하고 네이티브 가상 파일 드래그(지연 렌더링) 시작
  function onRowDragStart(e, row) {
    e.preventDefault(); // HTML5 드래그를 막고(파일 드롭 불가) 네이티브 DoDragDrop 으로 대체
    pendingSinglePath = null; // 드래그 시작 → 다중 선택을 유지(mouseup 축소 취소)
    // 대상: 선택에 포함된 행들(이 행이 선택에 없으면 이 행 하나)
    let targets = rows.filter((r) => $selectedPaths.has(r.path));
    if (!targets.some((r) => r.path === row.path)) targets = [row];
    const innerPaths = targets.map((r) => r.path);
    beginShellDrag($archivePath, innerPaths).catch((err) =>
      console.error("shell drag 실패:", err),
    );
  }

  // 가상 스크롤이라 대상 행이 화면 밖일 수 있음 → 컨테이너 스크롤 조정
  function ensureVisible(idx) {
    if (!bodyEl) return;
    const top = idx * ROW_H;
    const bottom = top + ROW_H;
    if (top < bodyEl.scrollTop) {
      bodyEl.scrollTop = top;
    } else if (bottom > bodyEl.scrollTop + viewportH) {
      bodyEl.scrollTop = bottom - viewportH;
    }
  }

  // 인덱스 이동 = 그 행 하나만 선택 + 화면에 보이도록 스크롤
  function moveTo(idx) {
    if (idx < 0 || idx >= total) return;
    setSelection(rows[idx].path);
    anchorIndex = idx; // 이후 Shift+클릭 범위 기준을 현재 위치로
    ensureVisible(idx);
  }

  // 목록 전체(뷰포트)에서의 키보드 조작: ↑/↓ 이동, Home/End, Enter(폴더 진입)
  function onListKeydown(e) {
    if (total === 0) return;
    const idx = activeIndex;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      moveTo(idx < 0 ? 0 : Math.min(total - 1, idx + 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      moveTo(idx < 0 ? 0 : Math.max(0, idx - 1));
    } else if (e.key === "Home") {
      e.preventDefault();
      moveTo(0);
    } else if (e.key === "End") {
      e.preventDefault();
      moveTo(total - 1);
    } else if (e.key === "Enter") {
      e.preventDefault();
      const row = rows[idx];
      // Enter = 더블클릭과 동일: 폴더 진입 / 파일 실행
      if (row && row.isDir) enterFolder(row.path);
      else if (row) openInnerEntry(row.path);
    }
  }

  // 종류(kind) 라벨 = 폴더 / 확장자 대문자 / 파일
  // 번역 함수(tr)를 인자로 받아 locale 변경 추적
  function kindOf(row, tr) {
    if (row.isDir) return tr("filetable.kindFolder");
    const dot = row.name.lastIndexOf(".");
    if (dot > 0 && dot < row.name.length - 1) {
      return row.name.slice(dot + 1).toUpperCase();
    }
    return tr("filetable.kindFile");
  }
</script>

<div class="table" data-ui="file-table" style="--cols: {colsCss}; --rowmin: {rowMinW}px;">
  <!-- 헤더 (세로 고정, 가로는 본문 scrollLeft 를 따라간다). 정렬 표시는 탐색기처럼 셀 상단 중앙에 작게.
       컬럼 사이 연한 구분선(.divide) + 오른쪽 가장자리 리사이즈 손잡이(.col-resizer). -->
  <div class="head" bind:this={headEl}>
    <div class="row header">
      <button class="cell name sortable divide" on:click={() => setSort("name")}>
        {#if $sortKey === "name"}<span class="sort-ind" class:up={$sortDir === "asc"} class:down={$sortDir !== "asc"}></span>{/if}
        {$t("filetable.name")}
        <!-- svelte-ignore a11y-no-static-element-interactions a11y-click-events-have-key-events -->
        <span class="col-resizer" on:mousedown={(e) => startResize(e, "name")} on:click|stopPropagation></span>
      </button>
      <button class="cell num sortable divide" on:click={() => setSort("size")}>
        {#if $sortKey === "size"}<span class="sort-ind" class:up={$sortDir === "asc"} class:down={$sortDir !== "asc"}></span>{/if}
        {$t("filetable.size")}
        <!-- svelte-ignore a11y-no-static-element-interactions a11y-click-events-have-key-events -->
        <span class="col-resizer" on:mousedown={(e) => startResize(e, "size")} on:click|stopPropagation></span>
      </button>
      <div class="cell num divide">
        {$t("filetable.packedSize")}
        <!-- svelte-ignore a11y-no-static-element-interactions -->
        <span class="col-resizer" on:mousedown={(e) => startResize(e, "packed")}></span>
      </div>
      <div class="cell kind divide">
        {$t("filetable.kind")}
        <!-- svelte-ignore a11y-no-static-element-interactions -->
        <span class="col-resizer" on:mousedown={(e) => startResize(e, "kind")}></span>
      </div>
      <button class="cell date sortable" on:click={() => setSort("date")}>
        {#if $sortKey === "date"}<span class="sort-ind" class:up={$sortDir === "asc"} class:down={$sortDir !== "asc"}></span>{/if}
        {$t("filetable.modified")}
      </button>
    </div>
  </div>

  <!-- 본문 (가상 스크롤) — 뷰포트가 키보드 포커스를 받아 ↑/↓ 이동을 처리한다. -->
  <!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
  <div
    class="body"
    on:scroll={onScroll}
    on:mousedown={onBodyMouseDown}
    on:contextmenu={onBodyContextMenu}
    bind:clientHeight={viewportH}
    bind:this={bodyEl}
    tabindex="0"
    role="listbox"
    aria-label={$t("filetable.ariaLabel")}
    on:keydown={onListKeydown}
  >
    {#if total === 0}
      <div class="empty-folder">{$t("filetable.emptyFolder")}</div>
    {:else}
      <div class="spacer" style="height: {total * ROW_H}px;">
        {#if marquee}
          <div
            class="marquee"
            style="left:{marquee.x}px; top:{marquee.y}px; width:{marquee.w}px; height:{marquee.h}px;"
          ></div>
        {/if}
        {#each slice as row, i (row.path)}
          <!-- 키보드 조작은 뷰포트(.body)가 담당하므로 행 자체엔 키 핸들러를 두지 않는다. -->
          <!-- svelte-ignore a11y-interactive-supports-focus a11y-click-events-have-key-events -->
          <div
            class="row data"
            class:selected={$selectedPaths.has(row.path)}
            class:dir={row.isDir}
            style="top: {(start + i) * ROW_H}px; height: {ROW_H}px;"
            role="option"
            aria-selected={$selectedPaths.has(row.path)}
            draggable="true"
            on:mousedown={(e) => onRowMouseDown(e, row, start + i)}
            on:mouseup={onRowMouseUp}
            on:dblclick={() => onRowDblClick(row)}
            on:dragstart={(e) => onRowDragStart(e, row)}
            on:contextmenu={(e) => onRowContextMenu(e, row)}
          >
            <div class="cell name" title={row.name}>
              <FileIcon name={row.name} isDir={row.isDir} />
              <span class="label">{row.name}</span>
            </div>
            <div class="cell num">{row.isDir ? "—" : formatSize(row.size)}</div>
            <div class="cell num">{row.isDir ? "—" : formatSize(row.packedSize)}</div>
            <div class="cell kind">{kindOf(row, $t)}</div>
            <div class="cell date">{formatModified(row.modified)}</div>
          </div>
        {/each}
      </div>
    {/if}
  </div>
</div>

<style>
  .table {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    font-size: 13px;
  }
  .row {
    display: grid;
    /* 컬럼 너비 = 헤더 리사이즈로 조절, .table 의 --cols 변수로 주입 */
    grid-template-columns: var(--cols, minmax(160px, 1fr) 110px 110px 90px 170px);
    align-items: center;
    /* 컬럼 합계보다 좁아지지 않는다 — 좁으면 컬럼을 줄이는 대신 본문이 가로 스크롤 */
    min-width: var(--rowmin, 0);
  }
  /* 헤더 클리핑 래퍼 — 자체 스크롤바 없이 onScroll 이 scrollLeft 만 맞춘다
     padding-top = 정렬 셰브론이 셀 위로 삐져나오는 만큼의 여유, 없으면 위 끝이 잘린다 */
  .head {
    flex: none;
    overflow: hidden;
    padding-top: 2px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
  }
  .body {
    flex: 1;
    overflow: auto;
    scrollbar-gutter: stable; /* 세로 스크롤바 유무로 컬럼 폭이 흔들리지 않게 */
    min-height: 0;
    position: relative;
  }
  .body:focus {
    outline: none; /* 목록 전체 테두리 대신 선택 행 강조로 커서를 표현한다. */
  }
  .spacer {
    position: relative;
    width: 100%;
    /* 행(.data)의 left/right 기준 — 가로 스크롤 폭 전체를 선택/호버 배경이 덮게 */
    min-width: var(--rowmin, 0);
  }
  /* 마퀴(고무줄) 선택 사각형, 마우스 이벤트는 통과시킨다, */
  .marquee {
    position: absolute;
    z-index: 2;
    pointer-events: none;
    border: 1px solid var(--accent);
    background: color-mix(in srgb, var(--accent) 15%, transparent);
  }
  .data {
    position: absolute;
    left: 0;
    right: 0;
    border-bottom: 1px solid var(--border);
    cursor: default;
    user-select: none;
  }
  .data:hover {
    background: var(--btn-bg);
  }
  .data.selected {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .cell {
    padding: 0 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    line-height: 28px;
  }
  /* 이름 외 부가 정보(크기, 압축크기, 종류, 수정일)는 같은 흐린 색, 헤더는 아래에서 되돌린다 */
  .cell.num,
  .cell.kind,
  .cell.date {
    color: var(--text-muted);
  }
  .cell.num {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .cell.name {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .cell.name .label {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  /* 헤더 셀 색상 통일(크기/수정일이 .cell.num/.cell.date 의 흐린 색을
     상속해 이름/종류와 달라 보이던 것을 덮어쓴다). 굵기/크기는 목록과 동일하게 유지.
     정렬 표시(sort-ind)를 상단에 얹기 위해 position:relative 를 준다. */
  /* 헤더 컬럼 사이 세로 구분선(마지막 '수정일' 제외)
     셀(버튼/div)마다 실제 높이가 달라도 동일하게 보이도록 고정 높이·중앙 정렬로 그린다. */
  .header .cell.divide::before {
    content: "";
    position: absolute;
    top: 50%;
    transform: translateY(-50%);
    right: 0;
    width: 1px;
    height: 16px;
    background: var(--border);
    pointer-events: none;
  }
  /* 컬럼 리사이즈 손잡이 — 헤더 셀 오른쪽 가장자리, */
  .col-resizer {
    position: absolute;
    top: 0;
    right: -3px;
    width: 7px;
    height: 100%;
    cursor: col-resize;
    z-index: 3;
  }
  .col-resizer:hover {
    background: color-mix(in srgb, var(--accent) 45%, transparent);
  }
  .header .cell {
    color: var(--text);
    position: relative;
    /* 정렬 셰브론(회전 시 위로 ~1px 삐져나옴)이 top:0 에서 잘리지 않도록 헤더 셀은
       클리핑하지 않는다. 헤더 라벨은 짧아 말줄임(.cell 의 overflow:hidden)이 불필요. */
    overflow: visible;
  }
  /* 탐색기식 정렬 표시 — 셀 상단 중앙에 얇은 셰브론(∧/∨), CSS 보더로 구현
     글꼴 삼각형(▲▼)보다 훨씬 작고 얇으며 크기를 픽셀로 정확히 제어한다. */
  .sort-ind {
    position: absolute;
    top: 0;
    left: 50%;
    width: 5px;
    height: 5px;
    border: solid var(--text-muted);
    border-width: 0 1.2px 1.2px 0; /* 우+하 보더 → 회전해 셰브론 */
    pointer-events: none;
  }
  .sort-ind.up {
    transform: translateX(-50%) rotate(-135deg); /* 위쪽(오름차순) */
  }
  .sort-ind.down {
    transform: translateX(-50%) rotate(45deg); /* 아래쪽(내림차순) */
  }
  button.cell {
    border: none;
    background: transparent;
    font: inherit;
    text-align: inherit;
    cursor: pointer;
  }
  button.cell.num {
    text-align: right;
  }
  .sortable:hover {
    color: var(--accent);
  }
  .empty-folder {
    padding: 24px;
    text-align: center;
    color: var(--text-muted);
  }
</style>
