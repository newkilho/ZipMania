<script>
  // 진행률, 취소 패널 — 진행 중일 때 퍼센트 바, 현재 파일, 경과 시간, 취소 버튼을 표시하고,
  // 완료 시 결과 메시지(성공/경고/취소/오류)를 인라인 알림으로 표시
  import { onDestroy } from "svelte";
  import { activeJob, jobResult, cancelActiveJob, closeJobResult } from "../lib/stores.js";
  import { t } from "../lib/i18n.js";

  // 경과 시간(초) = 프런트 계산, 진행 중에만 1초 간격 갱신
  let elapsed = 0;
  let timer = null;

  $: if ($activeJob) {
    startTimer();
  } else {
    stopTimer();
  }

  function startTimer() {
    if (timer) return;
    tick();
    timer = setInterval(tick, 1000);
  }
  function stopTimer() {
    if (timer) {
      clearInterval(timer);
      timer = null;
    }
  }
  function tick() {
    const job = $activeJob;
    if (!job) return;
    elapsed = Math.floor((Date.now() - job.startedAt) / 1000);
  }

  function fmtElapsed(sec) {
    const m = Math.floor(sec / 60);
    const s = sec % 60;
    return `${m}:${String(s).padStart(2, "0")}`;
  }

  // 결과 상태별 라벨, 아이콘
  const resultMeta = {
    ok: { icon: "✓", label: "완료", cls: "ok" },
    warning: { icon: "⚠", label: "경고", cls: "warn" },
    canceled: { icon: "⨯", label: "취소됨", cls: "cancel" },
    error: { icon: "⚠", label: "오류", cls: "err" },
  };

  // 결과 토스트 자동 사라짐 — 성공/취소/경고는 잠시 뒤 자동으로 닫는다
  // 오류는 사용자가 확인할 수 있게 자동으로 닫지 않는다(닫기 버튼으로만)
  let dismissTimer = null;
  $: scheduleDismiss($jobResult);
  function scheduleDismiss(result) {
    clearTimeout(dismissTimer);
    if (result && result.status !== "error") {
      dismissTimer = setTimeout(closeJobResult, 2500);
    }
  }

  onDestroy(() => {
    stopTimer();
    clearTimeout(dismissTimer);
  });
</script>

{#if $activeJob}
  <div class="progress-panel" data-ui="progress-panel">
    <div class="top">
      <span class="kind">{$activeJob.kind === "compress" ? $t("progress.compressing") : $t("progress.extracting")}</span>
      <span class="pct">{$activeJob.percent}%</span>
      <span class="filename" title={$activeJob.currentFile}>
        {$activeJob.currentFile || $t("progress.preparing")}
      </span>
      <span class="elapsed">{fmtElapsed(elapsed)}</span>
      <button class="cancel" on:click={cancelActiveJob}>{$t("common.cancel")}</button>
    </div>
    <div class="bar">
      <div class="fill" style="width: {$activeJob.percent}%"></div>
    </div>
  </div>
{/if}

{#if $jobResult}
  {@const meta = resultMeta[$jobResult.status] ?? resultMeta.ok}
  {#if $jobResult.status === "error"}
    <!-- 오류는 사용자가 확인해야 하므로 예전처럼 하단에 고정 표시(자동으로 사라지지 않음). -->
    <div class="result {meta.cls}" data-ui="job-result" role="alert">
      <span class="r-icon">{meta.icon}</span>
      <span class="r-msg">{$jobResult.message}</span>
      <button class="r-close" on:click={closeJobResult} title={$t("common.close")}>✕</button>
    </div>
  {:else}
    <!-- 성공/취소는 화면 중앙 토스트로 잠시 표시 후 자동으로 사라진다. -->
    <div class="toast-layer" data-ui="toast-layer">
      <div class="toast {meta.cls}" data-ui="toast" role="status">
        <span class="r-icon">{meta.icon}</span>
        <span class="r-msg">{$jobResult.message}</span>
        <button class="r-close" on:click={closeJobResult} title={$t("common.close")}>✕</button>
      </div>
    </div>
  {/if}
{/if}

<style>
  .progress-panel {
    padding: 8px 12px;
    border-top: 1px solid var(--border);
    background: var(--surface);
  }
  .top {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-bottom: 6px;
  }
  .kind {
    font-size: 12px;
    font-weight: 600;
    color: var(--accent);
    min-width: 44px;
  }
  .pct {
    font-variant-numeric: tabular-nums;
    font-weight: 600;
    font-size: 13px;
    min-width: 42px;
  }
  .filename {
    flex: 1;
    font-size: 13px;
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .elapsed {
    font-variant-numeric: tabular-nums;
    font-size: 12px;
    color: var(--text-muted);
  }
  .bar {
    height: 8px;
    border-radius: 4px;
    background: var(--border);
    overflow: hidden;
  }
  .fill {
    height: 100%;
    background: var(--accent);
    transition: width 0.2s ease;
  }
  .cancel {
    padding: 5px 12px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--btn-bg);
    color: var(--text);
    cursor: pointer;
    font-size: 12px;
  }

  /* 오류 결과 — 하단 고정 인라인 스트립(예전 방식), 자동으로 사라지지 않는다, */
  .result {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-top: 1px solid var(--border);
    font-size: 13px;
  }
  .result.err {
    background: var(--alert-bg, #fde8e8);
    color: var(--alert-text, #9b1c1c);
  }
  /* 결과 토스트 — 화면 전체를 덮는 투명 레이어의 정중앙에 배치, 레이어 자체는 클릭을
     통과시키고(pointer-events:none), 토스트 카드만 클릭을 받는다(닫기 버튼 동작). */
  .toast-layer {
    position: fixed;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    pointer-events: none;
    z-index: 1000;
  }
  .toast {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 16px;
    border-radius: 10px;
    font-size: 14px;
    max-width: 80vw;
    box-shadow: var(--toast-shadow);
    pointer-events: auto;
    animation: toast-in 0.15s ease-out;
  }
  @keyframes toast-in {
    from {
      opacity: 0;
      transform: translateY(8px) scale(0.97);
    }
    to {
      opacity: 1;
      transform: none;
    }
  }
  .r-icon {
    font-size: 15px;
  }
  .r-msg {
    flex: 1;
  }
  .r-close {
    border: none;
    background: transparent;
    color: inherit;
    cursor: pointer;
    font-size: 13px;
    padding: 2px 6px;
    opacity: 0.75;
  }
  .r-close:hover {
    opacity: 1;
  }
  .toast.ok {
    background: var(--ok-bg, #e6f4ea);
    color: var(--ok-text, #1e6b34);
  }
  .toast.warn {
    background: var(--warn-bg, #fdf3e0);
    color: var(--warn-text, #8a5a12);
  }
  .toast.cancel {
    background: var(--surface);
    color: var(--text-muted);
  }
  .toast.err {
    background: var(--alert-bg, #fde8e8);
    color: var(--alert-text, #9b1c1c);
  }
</style>
