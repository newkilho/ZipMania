<script>
  // 무결성 테스트 결과 다이얼로그 — 진행률 + 파일별 CRC 검증 표 + 요약, testState(store)로
  // 구동, running 단계 = 진행률 바, done 단계 = 결과 표
  import { testState, closeTest } from "../../lib/stores.js";
  import { t } from "../../lib/i18n.js";

  // 폴더는 CRC 가 없으므로 표, 집계에서 제외(파일 기준)
  $: files = ($testState.entries || []).filter((e) => !e.isDir);
  $: total = files.length;
  $: okCount = files.filter((e) => e.ok).length;
  $: errCount = total - okCount;
  // 결과가 비면 정상이 아니라 검사한 것이 없음, errCount === 0 만 보면 항목을
  // 하나도 못 읽은 경우까지 OK 로 표시
  $: verdict = $testState.error || errCount > 0 ? "error" : total === 0 ? "empty" : "ok";
  $: allOk = verdict === "ok";
  $: done = $testState.phase === "done";

  /** CRC 숫자를 8자리 대문자 16진수로, 없으면 "—", */
  function crcHex(n) {
    if (n == null) return "—";
    return (n >>> 0).toString(16).toUpperCase().padStart(8, "0");
  }

  function onKeydown(e) {
    if (e.key === "Escape" || e.key === "Enter") closeTest();
  }
</script>

{#if $testState.open}
  <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
  <div class="overlay" data-ui="test-result-dialog" on:click={closeTest}>
    <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
    <div class="card" data-ui="dialog-card" on:click|stopPropagation on:keydown={onKeydown}>
      <div class="head">
        <h2>
          {$t("test.title")}
          {#if done}
            <span class="suffix" class:err={verdict === "error"} class:warn={verdict === "empty"}>
              - {verdict === "error"
                ? $t("test.titleError")
                : verdict === "empty"
                  ? $t("test.titleEmpty")
                  : $t("test.titleOk")}
            </span>
          {/if}
        </h2>
        <button class="x" on:click={closeTest} title={$t("common.close")}>✕</button>
      </div>

      <!-- 상태 메시지 -->
      <p class="msg">
        {#if $testState.error}
          {$testState.error}
        {:else if done}
          {verdict === "error"
            ? $t("test.someBad", { count: errCount })
            : verdict === "empty"
              ? $t("test.empty")
              : $t("test.allOk", { count: total })}
        {:else}
          {$t("test.running")} {$testState.percent}%
        {/if}
      </p>

      <!-- 진행률 바(초록, 오류 시 빨강) -->
      <div class="bar">
        <div
          class="fill"
          class:err={done && verdict === "error"}
          class:warn={done && verdict === "empty"}
          style="width: {done ? 100 : $testState.percent}%"
        ></div>
      </div>

      {#if !done}
        <!-- 진행 중: 현재 파일명 -->
        <div class="cur" title={$testState.currentFile}>{$testState.currentFile || ""}</div>
      {:else if !$testState.error}
        <!-- 완료: 파일별 결과 표 -->
        <div class="table-wrap">
          <table>
            <thead>
              <tr>
                <th class="c-st">{$t("test.colStatus")}</th>
                <th class="c-name">{$t("test.colFile")}</th>
                <th class="c-crc">{$t("test.colExpectedCrc")}</th>
                <th class="c-crc">{$t("test.colActualCrc")}</th>
              </tr>
            </thead>
            <tbody>
              {#each files as f (f.path)}
                <tr class:bad={!f.ok}>
                  <td class="c-st {f.ok ? 'ok' : 'err'}">{f.ok ? $t("test.statusOk") : $t("test.statusError")}</td>
                  <td class="c-name" title={f.path}>{f.path}</td>
                  <td class="c-crc">{crcHex(f.expectedCrc)}</td>
                  <td class="c-crc">{crcHex(f.actualCrc)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}

      <!-- 하단: 요약 + 확인 -->
      <div class="foot">
        <div class="summary">
          <span class="chip">{$t("test.sumTotal", { count: total })}</span>
          <span class="chip ok">{$t("test.sumOk", { count: okCount })}</span>
          <span class="chip err">{$t("test.sumError", { count: errCount })}</span>
        </div>
        <div class="spacer"></div>
        <button class="primary" on:click={closeTest}>{$t("common.confirm")}</button>
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
  .table-wrap {
    margin-top: 10px;
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
    width: 56px;
  }
  .c-st.ok {
    color: var(--ok-text, #1e6b34);
    font-weight: 600;
  }
  .c-st.err {
    color: var(--alert-text, #9b1c1c);
    font-weight: 600;
  }
  .c-crc {
    width: 96px;
    font-variant-numeric: tabular-nums;
    color: var(--text-muted);
    text-align: right;
  }
  tr.bad td {
    background: color-mix(in srgb, var(--alert-text, #9b1c1c) 10%, transparent);
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
