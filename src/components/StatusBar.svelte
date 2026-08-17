<script>
  // 하단 상태줄 — 좌: "N개 항목, 전체 크기, (선택 시) 선택 N개", 우: "압축 <크기>, 압축률 N%"
  // (현재 보기 기준)
  import { onDestroy, onMount } from "svelte";
  import { visibleRows, selectedPaths } from "../lib/stores.js";
  import { formatSize } from "../lib/format.js";
  import { t } from "../lib/i18n.js";
  import { getUpdateNotify, onUpdateNotify, openUpdateUrl } from "../lib/api.js";

  export let preview = false;

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

  // 업데이트 공지 배지, Rust 가 앱 시작 시 서버에 물어보고 공지가 있을 때 한 번 보낸다, 델파이는
  // 메인 폼에 StatusBar 를 직접 만들어 붙였지만, 여기는 상태줄이 이미 항상 있으므로 오른쪽 끝에
  // 배지만 얹는다(파일이 열리지 않은 상태에서도 보인다)
  let notify = null;
  let unlistenNotify = null;

  onMount(async () => {
    if (preview) return;
    // 선구독 — 구독 전 도착분은 아래에서 회수, 이후는 이벤트 수신
    unlistenNotify = await onUpdateNotify((payload) => {
      notify = payload;
    });
    // 화면 표시 전 확인 완료 가능성, 이벤트만 대기하면 그 경우 누락
    if (!notify) notify = await getUpdateNotify();
  });

  onDestroy(() => {
    if (unlistenNotify) unlistenNotify();
  });

  async function openNotify() {
    if (notify?.url) await openUpdateUrl(notify.url);
  }
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
    {#if notify}
      <button class="update" type="button" on:click={openNotify} title={notify.url}>
        {notify.text}
      </button>
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

  /* 업데이트 공지 배지 — (S) 시맨틱 색상 "오류/알림(err)"
     델파이의 빨간 굵은 글씨 상태줄에 대응하되, 기존 정보를 가리지 않도록 배지로 둔다. */
  .update {
    border: 0;
    border-radius: 3px;
    padding: 1px 8px;
    font: inherit;
    font-weight: 600;
    cursor: pointer;
    background: var(--alert-bg);
    color: var(--alert-text);
  }
  .update:hover {
    filter: brightness(0.96);
  }
</style>
