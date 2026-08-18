<script>
  // 환경설정 창, 좌측 카테고리 + 우측 콘텐츠 + 하단 [초기화]/[닫기], 즉시 저장 + settings:changed 방송
  // 동작하는 것은 일반, 탐색기 메뉴, 압축 풀기, 나머지는 placeholder
  import { onMount, onDestroy } from "svelte";
  import { get } from "svelte/store";
  import { t } from "../lib/i18n.js";
  import { applyLanguage, LANGUAGES } from "../lib/i18n.js";
  import { applyTheme } from "../lib/theme.js";
  import {
    getSettings,
    saveSettings,
    emitSettingsChanged,
    fileAssocStatus,
    openDefaultApps,
    openDefaultAppPicker,
    finishDefaultAppPicker,
    syncFileAssoc,
    syncShellIntegration,
    closeCurrentWindow,
    setCurrentWindowTitle,
    onWindowFocus,
  } from "../lib/api.js";

  // 좌측 카테고리(첨부 이미지 구성), gap=true 는 위에 간격을 둔다(언어 설정 분리)
  const CATEGORIES = [
    { id: "general", labelKey: "settings.catGeneral" },
    { id: "extract", labelKey: "settings.catExtract" },
    { id: "fileAssoc", labelKey: "settings.catFileAssoc" },
    { id: "shellMenu", labelKey: "settings.catShellMenu" },
  ];
  // 실제 설정 콘텐츠가 있는 카테고리(그 외는 추후 지원 placeholder), 언어는 일반 설정에 포함
  const IMPLEMENTED = new Set(["general", "fileAssoc", "shellMenu", "extract"]);

  /**
   * 파일 연결 노출 확장자, Rust DEFAULT_ASSOC_EXTS 의 유일한 사본, 테스트가 대조, READ_EXTS 부분집합
   */
  const ASSOC_EXTS = ["zip", "7z", "rar", "tar", "gz", "tgz", "bz2", "xz", "egg", "alz", "cbz"];

  const THEMES = [
    { value: "system", labelKey: "settings.themeSystem" },
    { value: "light", labelKey: "settings.themeLight" },
    { value: "dark", labelKey: "settings.themeDark" },
  ];

  let selected = "general";

  // 화면에 보이는 값(= 저장된 값)
  let theme = "system";
  let language = "system";
  let extractCreateSubfolder = true;
  let extractDeleteAfter = false;
  let extractAutoClose = false; // 해제 성공 후 풀기 창 닫기
  let extractOpenFolder = false; // 해제 성공 후 대상 폴더 열기
  let shellIntegration = false; // 탐색기 우클릭 메뉴 통합(셸 확장)
  let fileAssoc = new Set(); // ZipMania 으로 연결할 확장자(점 없는 소문자)
  // 사용자의 연결 목록 직접 지정 이력, 설치 훅의 기본값 덮어쓰기 판단 근거
  let fileAssocInitialized = false;

  let loaded = false;
  let error = "";

  // 창 제목 = 현재 언어, Rust 초기 제목 덮어쓰기, 언어 변경 시 재적용
  $: setCurrentWindowTitle($t("settings.title")).catch(() => {});

  onMount(async () => {
    try {
      apply(await getSettings());
    } catch (e) {
      error = String(e);
    } finally {
      loaded = true;
    }
    refreshAssocStatus();
  });

  /** 설정 객체(snake_case, Rust Settings) → 이 창 상태, */
  function apply(s) {
    theme = s.theme ?? "system";
    language = s.language ?? "system";
    extractCreateSubfolder = s.extract_create_subfolder ?? true;
    extractDeleteAfter = s.extract_delete_after ?? false;
    extractAutoClose = s.extract_auto_close ?? false;
    extractOpenFolder = s.extract_open_folder ?? false;
    shellIntegration = s.shell_integration ?? false;
    // 저장 값에 목록 밖 확장자가 섞여도 화면에는 아는 것만 반영
    fileAssoc = new Set((s.file_assoc ?? []).filter((e) => ASSOC_EXTS.includes(e)));
    fileAssocInitialized = s.file_assoc_initialized ?? false;
  }

  /**
   * 현재 값 저장 + 전 창 방송, 저장 직전 파일 재읽기 후 병합(미편집 필드 보호)
   */
  async function persist() {
    error = "";
    try {
      const settings = {
        ...(await getSettings()),
        theme,
        language,
        extract_create_subfolder: extractCreateSubfolder,
        extract_delete_after: extractDeleteAfter,
        extract_auto_close: extractAutoClose,
        extract_open_folder: extractOpenFolder,
        shell_integration: shellIntegration,
        // 순서를 목록 순으로 고정 — Set 순서로 파일이 매번 달라 보이는 것 방지
        file_assoc: ASSOC_EXTS.filter((e) => fileAssoc.has(e)),
        file_assoc_initialized: fileAssocInitialized,
      };
      await saveSettings(settings);
      await emitSettingsChanged(settings);
    } catch (e) {
      error = String(e);
    }
  }

  // 테마, 언어는 이벤트를 기다리지 않고 이 창에 먼저 적용, 제목 표시줄은 항상 다크 고정
  function pickTheme(value) {
    theme = value;
    applyTheme(theme);
    persist();
  }

  function onLanguageChange(e) {
    language = e.target.value;
    applyLanguage(language);
    persist();
  }

  function onExtractChange(field, checked) {
    if (field === "createSubfolder") extractCreateSubfolder = checked;
    else if (field === "deleteAfter") extractDeleteAfter = checked;
    else if (field === "openFolder") extractOpenFolder = checked;
    else if (field === "autoClose") extractAutoClose = checked;
    persist();
  }

  /**
   * 확장자별로 지금 쥐고 있는 다른 프로그램(ext → 표시 이름), 이 목록이 [기본 앱 선택]이 필요한 행
   */
  let owners = new Map();
  /**
   * 기본 앱이 우리를 가리키는데 등록이 없는 확장자, 남이 쥔 것과 같은 [적용안됨] 뱃지로 묶는다
   */
  let broken = new Set();
  /** 선택 창을 여는 중인 확장자(최대 5초), 그 뱃지 잠금으로 이중 개창 방지 */
  let picking = "";
  /** 선택 창을 한 번이라도 띄웠나 — 창으로 돌아왔을 때 뒷정리할지 가른다, */
  let pickerOpened = false;

  async function refreshAssocStatus() {
    try {
      const list = await fileAssocStatus(ASSOC_EXTS);
      const next = new Map();
      const nextBroken = new Set();
      for (const s of list) {
        if (s.other) next.set(s.ext, s.other);
        if (s.broken) nextBroken.add(s.ext);
      }
      owners = next;
      broken = nextBroken;
    } catch {
      // 상태를 못 읽는다고 화면을 막지 않는다
      owners = new Map();
      broken = new Set();
    }
  }

  /** 뱃지 클릭 — 그 확장자의 [기본 앱 선택] 창 표시(등록은 이미 완료) */
  async function onPickDefault(ext) {
    if (picking) return; // 한 번에 하나만
    picking = ext;
    try {
      // 비문서화 경로라 실패 가능 — 그때는 기본 앱 설정 전체 목록으로 폴백
      if (await openDefaultAppPicker(ext)) pickerOpened = true;
      else await openDefaultApps();
    } catch (e) {
      error = String(e);
    } finally {
      picking = "";
    }
  }

  // 선택 창의 종료 시점 불명, 이 창 복귀 시 정리 + 결과 재읽기
  let unlistenFocus = null;
  onMount(async () => {
    unlistenFocus = await onWindowFocus(async (focused) => {
      if (!focused || !pickerOpened) return;
      pickerOpened = false;
      try {
        await finishDefaultAppPicker();
      } catch {
        // 정리 실패로 화면을 막을 이유 없음
      }
      await refreshAssocStatus();
    });
  });
  onDestroy(() => unlistenFocus?.());

  // 파일 연결 — 저장 + 레지스트리 반영 + file_assoc_initialized 설정(사용자가 직접 정했다는 표시)
  async function applyAssoc() {
    fileAssocInitialized = true;
    await persist();
    try {
      await syncFileAssoc(ASSOC_EXTS.filter((e) => fileAssoc.has(e)));
    } catch (e) {
      error = String(e);
    }
    await refreshAssocStatus();
  }

  // Set 직접 변경 시 Svelte 가 갱신 미인지 → 새로 생성
  function onAssocToggle(ext, checked) {
    const next = new Set(fileAssoc);
    if (checked) next.add(ext);
    else next.delete(ext);
    fileAssoc = next;
    applyAssoc();
  }

  function onAssocAll(checked) {
    fileAssoc = checked ? new Set(ASSOC_EXTS) : new Set();
    applyAssoc();
  }

  $: assocAllChecked = fileAssoc.size === ASSOC_EXTS.length;
  // 아래 안내 = 체크했는데 왜 안 되는지 설명 → 체크된 것 중 막힌 수 계수
  $: blockedCount = ASSOC_EXTS.filter((e) => fileAssoc.has(e) && owners.has(e)).length;

  async function onOpenDefaultApps() {
    try {
      await openDefaultApps();
    } catch (e) {
      error = String(e);
    }
  }

  // 탐색기 통합 = 저장 + 레지스트리 반영(등록/해제)
  async function onShellIntegrationChange(checked) {
    shellIntegration = checked;
    await persist();
    try {
      await syncShellIntegration(shellIntegration);
    } catch (e) {
      error = String(e);
    }
  }

  // 초기화 — 모든 설정을 기본값으로, 다른 항목과 같이 즉시 저장
  async function onReset() {
    theme = "system";
    language = "system";
    extractCreateSubfolder = true;
    extractDeleteAfter = false;
    extractAutoClose = false;
    extractOpenFolder = false;
    shellIntegration = false;
    fileAssoc = new Set();
    applyTheme(theme);
    applyLanguage(language);
    await persist();
    // 레지스트리 기록도 함께 복원 — 설정만 비우면 연결과 메뉴가 잔존
    try {
      await syncShellIntegration(shellIntegration);
      await syncFileAssoc([]);
    } catch (e) {
      error = String(e);
    }
    await refreshAssocStatus();
  }

  $: currentLabel = $t(CATEGORIES.find((c) => c.id === selected)?.labelKey ?? "settings.title");
