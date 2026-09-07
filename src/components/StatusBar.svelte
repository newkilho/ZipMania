<script>
  // 하단 상태줄 — 좌: "N개 항목, 전체 크기, (선택 시) 선택 N개", 우: "압축 <크기>, 압축률 N%"
  // (현재 보기 기준)
  import { visibleRows, selectedPaths } from "../lib/stores.js";
  import { formatSize } from "../lib/format.js";
  import { t } from "../lib/i18n.js";

  $: rows = $visibleRows;
  $: itemCount = rows.length;
  // 전체 크기: 현재 보기의 파일 크기 합 (폴더 제외)
  $: totalSize = rows.reduce((sum, r) => sum + (r.isDir ? 0 : r.size || 0), 0);
  // 압축 크기 합(폴더, 미상 제외)과 압축률(압축 크기 / 원본 크기)
  $: totalPacked = rows.reduce((sum, r) => sum + (r.isDir ? 0 : r.packedSize || 0), 0);
  $: ratio = totalSize > 0 ? Math.round((totalPacked / totalSize) * 100) : 0;

  $: selectedCount = $selectedPaths.size;
  $: selectedSize = rows
    .filter((r) => $selectedPaths.has(r.path) && !r.isDir)
    .reduce((sum, r) => sum + (r.size || 0), 0);
</script>

<div class="status-bar" data-ui="status-bar">
  <span class="left">
    {#if itemCount > 0}
      <span>{$t("status.items", { count: itemCount })}</span>
      <span class="dot">·</span>
      <span>{formatSize(totalSize)}</span>
      {#if selectedCount > 0}
        <span class="dot">·</span>
        <span class="sel">{$t("status.selected", { count: selectedCount, size: formatSize(selectedSize) })}</span>
      {/if}
    {/if}
  </span>
  <span class="right">
    {#if totalPacked > 0}
      <span>{$t("status.packed", { size: formatSize(totalPacked) })}</span>
      <span class="dot">·</span>
      <span>{$t("status.ratio", { pct: ratio })}</span>
    {/if}
  </span>
</div>

<style>
  .status-bar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 4px 12px;
    border-top: 1px solid var(--border);
    background: var(--surface);
    font-size: 12px;
    color: var(--text-muted);
  }
  .left,
  .right {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .dot {
    opacity: 0.5;
  }
  .sel {
    color: var(--accent);
  }
</style>
