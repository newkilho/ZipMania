<script>
  // 바이러스 검사(AMSI) 결과 다이얼로그 — 진행률 + 파일별 검사 상태 표 + 요약, scanState(store)로
  // 구동, running 단계 = 진행률 바, done 단계 = 결과 표
  import { scanState, closeScan } from "../../lib/stores.js";
  import { formatSize } from "../../lib/format.js";
  import { t } from "../../lib/i18n.js";

  // 검사 결과 = 파일만(백엔드가 폴더 제외)
  $: files = $scanState.entries || [];
  $: total = files.length;
  $: cleanCount = files.filter((e) => e.status === "clean").length;
  $: malwareCount = files.filter((e) => e.status === "malware").length;
  $: skippedCount = files.filter((e) => e.status === "skipped").length;
  $: errorCount = files.filter((e) => e.status === "error").length;
  // 미검사 항목(크기 초과 건너뜀 + 오류), 존재 시 안전 표기 불가
  $: unscannedCount = skippedCount + errorCount;

  // 결과는 셋 중 하나다, "위협 없음"과 "검사 불완전"을 합치지 않는다 — 검사하지 못한 파일이 있는데
  // 안전 표기 = 없는 확신 제공, threat 위협 발견, incomplete 위협은 없었지만
  // 검사하지 못한 항목이 있음 "clean" 전부 검사했고 위협 없음
  $: verdict = $scanState.error
    ? "threat" // 검사 자체 실패 → 안전 표기 금지(메시지는 error 그대로)
    : malwareCount > 0
      ? "threat"
      : unscannedCount > 0
        ? "incomplete"
        : "clean";
  $: done = $scanState.phase === "done";

  function statusLabel(s) {
    if (s === "malware") return $t("scan.statusMalware");
    if (s === "skipped") return $t("scan.statusSkipped");
    if (s === "error") return $t("scan.statusError");
    return $t("scan.statusClean");
  }

  function onKeydown(e) {
    if (e.key === "Escape" || e.key === "Enter") closeScan();
  }
</script>