</script>

<div class="settings" data-ui="settings-window">
  <div class="main">
    <!-- 좌측: 카테고리 + 하단 초기화/확인 -->
    <aside class="sidebar" data-ui="settings-sidebar">
      <nav class="cats">
        {#each CATEGORIES as cat}
          <button
            type="button"
            class="cat"
            class:on={selected === cat.id}
            class:gap={cat.gap}
            on:click={() => (selected = cat.id)}
          >
            {$t(cat.labelKey)}
          </button>
        {/each}
      </nav>
      <!-- [확인]이 없다. 바꾸는 즉시 저장되므로 확정할 것이 없고, [닫기]는 창만 닫는다. -->
      <div class="side-actions">
        <button class="btn" on:click={onReset} disabled={!loaded}>{$t("settings.reset")}</button>
        <button class="btn primary" on:click={() => closeCurrentWindow()}>
          {$t("common.close")}
        </button>
      </div>
    </aside>

    <!-- 우측: 선택한 카테고리 콘텐츠 -->
    <section class="content" data-ui="settings-content">
      <h2 class="pane-title">{currentLabel}</h2>
      <hr />

      {#if selected === "general"}
        <div class="row">
          <span class="label">{$t("settings.theme")}</span>
          <div class="segmented" role="group">
            {#each THEMES as opt}
              <button
                type="button"
                class="seg"
                class:on={theme === opt.value}
                on:click={() => pickTheme(opt.value)}
              >
                {$t(opt.labelKey)}
              </button>
            {/each}
          </div>
        </div>

        <!-- 언어 설정(일반 설정에 포함) -->
        <h3 class="pane-title sub">{$t("settings.catLanguage")}</h3>
        <hr />
        <div class="row">
          <span class="label">{$t("settings.language")}</span>
          <select class="select" value={language} on:change={onLanguageChange}>
            <option value="system">{$t("settings.languageSystem")}</option>
            {#each LANGUAGES as lang}
              <option value={lang.code}>{lang.label}</option>
            {/each}
          </select>
        </div>
      {:else if selected === "fileAssoc"}
        <p class="desc">{$t("settings.assocDesc")}</p>

        <label class="check all">
          <input
            type="checkbox"
            checked={assocAllChecked}
            on:change={(e) => onAssocAll(e.currentTarget.checked)}
          />
          <span>{$t("settings.assocSelectAll")}</span>
        </label>

        <div class="ext-grid">
          {#each ASSOC_EXTS as ext}
            <!-- 뱃지는 label 밖에 둔다. label 안에 두면 뱃지를 눌러도 label 이 활성화되어
                 선택 창이 뜨는 동시에 연결이 해제된다. -->
            <div class="ext-row">
              <label class="check ext">
                <input
                  type="checkbox"
                  checked={fileAssoc.has(ext)}
                  on:change={(e) => onAssocToggle(ext, e.currentTarget.checked)}
                />
                <span>.{ext}</span>
              </label>
              {#if fileAssoc.has(ext) && (owners.has(ext) || broken.has(ext))}
                <!-- 남이 쥐고 있거나(other) 우리를 가리키는데 등록이 없다(broken). 사용자가
                     손대야 넘어가는 상태라는 점이 같으므로 문구를 나누지 않는다.
                     그 자리에서 바로 [기본 앱 선택] 창으로 보낸다
                     (더블클릭 경로는 만들지 않는다). 누가 쥐고 있는지는 툴팁에 있다 — 뱃지에는
                     프로그램 이름 대신 할 일을 적는다(길이가 들쭉날쭉하지 않다). -->
                <button
                  class="tag"
                  type="button"
                  disabled={picking !== ""}
                  title={owners.has(ext)
                    ? $t("settings.assocPickerHint", { app: owners.get(ext) })
                    : $t("settings.assocBrokenHint")}
                  on:click={() => onPickDefault(ext)}
                >
                  {picking === ext
                    ? $t("settings.assocPickerWorking")
                    : $t("settings.assocPickerForce")}
                </button>
              {/if}
            </div>
          {/each}
        </div>

        <!-- 체크했는데 아무 일도 안 일어나는 상태가 제일 나쁘다. 그런 확장자가 있을 때만
             그 사실을 알린다. 아직 겪지도 않은 제약을 미리 설명하면 화면만 무거워진다. -->
        {#if blockedCount > 0}
          <p class="note warn">{$t("settings.assocBlockedNote", { count: blockedCount })}</p>
        {/if}
        <div>
          <button
            class="btn"
            class:primary={blockedCount > 0}
            type="button"
            on:click={onOpenDefaultApps}
          >
            {$t("settings.assocOpenDefaults")}
          </button>
        </div>
      {:else if selected === "shellMenu"}
        <label class="check">
          <input
            type="checkbox"
            checked={shellIntegration}
            on:change={(e) => onShellIntegrationChange(e.currentTarget.checked)}
          />
          <span>{$t("settings.shellMenuEnable")}</span>
        </label>
        <p class="soon">{$t("settings.shellMenuHint")}</p>
      {:else if selected === "extract"}
        <label class="check">
          <input
            type="checkbox"
            checked={extractCreateSubfolder}
            on:change={(e) => onExtractChange("createSubfolder", e.currentTarget.checked)}
          />
          <span>{$t("settings.extractCreateSubfolder")}</span>
        </label>
        <label class="check">
          <input
            type="checkbox"
            checked={extractDeleteAfter}
            on:change={(e) => onExtractChange("deleteAfter", e.currentTarget.checked)}
          />
          <span>{$t("settings.extractDeleteAfter")}</span>
        </label>
        <label class="check">
          <input
            type="checkbox"
            checked={extractOpenFolder}
            on:change={(e) => onExtractChange("openFolder", e.currentTarget.checked)}
          />
          <span>{$t("settings.extractOpenFolder")}</span>
        </label>
        <label class="check">
          <input
            type="checkbox"
            checked={extractAutoClose}
            on:change={(e) => onExtractChange("autoClose", e.currentTarget.checked)}
          />
          <span>{$t("settings.extractAutoClose")}</span>
        </label>
      {:else}
        <p class="soon">{$t("common.comingSoon")}</p>
      {/if}

      {#if error}
        <div class="error" role="alert">{error}</div>
      {/if}
    </section>
  </div>
</div>

<style>
  .settings {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--bg);
    color: var(--text);
    font-size: 13px;
  }
  .main {
    flex: 1;
    display: flex;
    min-height: 0;
  }

  /* 좌측 사이드바 */
  .sidebar {
    width: 150px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border-right: 1px solid var(--border);
  }
  .cats {
    flex: 1;
    display: flex;
    flex-direction: column;
    padding: 8px 0;
    overflow-y: auto;
  }
  .cat {
    text-align: left;
    padding: 8px 16px;
    border: none;
    background: transparent;
    color: var(--text);
    cursor: pointer;
    font-size: 13px;
  }
  .cat:hover:not(.on) {
    background: var(--btn-bg);
  }
  .cat.on {
    background: var(--accent);
    color: var(--accent-contrast);
  }
  .cat.gap {
    margin-top: 16px;
  }
  .side-actions {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px;
    border-top: 1px solid var(--border);
  }

  /* 우측 콘텐츠 */
  .content {
    flex: 1;
    min-width: 0;
    padding: 16px 20px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .pane-title {
    margin: 0;
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
  }
  .pane-title.sub {
    margin-top: 22px;
  }
  .content hr {
    margin: 0 0 2px;
    border: none;
    border-top: 1px solid var(--border);
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }
  .label {
    font-size: 13px;
    color: var(--text);
  }
  .segmented {
    display: inline-flex;
    border: 1px solid var(--border);
    border-radius: 8px;
    overflow: hidden;
  }
  .seg {
    padding: 6px 14px;
    border: none;
    background: var(--btn-bg);
    color: var(--text);
    cursor: pointer;
    font-size: 13px;
    border-left: 1px solid var(--border);
  }
  .seg:first-child {
    border-left: none;
  }
  .seg.on {
    background: var(--accent);
    color: var(--accent-contrast);
  }
  .select {
    height: 32px;
    padding: 0 10px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--btn-bg);
    color: var(--text);
    font-size: 13px;
    cursor: pointer;
    min-width: 150px;
  }
  .check {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    font-size: 13px;
    color: var(--text);
    cursor: pointer;
    line-height: 1.4;
  }
  .check input {
    margin-top: 1px;
    flex-shrink: 0;
  }
  .soon {
    margin: 0;
    color: var(--text-muted);
    font-size: 13px;
  }
  .desc {
    margin: 0;
    font-size: 13px;
    line-height: 1.5;
  }
  /* 안내 문구 — 본문보다 약하게, 다만 판독은 가능해야 함(체크 무효 사례 설명) */
  .note {
    margin: 0;
    padding: 8px 10px;
    border-left: 2px solid var(--border);
    color: var(--text-muted);
    font-size: 12px;
    line-height: 1.5;
  }
  /* 전체 선택 = 목록의 머리 → 아래 항목과 선으로 구분 */
  .check.all {
    padding-bottom: 10px;
    border-bottom: 1px solid var(--border);
    font-weight: 600;
  }
  /* 확장자 10개를 3열로, 창 축소 시 자연히 2열
     최소 폭은 체크 + 확장자 칸 + [적용안됨] 뱃지가 한 줄에 들어가는 값이다. */
  .ext-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: 8px 12px;
  }
  /* 확장자 칸 고정 폭 — .7z 와 .bz2 의 글자 폭 차이로 뱃지 시작 위치가
     행마다 어긋나기 때문이다. 가장 긴 것(.bz2·.cbz)이 들어가는 폭으로 잡는다. */
  .check.ext span {
    font-variant-numeric: tabular-nums;
    min-width: 3.4em;
  }
  /* 확장자 한 줄 = 체크박스 + (막혔으면) 뱃지, 뱃지가 label 밖이라 줄로 묶는다, */
  .ext-row {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .ext-row .check.ext {
    align-items: center;
    min-width: 0;
  }
  /* 막힌 확장자 — 체크는 되지만 실제로는 다른 프로그램이 열림, 사용자 조작 필요
     상태이므로 (S) 시맨틱 "오류(alert)" 색을 쓴다(아래 .error 와 같은 토큰).
     누르면 그 확장자의 [기본 앱 선택] 창이 뜨므로 버튼처럼 보이게 한다. */
  .tag {
    padding: 0 6px;
    border: 1px solid transparent;
    border-radius: 3px;
    background: var(--alert-bg, #fde8e8);
    color: var(--alert-text, #9b1c1c);
    font-family: inherit;
    font-size: 11px;
    line-height: 1.6;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    cursor: pointer;
  }
  .tag:hover:not(:disabled) {
    border-color: currentColor;
  }
  .tag:disabled {
    cursor: default;
    opacity: 0.6;
  }
  .note.warn {
    border-left-color: var(--warn-text);
    color: var(--warn-text);
  }
  .error {
    color: var(--alert-text, #9b1c1c);
    background: var(--alert-bg, #fde8e8);
    padding: 8px 12px;
    border-radius: 6px;
    font-size: 13px;
  }

  .btn {
    padding: 7px 14px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--btn-bg);
    color: var(--text);
    cursor: pointer;
    font-size: 13px;
  }
  .btn.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--accent-contrast);
    font-weight: 600;
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
