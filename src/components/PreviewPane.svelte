<script>
  // 좌측 하단 이미지 미리보기 — 우측 목록에서 이미지 파일을 하나 선택하면
  // 아카이브에서 임시 해제 없이 메모리로 읽어 이 패널에 그대로 표시
  // (반디집의 좌측 하단 썸네일과 같은 자리)
  import { previewImage } from "../lib/stores.js";
  import { t } from "../lib/i18n.js";
</script>

{#if $previewImage}
  <div class="preview" data-ui="preview-pane" title={$previewImage.name}>
    {#if $previewImage.dataUri}
      <img src={$previewImage.dataUri} alt={$previewImage.name} />
    {:else if $previewImage.loading}
      <div class="msg">{$t("preview.loading")}</div>
    {:else if $previewImage.error}
      <div class="msg err">{$previewImage.error}</div>
    {/if}
  </div>
{/if}

<style>
  .preview {
    height: 180px;
    flex-shrink: 0;
    border-top: 1px solid var(--border);
    background: var(--preview-bg);
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
    padding: 6px;
  }
  .preview img {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
    /* 투명 PNG 가 흰 배경 위에서도 보이도록 옅은 체크무늬 대신 단색 배경 사용 */
  }
  .msg {
    font-size: 12px;
    color: var(--text-muted);
    text-align: center;
    padding: 0 8px;
  }
  .msg.err {
    color: var(--alert-text, #9b1c1c);
  }
</style>
