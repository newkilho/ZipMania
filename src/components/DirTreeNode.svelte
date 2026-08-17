<script>
  // 파일시스템 폴더 트리의 재귀 노드(<svelte:self>) — 하위 폴더는 전개 시 지연 로딩
  // 행 클릭 = 선택, 화살표와 더블클릭 = 전개, revealPath 하위면 마운트 때 자동 전개
  import { onMount } from "svelte";
  import FileIcon from "./FileIcon.svelte";
  import { listDirChildren } from "../lib/api.js";
  import { t } from "../lib/i18n.js";

  export let node; // { name, path, hasChildren }
  export let selectedPath = ""; // 현재 선택된 대상 폴더
  export let onSelect; // (path) => void
  export let depth = 0;
  export let revealPath = ""; // 마운트 시 이 경로까지 자동으로 펼침(선택 상태 포함)

  let expanded = false;
  let children = null; // null = 아직 로드 안 함
  let loading = false;

  /** 경로 비교용 정규화(구분자 통일 + 끝 슬래시 제거 + 소문자), */
  function norm(p) {
    return String(p ?? "").replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase();
  }
  /** anc 가 desc 의 (엄격한) 상위 폴더인지, */
  function isAncestor(anc, desc) {
    const a = norm(anc);
    const d = norm(desc);
    if (!d || a === d) return false;
    return a === "" || d.startsWith(a + "/");
  }

  $: isActive = norm(node.path) === norm(selectedPath);

  async function loadChildren() {
    if (children) return;
    loading = true;
    try {
      children = await listDirChildren(node.path);
    } catch {
      children = [];
    } finally {
      loading = false;
    }
  }

  async function toggle() {
    if (!node.hasChildren) return;
    if (expanded) {
      expanded = false;
      return;
    }
    await loadChildren();
    expanded = true;
  }

  function selectSelf() {
    if (onSelect) onSelect(node.path);
  }

  onMount(async () => {
    // 대상 경로가 이 노드 하위에 있으면 자동으로 펼쳐 트리에 드러낸다(하위 노드가 연쇄로 처리)
    if (node.hasChildren && revealPath && isAncestor(node.path, revealPath)) {
      await loadChildren();
      expanded = true;
    }
  });
</script>

<div
  class="dnode"
  data-ui="directory-tree-node"
  class:active={isActive}
  style="padding-left: {depth * 14 + 6}px"
  role="treeitem"
  aria-selected={isActive}
  aria-expanded={node.hasChildren ? expanded : undefined}
  tabindex="0"
  on:click={selectSelf}
  on:dblclick={toggle}
  on:keydown={(e) => (e.key === "Enter" ? selectSelf() : null)}
>
  {#if node.hasChildren}
    <button
      class="chev"
      class:loading
      on:click|stopPropagation={toggle}
      title={expanded ? $t("tree.collapse") : $t("tree.expand")}
      tabindex="-1"
    >
      <svg class="chev-svg" class:open={expanded} viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
        <path d="M6 4l4 4-4 4" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" />
      </svg>
    </button>
  {:else}
    <span class="chev-space"></span>
  {/if}
  <FileIcon name={node.name} isDir={true} />
  <span class="name" title={node.name}>{node.name}</span>
</div>

{#if expanded && children}
  {#each children as child (child.path)}
    <svelte:self node={child} depth={depth + 1} {selectedPath} {onSelect} {revealPath} />
  {/each}
{/if}

<style>
  .dnode {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 4px 8px 4px 6px;
    font-size: 13px;
    color: var(--text);
    cursor: pointer;
    white-space: nowrap;
    user-select: none;
  }
  .dnode:hover {
    background: var(--btn-bg);
  }
  .dnode.active {
    background: var(--accent);
    color: var(--accent-contrast);
  }
  .chev {
    border: none;
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
    width: 18px;
    min-width: 18px;
    height: 18px;
    padding: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }
  .chev:hover {
    color: var(--text);
  }
  .dnode.active .chev {
    color: var(--accent-contrast);
  }
  .chev-svg {
    transition: transform 0.12s ease;
  }
  .chev-svg.open {
    transform: rotate(90deg);
  }
  .chev.loading .chev-svg {
    opacity: 0.4;
  }
  .chev-space {
    width: 18px;
    min-width: 18px;
    display: inline-block;
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
