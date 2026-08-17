<script>
  // 재귀 폴더 트리 노드 — <svelte:self> 로 자기 자신을 렌더링해 하위 폴더 표현
  // 확장 상태(expanded Set), 현재 폴더(currentPath), 콜백은 상위(FolderTree)에서 내려받는다
  export let node; // { name, path, children }
  export let depth = 0;
  export let expanded; // Set<string> — 펼쳐진 폴더 경로들
  export let currentPath = ""; // 현재 선택된 폴더 경로
  export let onNavigate; // (path) => void
  export let onToggle; // (path) => void

  import FileIcon from "./FileIcon.svelte";
  import { t } from "../lib/i18n.js";

  $: hasChildren = node.children && node.children.length > 0;
  $: isOpen = expanded.has(node.path);
  $: isActive = node.path === currentPath;
</script>

<div
  class="node"
  data-ui="archive-tree-node"
  class:active={isActive}
  style="padding-left: {depth * 14 + 6}px"
  on:click={() => onNavigate(node.path)}
  on:keydown={(e) => (e.key === "Enter" ? onNavigate(node.path) : null)}
  role="treeitem"
  aria-selected={isActive}
  aria-expanded={hasChildren ? isOpen : undefined}
  tabindex="0"
>
  {#if hasChildren}
    <button
      class="chev"
      on:click|stopPropagation={() => onToggle(node.path)}
      title={isOpen ? $t("tree.collapse") : $t("tree.expand")}
      tabindex="-1"
    >
      <!-- Windows 탐색기식 얇은 갈매기(chevron): 접힘=오른쪽, 펼침=아래(90° 회전) -->
      <svg
        class="chev-svg"
        class:open={isOpen}
        viewBox="0 0 16 16"
        width="16"
        height="16"
        aria-hidden="true"
      >
        <path
          d="M6 4l4 4-4 4"
          fill="none"
          stroke="currentColor"
          stroke-width="1.6"
          stroke-linecap="round"
          stroke-linejoin="round"
        />
      </svg>
    </button>
  {:else}
    <span class="chev-space"></span>
  {/if}
  {#if depth === 0}
    <!-- 루트: 아카이브 자체 → 아카이브 파일의 시스템 아이콘(.7z/.zip 등) -->
    <FileIcon name={node.name} isDir={false} />
  {:else}
    <!-- 하위 폴더: 시스템 폴더 아이콘 -->
    <FileIcon name={node.name} isDir={true} />
  {/if}
  <span class="name" title={node.name}>{node.name}</span>
</div>

{#if hasChildren && isOpen}
  {#each node.children as child (child.path)}
    <svelte:self
      node={child}
      depth={depth + 1}
      {expanded}
      {currentPath}
      {onNavigate}
      {onToggle}
    />
  {/each}
{/if}

<style>
  .node {
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
  .node:hover {
    background: var(--btn-bg);
  }
  .node.active {
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
  /* 선택(강조) 노드에서 화살표도 흰색으로 보이도록 currentColor 상속 */
  .node.active .chev {
    color: var(--accent-contrast);
  }
  .chev-svg {
    transition: transform 0.12s ease;
  }
  .chev-svg.open {
    transform: rotate(90deg);
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
