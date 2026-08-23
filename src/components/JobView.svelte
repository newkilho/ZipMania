<script>
  // 작업 진행/완료 화면 — 압축 창과 해제 창이 같은 것을 쓴다(사본 금지)
  // 진행: 제목 + 진행률 바 + 처리량/속도/경과/예상 + 현재 파일
  // 완료: 결과 줄 + 빠진 항목 + 경과
  // 버튼은 창마다 다르므로 slot, 진행 중에만 보일 것은 options slot
  import { t } from "../lib/i18n.js";
  import { formatSize } from "../lib/format.js";
  import { etaSec, formatDuration, formatSpeed } from "../lib/progress.js";

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
  /** @type {string} 현재 처리 중 파일 */
  export let file = "";
  /** @type {{status:string,message:string}|null} */
  export let result = null;
  /** @type {string[]} 빠진/건너뛴 항목, 있을 때만 표시 */
  export let missing = [];

  $: speedText = formatSpeed(meter ? meter.bps : 0);
  $: etaText = formatDuration(etaSec(meter, done, total));
  $: elapsedText = formatDuration(elapsedSec);

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

<div class="job-view">
  {#if phase === "running"}
    <div class="job-title">
      {title}
      {#if badge}<span class="batch-of">{badge}</span>{/if}
    </div>
    <div class="bar"><div class="fill" style="width: {percent}%"></div></div>
    <div class="job-meta">
      <span class="pct">{percent}%</span>
      <span class="bytes">{formatSize(done)} / {total ? formatSize(total) : "—"}</span>
      <span class="speed">{speedText}</span>
      <span class="times">{$t("progress.elapsed")} {elapsedText} · {$t("progress.remaining")} {etaText}</span>
    </div>
    <div class="job-file" title={file}>{file || $t("progress.preparing")}</div>
  {:else}
    <div class="result {resultCls}">
      <span class="r-ic">{resultIcon}</span>
      <span class="r-msg">{result ? result.message : ""}</span>
    </div>
    <div class="job-meta done">
      <span class="times">{$t("progress.elapsed")} {elapsedText}</span>
    </div>
  {/if}

  <!-- 빠진 항목 = 로그 대신 이 자리, 있을 때만 -->
  {#if missing.length > 0}
    <div class="missing">
      {#each missing as line}
        <div class="missing-line">{line}</div>
      {/each}
    </div>
  {/if}

  <slot name="notice" />

  {#if phase === "running"}
    <slot name="options" />
  {/if}

  <div class="job-actions">
    <slot name="actions" />
  </div>
</div>

<style>
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
    overflow: hidden;
  }
  .job-title {
    color: var(--text-muted);
  }
  .batch-of {
    margin-left: 8px;
    font-size: 12px;
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
    flex-wrap: wrap;
    justify-content: center;
    gap: 10px;
    align-items: center;
    font-size: 13px;
    color: var(--text-muted);
    max-width: 100%;
    font-variant-numeric: tabular-nums;
  }
  .job-meta .pct {
    font-weight: 600;
    color: var(--text);
  }
  /* 현재 파일 = 한 줄 말줄임, 긴 경로가 창을 늘리지 않게 */
  .job-file {
    font-size: 12px;
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
  }
  /* 빠진 항목 — 경고일 때만 나오므로 평소에는 자리를 차지하지 않는다 */
  .missing {
    flex: 0 1 auto;
    min-height: 0;
    max-height: 92px;
    overflow-y: auto;
    font-size: 12px;
    line-height: 1.5;
    color: var(--text-muted);
    background: var(--btn-bg);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 6px 10px;
    width: 100%;
    text-align: left;
  }
  .missing-line {
    white-space: pre-wrap;
    word-break: break-all;
  }
  .result {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 13px;
    padding: 14px 20px;
    border-radius: 8px;
    max-width: 100%;
  }
  .result .r-msg {
    overflow: hidden;
    text-overflow: ellipsis;
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
    gap: 8px;
  }
</style>
