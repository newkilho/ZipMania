<script>
  // 암호 입력 다이얼로그 — "암호 필요" 오류 시 표시
  // 입력 후 재시도, 틀리면 오류 메시지 표시 후 재입력
  import { passwordState, loading, submitPassword, cancelPassword } from "../../lib/stores.js";
  import { t } from "../../lib/i18n.js";

  let password = "";

  async function onSubmit() {
    if (!password) return;
    const pw = password;
    password = "";
    await submitPassword(pw);
  }

  function onCancel() {
    password = "";
    cancelPassword();
  }

  function onKeydown(e) {
    if (e.key === "Enter") onSubmit();
    else if (e.key === "Escape") onCancel();
  }
</script>

{#if $passwordState.open}
  <!-- 배경(백드롭) 클릭으로 닫기. 정적 요소이므로 a11y 경고를 명시적으로 무시한다. -->
  <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
  <div class="overlay" data-ui="password-dialog" on:click={onCancel}>
    <!-- 카드 클릭이 오버레이로 전파되어 닫히지 않도록 -->
    <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
    <div class="card" data-ui="dialog-card" on:click|stopPropagation>
      <h2>{$t("password.title")}</h2>
      <p class="desc">{$t("password.desc")}</p>

      <!-- svelte-ignore a11y-autofocus -->
      <input
        type="password"
        bind:value={password}
        on:keydown={onKeydown}
        placeholder={$t("password.placeholder")}
        autofocus
      />

      {#if $passwordState.error}
        <p class="error">{$passwordState.error}</p>
      {/if}

      <div class="actions">
        <button class="ghost" on:click={onCancel}>{$t("common.cancel")}</button>
        <button class="primary" on:click={onSubmit} disabled={$loading || !password}>
          {$loading ? $t("common.checking") : $t("common.open")}
        </button>
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
    width: 320px;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 20px;
    box-shadow: var(--dialog-shadow);
  }
  h2 {
    margin: 0 0 6px;
    font-size: 16px;
  }
  .desc {
    margin: 0 0 12px;
    font-size: 13px;
    color: var(--text-muted);
  }
  input {
    width: 100%;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--btn-bg);
    color: var(--text);
    font-size: 14px;
  }
  .error {
    margin: 8px 0 0;
    font-size: 12px;
    color: var(--alert-text);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 16px;
  }
  button {
    padding: 7px 16px;
    border-radius: 6px;
    border: 1px solid var(--border);
    cursor: pointer;
    font-size: 13px;
  }
  .ghost {
    background: var(--btn-bg);
    color: var(--text);
  }
  .primary {
    background: var(--accent);
    color: var(--accent-contrast);
    border-color: var(--accent);
  }
  .primary:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
