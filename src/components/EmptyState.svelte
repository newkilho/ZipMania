<script>
  // 빈 상태 화면 — 아카이브 미열림, [열기] 버튼 동작 + 드래그&드롭 안내
  // 실제 드롭 처리는 App.svelte 의 창 전역 onFileDrop 이 담당하므로,
  // 여기서는 안내만 표시
  import { pickArchiveFile, openCompressWindow } from "../lib/api.js";
  import { openArchiveByPath, loading } from "../lib/stores.js";
  import { t } from "../lib/i18n.js";

  async function onOpen() {
    const path = await pickArchiveFile();
    if (path) await openArchiveByPath(path);
  }

  // 새 압축 창을 연다(파일/폴더 추가는 그 창 안에서), 툴바 [새로 압축]과 동일
  function onNewArchive() {
    openCompressWindow([]);
  }
</script>

<div class="empty-state" data-ui="empty-state">
  <div class="icon">📦</div>
  <p class="title">{$t("empty.title")}</p>
  <p class="hint">{$t("empty.hint")}</p>
  <div class="actions">
    <button class="action" on:click={onOpen} disabled={$loading}>
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M3 7a2 2 0 0 1 2-2h4l2 2h6a2 2 0 0 1 2 2v1H3z" fill="currentColor" opacity="0.45" />
        <path d="M3 9h18l-2 9a2 2 0 0 1-2 1.6H6.9A2 2 0 0 1 5 18z" fill="currentColor" />
      </svg>
      <span>{$t("empty.openArchive")}</span>
    </button>
    <button class="action" on:click={onNewArchive} disabled={$loading}>
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M4 6a2 2 0 0 1 2-2h12a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2z" fill="currentColor" />
        <path d="M12 8v8m-4-4h8" stroke="var(--icon-glyph)" stroke-width="2" stroke-linecap="round" fill="none" />
      </svg>
      <span>{$t("empty.newArchive")}</span>
    </button>
  </div>
</div>

<style>
  .empty-state {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    color: var(--text-muted);
    user-select: none;
  }
  .icon {
    font-size: 64px;
    opacity: 0.7;
  }
  .title {
    font-size: 16px;
    color: var(--text);
    margin: 0;
  }
  .hint {
    font-size: 13px;
    margin: 0;
  }
  .actions {
    margin-top: 10px;
    display: flex;
    gap: 12px;
  }
  .action {
    display: inline-flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    width: 116px;
    padding: 16px 12px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--btn-bg);
    color: var(--text);
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: filter 0.12s ease, background 0.12s ease;
  }
  /* 아이콘만 색을 갖는다(글자는 --text 그대로), 도형은 fill="currentColor" 이므로
     여기서 color 만 지정하면 뒤판(opacity)까지 같은 색조로 따라온다.
     두 버튼은 같은 색이다 — 나란히 놓인 같은 위계의 시작 동작이라 색으로 나눌 이유가 없다. */
  .action svg {
    width: 34px;
    height: 34px;
    flex: 0 0 auto;
    color: var(--icon-archive);
  }
  /* 비활성 시 색이 남아 클릭 가능처럼 보임 → 회색으로 상태 명시 */
  .action:disabled svg {
    color: var(--text-muted);
  }
  .action:hover:not(:disabled) {
    filter: brightness(1.06);
  }
  .action:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
