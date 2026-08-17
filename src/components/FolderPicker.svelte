<script>
  // 인라인 폴더 브라우저 — 주소줄 + 지연 로딩 폴더 트리로 대상 폴더를 한 창 안에서 고른다
  // 선택은 bind:path 로 부모(ExtractWindow)와 공유하고, [새 폴더]는 부모가 두고 revealTo() 로
  // 드러낸다, 백엔드 호출은 api.js 만 경유
  import { onMount } from "svelte";
  import DirTreeNode from "./DirTreeNode.svelte";
  import { t } from "../lib/i18n.js";
  import { listDirChildren } from "../lib/api.js";

  export let path = ""; // 선택된 대상 폴더(부모와 bind)
  export let initialPath = ""; // 마운트 시 드러낼 초기 경로

  let roots = []; // 루트 노드(드라이브 등)
  let revealPath = ""; // 트리가 자동으로 펼쳐 드러낼 경로
  let treeVersion = 0; // 값 변경 시 트리 remount 후 revealPath 로 재전개

  onMount(async () => {
    try {
      roots = await listDirChildren();
    } catch {
      roots = [];
    }
    // 초기 경로를 선택 + 드러냄
    if (initialPath) {
      path = initialPath;
      revealTo(initialPath);
    }
  });

  /** 경로 변경 + 트리를 그 경로까지 전개, 부모(새 폴더 생성 등)에서도 호출 가능한 공개 메서드 */
  export function revealTo(target) {
    path = target;
    revealPath = target;
    treeVersion += 1; // 트리 remount → DirTreeNode 들이 revealPath 로 자동 펼침
  }

  /** 트리 노드 클릭 — 선택만 바꾼다(사용자가 펼쳐둔 트리를 접지 않도록 remount 안 함), */
  function onSelectNode(p) {
    path = p;
  }

  function onAddressKey(e) {
    if (e.key === "Enter") {
      revealTo(e.currentTarget.value.trim());
    }
  }
</script>

<div class="picker" data-ui="folder-picker">
  <!-- 주소줄 -->
  <div class="address">
    <span class="lb">{$t("extract.destFolder")}</span>
    <input
      type="text"
      value={path}
      placeholder={$t("extract.destPlaceholder")}
      on:change={(e) => (path = e.currentTarget.value)}
      on:keydown={onAddressKey}
    />
  </div>

  <!-- 브라우저 본문: 폴더 트리(드라이브 루트부터) -->
  <div class="browser">
    <div class="tree" role="tree" aria-label={$t("folderPicker.treeLabel")}>
      {#key treeVersion}
        {#each roots as r (r.path)}
          <DirTreeNode node={r} selectedPath={path} onSelect={onSelectNode} {revealPath} />
        {/each}
      {/key}
    </div>
  </div>
</div>

<style>
  .picker {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-height: 0;
    flex: 1;
  }
  .address {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .lb {
    flex: 0 0 auto;
    font-size: 12px;
    color: var(--text-muted);
  }
  .address input {
    flex: 1 1 auto;
    min-width: 0;
    padding: 7px 9px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--btn-bg);
    color: var(--text);
    font-size: 13px;
  }
  /* 폼 배경 = 메인과 동일(테마색), 입력부인 트리 영역만 흰색
     좌측 폴더 트리(FolderTree)와 동일하게 흰 배경 + var(--text) 를 쓴다. */
  .browser {
    flex: 1;
    min-height: 0;
    display: flex;
    border: 1px solid var(--border);
    border-radius: 6px;
    overflow: hidden;
    background: var(--tree-bg);
  }
  .tree {
    flex: 1 1 auto;
    min-width: 0;
    overflow: auto;
    padding: 4px 0;
    background: var(--tree-bg);
  }
</style>
