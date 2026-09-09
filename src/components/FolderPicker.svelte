<script>
  // 인라인 폴더 브라우저 — 주소줄 + 지연 로딩 폴더 트리로 대상 폴더를 한 창 안에서 고른다
  // 선택은 bind:path 로 부모(ExtractWindow)와 공유하고, [새 폴더]는 부모가 두고 revealTo() 로
  // 드러낸다, 백엔드 호출은 api.js 만 경유
  import { onMount, tick } from "svelte";
  import DirTreeNode from "./DirTreeNode.svelte";
  import { get } from "svelte/store";
  import { t } from "../lib/i18n.js";
  import FileIcon from "./FileIcon.svelte";
  import { createDirectory, listDirChildren, listQuickAccess } from "../lib/api.js";

  export let path = ""; // 선택된 대상 폴더(부모와 bind)
  export let initialPath = ""; // 마운트 시 드러낼 초기 경로
  /** @type {Array|null} 스킨 미리보기 표본 루트, 있으면 백엔드 미조회 */
  export let previewRoots = null;
  /** @type {Array|null} 스킨 미리보기 표본 바로가기, 있으면 백엔드 미조회 */
  export let previewQuick = null;

  let roots = []; // 루트 노드(드라이브 등)
  let quick = []; // 바로가기(바탕 화면, 문서, 다운로드, 홈, 드라이브)
  let revealPath = ""; // 트리가 자동으로 펼쳐 드러낼 경로
  let treeVersion = 0; // 값 변경 시 트리 remount 후 revealPath 로 재전개

  // ── 트리 우클릭 메뉴 + 새 폴더 ──
  let menuPath = ""; // 우클릭한 폴더, 빈 값 = 메뉴 닫힘
  let menuX = 0;
  let menuY = 0;
  let naming = false; // 이름 입력 단계
  let newName = "";
  let creating = false;
  let menuError = "";

  /** 트리 노드 우클릭 — 그 폴더를 선택하고 커서 자리에 메뉴, */
  function onNodeContext(p, e) {
    path = p;
    menuPath = p;
    naming = false;
    menuError = "";
    const rect = browserEl?.getBoundingClientRect();
    menuX = e.clientX - (rect?.left ?? 0);
    menuY = e.clientY - (rect?.top ?? 0);
  }

  function closeMenu() {
    menuPath = "";
    naming = false;
    newName = "";
    menuError = "";
  }

  /** [새 폴더] — 탐색기와 같은 기본 이름을 채우고 전체 선택 상태로 연다, */
  function startNewFolder() {
    naming = true;
    menuError = "";
    newName = get(t)("folderPicker.newFolder");
    tick().then(() => {
      const el = document.getElementById("new-folder-input");
      el?.focus();
      el?.select();
    });
  }

  async function confirmNewFolder() {
    const name = newName.trim();
    if (!name || creating) return;
    creating = true;
    menuError = "";
    try {
      const created = await createDirectory(menuPath, name);
      closeMenu();
      revealTo(created); // 만든 폴더를 선택 + 트리에 드러낸다
    } catch (err) {
      menuError = (err && err.message) || String(err);
    } finally {
      creating = false;
    }
  }

  // ── 바로가기/트리 사이 크기 조절(스플리터) ──
  const QUICK_MIN = 120; // 좌측(바로가기) 최소 너비
  const TREE_MIN = 220; // 우측(폴더 트리) 최소 너비
  let quickWidth = 160; // 좌측 열 현재 너비(px)
  let browserEl; // 너비 계산 기준 컨테이너
  let treeEl; // 스크롤 기준 트리 영역
  let dragging = false;

  function startDrag(e) {
    e.preventDefault();
    dragging = true;
    window.addEventListener("mousemove", onDrag);
    window.addEventListener("mouseup", endDrag);
  }

  function onDrag(e) {
    if (!browserEl) return;
    const rect = browserEl.getBoundingClientRect();
    let w = e.clientX - rect.left; // 좌측 열 = 컨테이너 좌단 ~ 커서
    const max = rect.width - TREE_MIN; // 우측 최소 너비 보장
    if (w < QUICK_MIN) w = QUICK_MIN;
    if (w > max) w = Math.max(QUICK_MIN, max);
    quickWidth = w;
  }

  function endDrag() {
    dragging = false;
    window.removeEventListener("mousemove", onDrag);
    window.removeEventListener("mouseup", endDrag);
  }

  onMount(async () => {
    if (previewRoots) {
      roots = previewRoots;
      quick = previewQuick ?? [];
      if (initialPath) path = initialPath;
      return;
    }
    try {
      roots = await listDirChildren();
    } catch {
      roots = [];
    }
    try {
      quick = await listQuickAccess();
    } catch {
      quick = [];
    }
    // 초기 경로를 선택 + 드러냄
    if (initialPath) {
      path = initialPath;
      revealTo(initialPath);
    }
  });

  /** 경로 변경 + 트리를 그 경로까지 전개, 부모(새 폴더 생성 등)에서도 호출 가능한 공개 메서드 */
  export function revealTo(target) {
    path = target;
    revealPath = target;
    treeVersion += 1; // 트리 remount → DirTreeNode 들이 revealPath 로 자동 펼침
    scrollSelectedToTop();
  }

  /** 선택 노드를 트리 상단으로, 전개가 비동기 연쇄라 잠시 반복 추적, */
  function scrollSelectedToTop() {
    let tries = 0;
    const tick = () => {
      tries += 1;
      const el = treeEl?.querySelector('[data-ui="directory-tree-node"].active');
      if (el) {
        treeEl.scrollTop += el.getBoundingClientRect().top - treeEl.getBoundingClientRect().top;
      }
      if (tries < 12) setTimeout(tick, 50);
    };
    setTimeout(tick, 0);
  }

  /** 트리 노드 클릭 — 선택만 바꾼다(사용자가 펼쳐둔 트리를 접지 않도록 remount 안 함), */
  function onSelectNode(p) {
    path = p;
  }

  /** 경로 비교용 정규화(구분자 통일 + 끝 슬래시 제거 + 소문자), */
  function norm(p) {
    return String(p ?? "").replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase();
  }

  /** 바로가기 표시 이름 — kind 번역 우선, drive 는 백엔드 이름(반응성 위해 $t 를 인자로), */
  function quickLabel(q, tf) {
    const key = "folderPicker." + q.kind;
    const label = tf(key);
    return label === key ? q.name : label;
  }

  function onAddressKey(e) {
    if (e.key === "Enter") {
      revealTo(e.currentTarget.value.trim());
    }
  }
