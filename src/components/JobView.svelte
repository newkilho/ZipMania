<script>
  // 작업 진행/완료 화면 — 압축 창과 해제 창이 같은 것을 쓴다(사본 금지)
  // 두 국면의 골격 동일 — 상태 줄 + 진행률 바 + 정보 줄 + 작업 로그, 값만 교체
  // 로그: 위쪽 log = 쌓이는 줄(요약, 배치 항목, 실패), 아래 current = 계속 덮어쓰는 한 줄
  // 버튼은 창마다 다르므로 slot, 진행 중에만 보일 것은 options slot
  import { afterUpdate } from "svelte";
  import { t } from "../lib/i18n.js";
  import { formatSize } from "../lib/format.js";
  import { etaSec, formatDuration, formatSpeed } from "../lib/progress.js";

  /** 화면에 남기는 로그 줄 상한, 배치 길이만큼 DOM 이 늘지 않게 */
  const LOG_CAP = 200;

  /** @type {"running"|"done"} */
  export let phase = "running";
  /** @type {string} 진행 중 제목 */
  export let title = "";
  /** @type {string} 제목 옆 보조 표기(배치 n/m), 빈 문자열이면 미표시 */
  export let badge = "";
  export let percent = 0;
  export let done = 0;
  export let total = 0;
  /** @type {{bps:number,startMs:number}|null} 속도 측정기 */
  export let meter = null;
  export let elapsedSec = 0;
  /** @type {string[]} 쌓이는 로그 줄, 추가만 한다 */
  export let log = [];
  /** @type {string} 마지막 줄, 진행은 처리 중 파일, 완료는 산출 위치 */
  export let current = "";
  /** @type {number} 실패 개수(빠진 항목), 0 이면 미표시 */
  export let failed = 0;
  /** @type {{status:string,message:string}|null} */
  export let result = null;

  let box;
  let stick = true;
  let lastLen = 0;

  $: speedText = formatSpeed(meter ? meter.bps : 0);
  $: etaText = formatDuration(etaSec(meter, done, total));
  $: elapsedText = formatDuration(elapsedSec);
  // 시간 표기 한 덩어리 — 진행은 경과와 남음, 완료는 경과만
  $: timesText =
    phase === "running"
      ? `${$t("progress.elapsed")} ${elapsedText} · ${$t("progress.remaining")} ${etaText}`
      : `${$t("progress.elapsed")} ${elapsedText}`;
  $: shown = log.length > LOG_CAP ? log.slice(-LOG_CAP) : log;

  // 바닥에 붙어 있을 때만 따라간다, 위로 올려 읽는 중이면 건드리지 않는다
  function onScroll() {
    if (!box) return;
    stick = box.scrollHeight - box.scrollTop - box.clientHeight < 8;
  }

  // 줄이 늘었을 때만 스크롤 — 덮어쓰는 줄은 높이가 변하지 않는다
  afterUpdate(() => {
    if (!box || shown.length === lastLen) return;
    lastLen = shown.length;
    if (stick) box.scrollTop = box.scrollHeight;
  });

  $: resultCls =
    result == null
      ? ""
      : result.status === "ok"
        ? "ok"
        : result.status === "warning"
          ? "warn"
          : result.status === "canceled"
            ? "cancel"
            : "err";
  $: resultIcon =
    result == null ? "" : result.status === "ok" ? "✓" : result.status === "canceled" ? "⨯" : "⚠";
</script>

