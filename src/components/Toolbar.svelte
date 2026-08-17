<script>
  // 상단 툴바 — 아이콘 위 / 텍스트 아래 형태의 큰 버튼 + 그룹 구분선
  // 동작 항목 = [열기], [풀기], [새로 압축], [파일 추가], [파일 삭제], [오류검사], [보안검사]
  // (파일 추가/삭제 = 아카이브 편집, 7z/zip/tar 만 가능.)
  import { pickArchiveFile, openCompressWindow, openSettingsWindow } from "../lib/api.js";
  import { t } from "../lib/i18n.js";
  import {
    archivePath,
    flatView,
    hasFolders,
    loading,
    jobRunning,
    selectedPaths,
    openArchiveByPath,
    openExtract,
    openTest,
    openScan,
    addFilesToArchive,
    removeSelectedFromArchive,
    toggleFlat,
  } from "../lib/stores.js";

  async function onOpen() {
    const path = await pickArchiveFile();
    if (path) await openArchiveByPath(path);
  }

  function onCompress() {
    // 별도 네이티브 창(새 압축) 열기, 파일/폴더 추가는 그 창 안에서
    openCompressWindow([]);
  }

  // 편집(파일 추가/삭제) 가능 포맷 여부(7z/zip/tar), 아니면 버튼 비활성
  $: editable = (() => {
    const p = $archivePath;
    if (!p) return false;
    const m = String(p).toLowerCase().match(/\.([^.\\/]+)$/);
    return !!m && ["7z", "zip", "tar"].includes(m[1]);
  })();
</script>

