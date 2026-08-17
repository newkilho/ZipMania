<script>
  // 주소줄 — "아카이브파일명 > 내부 > 경로", 각 조각 클릭으로 상위 이동
  import { archiveName, archivePath, currentPath, flatView, navigateTo } from "../lib/stores.js";
  import { t } from "../lib/i18n.js";

  // 현재 내부 경로를 조각 배열로, "" 면 빈 배열(루트)
  $: segments = $currentPath ? $currentPath.split("/") : [];

  // i 번째 조각까지의 경로로 이동
  function goTo(index) {
    navigateTo(segments.slice(0, index + 1).join("/"));
  }
</script>

<div class="breadcrumb" data-ui="breadcrumb">
  {#if $archivePath}
    <button class="crumb root" on:click={() => navigateTo("")} title={$t("breadcrumb.rootTitle")}>
      📦 {$archiveName}
    </button>
    {#if !$flatView}
      {#each segments as seg, i}
        <span class="sep">›</span>
        <button class="crumb" on:click={() => goTo(i)}>{seg}</button>
      {/each}
    {:else}
      <span class="sep">›</span>
      <span class="flat-label">{$t("breadcrumb.flat")}</span>
    {/if}
  {:else}
    <span class="muted">{$t("breadcrumb.none")}</span>
  {/if}
</div>

<style>
  .breadcrumb {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 6px 12px;
    border-bottom: 1px solid var(--border);
    font-size: 13px;
    background: var(--surface);
    overflow-x: auto;
    white-space: nowrap;
  }
  .crumb {
    border: none;
    background: transparent;
    color: var(--text);
    cursor: pointer;
    padding: 2px 6px;
    border-radius: 4px;
    font-size: 13px;
  }
  .crumb:hover {
    background: var(--btn-bg);
    color: var(--accent);
  }
  .crumb.root {
    font-weight: 600;
  }
  .sep {
    color: var(--text-muted);
  }
  .flat-label {
    color: var(--text-muted);
    padding: 2px 6px;
  }
  .muted {
    color: var(--text-muted);
  }
</style>