<div class="job-view" data-ui="job-view">
  <!-- 상태 줄 — 진행은 제목, 완료는 결과 문장, 같은 자리/같은 높이 -->
  {#if phase === "running"}
    <div class="result busy" data-ui="job-title">
      <span class="r-ic spin" aria-hidden="true"></span>
      <span class="r-msg">{title}</span>
      {#if badge}<span class="batch-of">{badge}</span>{/if}
    </div>
  {:else}
    <div class="result {resultCls}" data-ui="job-result">
      <span class="r-ic">{resultIcon}</span>
      <span class="r-msg">{result ? result.message : ""}</span>
    </div>
  {/if}
  <div class="bar" data-ui="progress-bar">
    <div class="fill {resultCls}" data-ui="progress-fill" style="width: {percent}%"></div>
  </div>
  <!-- 정보 줄 — 왼쪽 처리량, 오른쪽 속도/시간/실패 -->
  <div class="job-meta" data-ui="job-meta">
    <span class="pct">{percent}%</span>
    <span class="bytes">{formatSize(done)} / {total ? formatSize(total) : "—"}</span>
    {#if phase === "running"}<span class="speed">{speedText}</span>{/if}
    <span class="times">{timesText}</span>
    {#if failed > 0}<span class="failed">{$t("job.failedCount", { count: failed })}</span>{/if}
  </div>
  <!-- 작업 로그 — 쌓이는 줄 + 계속 덮어쓰는 마지막 줄 -->
  <div class="job-log" data-ui="job-log" bind:this={box} on:scroll={onScroll}>
    {#each shown as line}
      <div class="log-line">{line}</div>
    {/each}
    <div class="log-line now" title={current}>{current || $t("progress.preparing")}</div>
  </div>

  <slot name="notice" />

  {#if phase === "running"}
    <slot name="options" />
  {/if}

  <div class="job-actions" data-ui="job-actions">
    <slot name="actions" />
  </div>
</div>

<style>
  /* 한 열 — 상태 줄, 바, 정보, 로그, 옵션, 버튼이 모두 같은 좌우 끝 */
  .job-view {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 10px;
    padding: 16px 18px;
    text-align: left;
    overflow: hidden;
  }
  .batch-of {
    margin-left: auto;
    padding-left: 10px;
    font-size: 12px;
    color: var(--text-muted);
    white-space: nowrap;
  }
  .bar {
    height: 8px;
    border-radius: 999px;
    background: var(--border);
    overflow: hidden;
  }
  .fill {
    height: 100%;
    border-radius: inherit;
    background: var(--accent);
    transition: width 0.2s ease;
  }
  /* 마감 색 = 상태 줄과 같은 계열, 완료 화면에서도 바가 같은 자리에 남는다 */
  .fill.warn {
    background: color-mix(in srgb, var(--warn-text, #8a5a12) 55%, var(--warn-bg, #fdf3e0));
  }
  .fill.cancel {
    background: var(--text-muted);
  }
  .fill.err {
    background: color-mix(in srgb, var(--alert-text, #9b1c1c) 55%, var(--alert-bg, #fde8e8));
  }
  .job-meta {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: 4px 10px;
    font-size: 12px;
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
  /* 처리량까지가 왼쪽, 속도/시간/실패는 오른쪽 끝 */
  .job-meta .bytes {
    margin-right: auto;
  }
  .job-meta .pct {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
  }
  .job-meta .failed {
    color: var(--alert-text, #9b1c1c);
  }
  /* 작업 로그 — 남은 높이를 다 쓴다, 진행/완료가 같은 높이 */
  .job-log {
    flex: 1 1 auto;
    min-height: 90px;
    overflow-y: auto;
    overflow-x: hidden;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--surface);
    color: var(--text-muted);
    font-size: 12px;
    line-height: 1.6;
    user-select: text;
  }
  .log-line {
    white-space: pre-wrap;
    word-break: break-all;
  }
  /* 덮어쓰는 줄 = 지금 처리 중, 한 줄 말줄임 */
  .log-line.now {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
  }
  .result {
    display: flex;
    align-items: center;
    gap: 9px;
    font-size: 13px;
    padding: 11px 14px;
    border-radius: 8px;
    min-height: 42px;
  }
  .result .r-ic {
    flex: none;
    width: 16px;
    text-align: center;
    font-size: 14px;
    line-height: 1;
  }
  .result .r-msg {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* 진행 표시 = 회전 링, 같은 자리의 결과 아이콘과 크기 동일 */
  .r-ic.spin {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    border: 2px solid var(--border);
    border-top-color: var(--accent);
    animation: job-spin 0.8s linear infinite;
  }
  @keyframes job-spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .r-ic.spin {
      animation: none;
    }
  }
  .result.busy {
    background: var(--surface);
    color: var(--text);
  }
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
    justify-content: flex-end;
    gap: 8px;
  }
</style>