<div class="toolbar" data-ui="toolbar">
  <!-- 그룹 1: 아카이브 열기/풀기/만들기 -->
  <div class="group">
    <button class="tbtn" on:click={onOpen} disabled={$loading || $jobRunning} title={$t("toolbar.openTitle")}>
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M3 7a2 2 0 0 1 2-2h4l2 2h6a2 2 0 0 1 2 2v1H3z" fill="currentColor" opacity="0.35" />
        <path d="M3 9h18l-2 9a2 2 0 0 1-2 1.6H6.9A2 2 0 0 1 5 18z" fill="currentColor" />
      </svg>
      <span>{$t("common.open")}</span>
    </button>

    <button
      class="tbtn"
      on:click={openExtract}
      disabled={!$archivePath || $jobRunning}
      title={$t("toolbar.extractTitle")}
    >
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M4 5a2 2 0 0 1 2-2h5l2 2h5a2 2 0 0 1 2 2v3H4z" fill="currentColor" opacity="0.35" />
        <path d="M4 10h16v8a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2z" fill="currentColor" />
        <path d="M12 12v5m0 0 2.2-2.2M12 17l-2.2-2.2" stroke="var(--accent-contrast)" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" fill="none" />
      </svg>
      <span>{$t("toolbar.extract")}</span>
    </button>

    <button class="tbtn" on:click={onCompress} disabled={$jobRunning} title={$t("toolbar.newArchiveTitle")}>
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M4 6a2 2 0 0 1 2-2h12a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2z" fill="currentColor" />
        <path d="M12 8v8m-4-4h8" stroke="var(--accent-contrast)" stroke-width="2" stroke-linecap="round" fill="none" />
      </svg>
      <span>{$t("toolbar.newArchive")}</span>
    </button>
  </div>

  <span class="divider"></span>

  <!-- 그룹 2: 아카이브 편집/검사 -->
  <div class="group">
    <button
      class="tbtn"
      on:click={addFilesToArchive}
      disabled={!editable || $jobRunning}
      title={$t("toolbar.addFileTitle")}
    >
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M6 3h8l4 4v14a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1z" fill="currentColor" opacity="0.55" />
        <path d="M14 3v4h4" fill="currentColor" />
        <path d="M12 11v6m-3-3h6" stroke="var(--accent-contrast)" stroke-width="1.8" stroke-linecap="round" fill="none" />
      </svg>
      <span>{$t("toolbar.addFile")}</span>
    </button>

    <button
      class="tbtn"
      on:click={removeSelectedFromArchive}
      disabled={!editable || $jobRunning || $selectedPaths.size === 0}
      title={$t("toolbar.removeFileTitle")}
    >
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M6 3h8l4 4v14a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1z" fill="currentColor" opacity="0.55" />
        <path d="M14 3v4h4" fill="currentColor" />
        <path d="M9 14h6" stroke="var(--accent-contrast)" stroke-width="1.8" stroke-linecap="round" fill="none" />
      </svg>
      <span>{$t("toolbar.removeFile")}</span>
    </button>

    <button
      class="tbtn"
      on:click={openTest}
      disabled={!$archivePath || $jobRunning}
      title={$t("toolbar.testTitle")}
    >
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M12 3l7 3v5c0 4.4-2.9 8.3-7 9.6C7.9 19.3 5 15.4 5 11V6z" fill="currentColor" />
        <path d="M9 11.5l2.2 2.2L15 10" stroke="var(--accent-contrast)" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" fill="none" />
      </svg>
      <span>{$t("toolbar.test")}</span>
    </button>

    <!-- 바이러스 검사(AMSI) — 압축 내부 파일(10MB 미만)을 검사. -->
    <button
      class="tbtn"
      on:click={openScan}
      disabled={!$archivePath || $jobRunning}
      title={$t("toolbar.virusScanTitle")}
    >
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M12 3l7 3v5c0 4.4-2.9 8.3-7 9.6C7.9 19.3 5 15.4 5 11V6z" fill="currentColor" />
        <circle cx="12" cy="11" r="2.3" fill="none" stroke="var(--accent-contrast)" stroke-width="1.5" />
        <path d="M12 8.7v-2M12 15.3v-2M8.7 11h-2M17.3 11h-2" stroke="var(--accent-contrast)" stroke-width="1.3" stroke-linecap="round" />
      </svg>
      <span>{$t("toolbar.virusScan")}</span>
    </button>
  </div>

  <span class="divider"></span>

  <!-- 그룹 3: 보기 -->
  <div class="group">
    <button
      class="tbtn"
      class:active={$flatView}
      on:click={toggleFlat}
      disabled={!$archivePath || !$hasFolders}
      title={$t("toolbar.flatTitle")}
    >
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M4 6h16M4 12h16M4 18h16" stroke="currentColor" stroke-width="2" stroke-linecap="round" fill="none" />
      </svg>
      <span>{$t("toolbar.flat")}</span>
    </button>
  </div>

  <span class="spacer"></span>

  <div class="group">
    <button class="tbtn" on:click={openSettingsWindow} title={$t("toolbar.settings")}>
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M12 8.5a3.5 3.5 0 1 0 0 7 3.5 3.5 0 0 0 0-7z" fill="none" stroke="currentColor" stroke-width="2" />
        <path d="M19.4 13a7.6 7.6 0 0 0 .06-2l1.6-1.2-1.6-2.8-1.9.7a7.6 7.6 0 0 0-1.7-1l-.3-2H10.4l-.3 2a7.6 7.6 0 0 0-1.7 1l-1.9-.7-1.6 2.8L4.5 11a7.6 7.6 0 0 0 0 2l-1.6 1.2 1.6 2.8 1.9-.7c.5.4 1.1.7 1.7 1l.3 2h3.2l.3-2c.6-.3 1.2-.6 1.7-1l1.9.7 1.6-2.8z" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round" />
      </svg>
      <span>{$t("toolbar.settings")}</span>
    </button>
  </div>
</div>

<style>
  .toolbar {
    display: flex;
    align-items: stretch;
    gap: 2px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
  }
  .group {
    display: flex;
    align-items: stretch;
    gap: 2px;
  }
  .spacer {
    flex: 1;
  }
  .divider {
    width: 1px;
    align-self: stretch;
    margin: 4px 6px;
    background: var(--border);
  }
  .tbtn {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 4px;
    min-width: 58px;
    padding: 6px 10px;
    border: 1px solid transparent;
    border-radius: 6px;
    background: transparent;
    color: var(--text);
    cursor: pointer;
    line-height: 1;
    font-size: 12px;
  }
  .tbtn svg {
    width: 26px;
    height: 26px;
    color: var(--accent);
  }
  .tbtn span {
    white-space: nowrap;
  }
  .tbtn:hover:not(:disabled) {
    background: var(--btn-bg);
    border-color: var(--border);
  }
  .tbtn:active:not(:disabled) {
    background: var(--accent);
    color: var(--accent-contrast);
  }
  .tbtn:active:not(:disabled) svg {
    color: var(--accent-contrast);
  }
  .tbtn:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .tbtn.active,
  .tbtn.active:hover:not(:disabled) {
    background: var(--accent);
    color: var(--accent-contrast);
    border-color: var(--accent);
  }
  .tbtn.active svg {
    color: var(--accent-contrast);
  }
</style>
