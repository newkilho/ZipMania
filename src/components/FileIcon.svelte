<script>
  // 파일/폴더의 실제 Windows 시스템 아이콘(16x16) 표시
  // 로드 전과 실패 시 이모지(📁/📄) 폴백, 로드 후 <img> 로 교체
  import { getFileIcon } from "../lib/icons.js";

  /** 파일명(확장자 추출용), */
  export let name = "";
  /** 폴더 여부, */
  export let isDir = false;

  /** 로드된 아이콘 데이터 URI, null = 미로드/실패(이모지 폴백), */
  let src = null;

  // name/isDir 변경 시 아이콘 재로드, 비동기 응답의 뒤늦은 도착으로
  // 이전 요청 결과가 덮어쓰는 것을 요청 토큰(reqId)으로 차단
  let reqId = 0;
  $: load(name, isDir);

  function load(n, dir) {
    const my = ++reqId;
    src = null;
    getFileIcon(n, dir).then((uri) => {
      // 최신 요청이 아니면 무시(경합 방지)
      if (my === reqId) src = uri;
    });
  }
</script>

{#if src}
  <img class="icon" {src} width="16" height="16" alt="" draggable="false" />
{:else}
  <span class="icon">{isDir ? "📁" : "📄"}</span>
{/if}

<style>
  .icon {
    /* 이모지와 이미지가 같은 16x16 자리를 차지해 정렬이 흔들리지 않도록 고정, */
    width: 16px;
    height: 16px;
    flex: 0 0 16px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    line-height: 16px;
  }
  img.icon {
    object-fit: contain;
  }
</style>