{#if $scanState.open}
  <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
  <div class="overlay" data-ui="scan-result-dialog" on:click={closeScan}>
    <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
    <div class="card" data-ui="dialog-card" on:click|stopPropagation on:keydown={onKeydown}>
      <div class="head">
        <h2>
          {$t("scan.title")}
          {#if done}
            <span class="suffix" class:err={verdict === "threat"} class:warn={verdict === "incomplete"}>
              - {verdict === "threat"
                ? $t("scan.titleThreat")
                : verdict === "incomplete"
                  ? $t("scan.titleIncomplete")
                  : $t("scan.titleOk")}
            </span>
          {/if}
        </h2>
        <button class="x" on:click={closeScan} title={$t("common.close")}>✕</button>
      </div>

      <p class="msg">
        {#if $scanState.error}
          {$scanState.error}
        {:else if done}
          {verdict === "threat"
            ? $t("scan.threatFound", { count: malwareCount })
            : verdict === "incomplete"
              ? $t("scan.incomplete", { count: unscannedCount })
              : $t("scan.clean")}
        {:else}
          {$t("scan.running")} {$scanState.percent}%
        {/if}
      </p>

      <div class="bar">
        <div
          class="fill"
          class:err={done && verdict === "threat"}
          class:warn={done && verdict === "incomplete"}
          style="width: {done ? 100 : $scanState.percent}%"
        ></div>
      </div>

      {#if !done}
        <div class="cur" title={$scanState.currentFile}>{$scanState.currentFile || ""}</div>
      {:else if !$scanState.error}
        <div class="hint">{$t("scan.skipHint")}</div>
        <div class="table-wrap">
          <table>
            <thead>
              <tr>
                <th class="c-st">{$t("scan.colStatus")}</th>
                <th class="c-name">{$t("scan.colFile")}</th>
                <th class="c-size">{$t("scan.colSize")}</th>
              </tr>
            </thead>
            <tbody>
              {#each files as f (f.path)}
                <tr class:bad={f.status === "malware"}>
                  <td class="c-st st-{f.status}">{statusLabel(f.status)}</td>
                  <td class="c-name" title={f.path}>{f.path}</td>
                  <td class="c-size">{formatSize(f.size)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}

      <div class="foot">
        <div class="summary">
          <span class="chip">{$t("scan.sumTotal", { count: total })}</span>
          <span class="chip ok">{$t("scan.sumClean", { count: cleanCount })}</span>
          <span class="chip err">{$t("scan.sumMalware", { count: malwareCount })}</span>
          <span class="chip" class:warn={skippedCount > 0}>
            {$t("scan.sumSkipped", { count: skippedCount })}
          </span>
          <span class="chip" class:warn={errorCount > 0}>
            {$t("scan.sumError", { count: errorCount })}
          </span>
        </div>
        <div class="spacer"></div>
        <button class="primary" on:click={closeScan}>{$t("common.confirm")}</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: var(--overlay-bg);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 100;
  }
  .card {
    width: 720px;
    max-width: calc(100vw - 32px);
    max-height: calc(100vh - 60px);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 16px 18px;
    box-shadow: var(--dialog-shadow);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  h2 {
    flex: 1;
    margin: 0;
    font-size: 15px;
  }
  .suffix {
    color: var(--ok-text, #1e6b34);
    font-weight: 600;
  }
  .suffix.err {
    color: var(--alert-text, #9b1c1c);
  }
  .suffix.warn {
    color: var(--warn-text, #8a5a12);
  }
  .x {
    border: none;
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
    font-size: 14px;
    padding: 2px 6px;
  }
  .msg {
    margin: 10px 0 8px;
    font-size: 13px;
    color: var(--text);
  }
  .bar {
    height: 16px;
    border-radius: 4px;
    background: var(--border);
    overflow: hidden;
    flex: 0 0 auto;
  }
  .fill {
    height: 100%;
    background: var(--progress-success);
    transition: width 0.2s ease;
  }
  .fill.err {
    background: var(--alert-text, #9b1c1c);
  }
  .fill.warn {
    background: var(--warn-text, #8a5a12);
  }
  .cur {
    margin-top: 8px;
    font-size: 12px;
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-height: 40px;
  }
  .hint {
    margin-top: 8px;
    font-size: 11.5px;
    color: var(--text-muted);
  }
  .table-wrap {
    margin-top: 6px;
    flex: 1 1 auto;
    min-height: 160px;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: 6px;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12.5px;
  }
  th {
    position: sticky;
    top: 0;
    background: var(--surface);
    text-align: left;
    font-weight: 600;
    color: var(--text-muted);
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
    white-space: nowrap;
  }
  td {
    padding: 4px 10px;
    border-bottom: 1px solid var(--border);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 0;
  }
  .c-st {
    width: 64px;
    font-weight: 600;
  }
  .st-clean {
    color: var(--ok-text, #1e6b34);
  }
  .st-malware {
    color: var(--alert-text, #9b1c1c);
  }
  .st-error {
    color: var(--warn-text, #8a5a12);
  }
  .st-skipped {
    color: var(--text-muted);
    font-weight: 400;
  }
  .c-size {
    width: 96px;
    text-align: right;
    font-variant-numeric: tabular-nums;
    color: var(--text-muted);
  }
  tr.bad td {
    background: color-mix(in srgb, var(--alert-text, #9b1c1c) 12%, transparent);
  }
  .foot {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 12px;
  }
  .summary {
    display: flex;
    gap: 8px;
  }
  .chip {
    font-size: 12px;
    padding: 3px 8px;
    border: 1px solid var(--border);
    border-radius: 5px;
    color: var(--text-muted);
  }
  .chip.ok {
    color: var(--ok-text, #1e6b34);
  }
  .chip.err {
    color: var(--alert-text, #9b1c1c);
  }
  .chip.warn {
    color: var(--warn-text, #8a5a12);
  }
  .spacer {
    flex: 1;
  }
  .primary {
    padding: 7px 20px;
    border-radius: 6px;
    border: 1px solid var(--accent);
    background: var(--accent);
    color: var(--accent-contrast);
    cursor: pointer;
    font-size: 13px;
  }
</style>