</script>

<svelte:window
  on:mousedown={(e) => (menuPath && !e.target.closest(".menu") ? closeMenu() : null)}
  on:keydown={(e) => (menuPath && e.key === "Escape" ? closeMenu() : null)}
/>

<div class="picker" data-ui="folder-picker">
  <!-- 주소줄 -->
  <div class="address">
    <span class="lb">{$t("extract.destFolder")}</span>
    <input
      type="text"
      value={path}
      placeholder={$t("extract.destPlaceholder")}
      on:change={(e) => (path = e.currentTarget.value)}
      on:keydown={onAddressKey}
    />
  </div>

  <!-- 브라우저 본문: 좌측 바로가기 + 폴더 트리(드라이브 루트부터) -->
  <div class="browser" class:dragging bind:this={browserEl}>
    <div class="quick" data-ui="quick-access" style="width: {quickWidth}px;">
      {#each quick as q, i (q.kind + q.path)}
        <!-- 알려진 폴더와 드라이브 사이 구분선, 탐색기와 같은 묶음, -->
        {#if q.kind === "drive" && i > 0 && quick[i - 1].kind !== "drive"}
          <div class="qsep"></div>
        {/if}
        <div
          class="qitem"
          class:active={norm(q.path) === norm(path)}
          role="button"
          tabindex="0"
          title={q.path}
          on:click={() => revealTo(q.path)}
          on:keydown={(e) => (e.key === "Enter" ? revealTo(q.path) : null)}
        >
          {#if q.icon}
            <img class="qicon" src={q.icon} width="16" height="16" alt="" draggable="false" />
          {:else}
            <FileIcon name={q.name} isDir={true} />
          {/if}
          <span class="qname">{quickLabel(q, $t)}</span>
        </div>
      {/each}
    </div>
    <!-- 바로가기/트리 사이 크기 조절 손잡이 -->
    <!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
    <div
      class="gutter"
      data-ui="splitter"
      role="separator"
      aria-orientation="vertical"
      title={$t("app.gutterTitle")}
      on:mousedown={startDrag}
    ></div>
    <div class="tree" role="tree" bind:this={treeEl} aria-label={$t("folderPicker.treeLabel")}>
      {#key treeVersion}
        {#each roots as r (r.path)}
          <DirTreeNode node={r} selectedPath={path} onSelect={onSelectNode} onContext={onNodeContext} {revealPath} />
        {/each}
      {/key}
    </div>

    <!-- 우클릭 메뉴, 항목 선택 후 이름 입력까지 한 자리에서 -->
    {#if menuPath}
      <div class="menu" style="left: {menuX}px; top: {menuY}px;" role="menu">
        {#if naming}
          <div class="menu-name">
            <input
              id="new-folder-input"
              type="text"
              bind:value={newName}
              placeholder={$t("folderPicker.newFolderName")}
              on:keydown={(e) => (e.key === "Enter" ? confirmNewFolder() : e.key === "Escape" ? closeMenu() : null)}
            />
            <button class="mbtn" on:click={confirmNewFolder} disabled={!newName.trim() || creating}>
              {$t("common.confirm")}
            </button>
          </div>
          {#if menuError}
            <div class="menu-error" role="alert">⚠ {menuError}</div>
          {/if}
        {:else}
          <button class="menu-item" role="menuitem" on:click={startNewFolder}>
            {$t("folderPicker.newFolderMenu")}
          </button>
        {/if}
      </div>
    {/if}
  </div>
</div>

<style>
  .picker {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-height: 0;
    flex: 1;
  }
  .address {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .lb {
    flex: 0 0 auto;
    font-size: 12px;
    color: var(--text-muted);
  }
  .address input {
    flex: 1 1 auto;
    min-width: 0;
    padding: 7px 9px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--btn-bg);
    color: var(--text);
    font-size: 13px;
  }
  /* 폼 배경 = 메인과 동일(테마색), 입력부인 트리 영역만 흰색
     좌측 폴더 트리(FolderTree)와 동일하게 흰 배경 + var(--text) 를 쓴다. */
  .browser {
    position: relative;
    flex: 1;
    min-height: 0;
    display: flex;
    border: 1px solid var(--border);
    border-radius: 6px;
    overflow: hidden;
    background: var(--tree-bg);
  }
  .quick {
    flex: 0 0 auto;
    min-width: 0;
    overflow: auto;
    padding: 4px 0;
    background: var(--tree-bg);
  }
  /* 글자 크기, 줄 높이는 폴더 트리 노드와 같게, 좌우 여백은 트리의 펼침 화살표 자리를 감안, */
  .qitem {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 4px 12px;
    font-size: 13px;
    line-height: 18px;
    color: var(--text);
    cursor: pointer;
    white-space: nowrap;
    user-select: none;
  }
  .qitem:hover {
    background: var(--btn-bg);
  }
  .qitem.active {
    background: var(--accent);
    color: var(--accent-contrast);
  }
  .qsep {
    height: 1px;
    margin: 4px 12px;
    background: var(--border);
  }
  .qicon {
    width: 16px;
    height: 16px;
    flex: 0 0 16px;
    object-fit: contain;
  }
  .qname {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .menu {
    position: absolute;
    z-index: 10;
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 4px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    box-shadow: var(--dialog-shadow, 0 4px 16px rgba(0, 0, 0, 0.18));
  }
  .menu-item {
    padding: 6px 14px 6px 10px;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: var(--text);
    font-size: 13px;
    text-align: left;
    white-space: nowrap;
    cursor: pointer;
  }
  .menu-item:hover {
    background: var(--btn-bg);
  }
  .menu-name {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .menu-name input {
    width: 160px;
    padding: 5px 8px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--btn-bg);
    color: var(--text);
    font-size: 13px;
  }
  .mbtn {
    padding: 5px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--btn-bg);
    color: var(--text);
    font-size: 13px;
    cursor: pointer;
  }
  .mbtn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .menu-error {
    max-width: 220px;
    padding: 0 4px;
    color: var(--alert-text, #9b1c1c);
    font-size: 12px;
  }
  .gutter {
    flex: 0 0 5px;
    cursor: col-resize;
    background: color-mix(in srgb, var(--border) 40%, transparent);
  }
  .gutter:hover,
  .browser.dragging .gutter {
    background: var(--accent);
  }
  /* 드래그 중 텍스트 선택/커서 흔들림 방지, */
  .browser.dragging {
    cursor: col-resize;
    user-select: none;
  }
  .tree {
    flex: 1 1 auto;
    min-width: 0;
    overflow: auto;
    padding: 4px 0;
    background: var(--tree-bg);
  }
</style>
