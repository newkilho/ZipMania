<script>
  /**
   * 메인 창 상단 안내, 우리로 열리지 않는 확장자 하나 제시, 누르면 등록 → 그래도 아니면 [기본 앱 선택]
   * 확장자는 시작할 때 한 번만 고르고 실행 중에 바꾸지 않는다(D4)
   */
  import { onMount, onDestroy } from "svelte";
  import { t } from "../lib/i18n.js";
  import {
    defaultAssocExts,
    emitSettingsChanged,
    fileAssocStatus,
    finishDefaultAppPicker,
    getSettings,
    onSettingsChanged,
    onWindowFocus,
    openDefaultAppPicker,
    openDefaultApps,
    saveSettings,
    syncFileAssoc,
  } from "../lib/api.js";

  /** 이번 실행에서 물어볼 확장자, pick() 이 한 번 정하고 바뀌지 않는다, */
  let ext = "";
  let show = false;
  /** 등록, 선택 창 여는 중, 두 번 눌려 두 번 열리지 않게 잠근다, */
  let working = false;
  /** 선택 창을 띄웠나 — 창으로 돌아왔을 때 뒷정리할지 가른다, */
  let pickerOpened = false;

  /**
   * 이번 실행에서 물어볼 확장자 선택(시작 시 1회), 후보 = 사용자가 체크한 것, 정한 적 없으면 전체
   * 상태를 읽지 못하면 아무것도 띄우지 않는다
   */
  async function pick() {
    try {
      const s = await getSettings();
      if (s.assoc_banner_dismissed) return;

      const all = await defaultAssocExts();
      const chosen = s.file_assoc_initialized
        ? all.filter((e) => (s.file_assoc ?? []).includes(e))
        : all;
      if (chosen.length === 0) return;

      const status = new Map(
        (await fileAssocStatus(chosen)).map((x) => [x.ext, x]),
      );
      // 목록 순서 = 우선순위, 환경설정 표시 차례대로 하나씩 처리
      for (const e of chosen) {
        if (status.get(e)?.ours === false) {
          ext = e;
          show = true;
          return;
        }
      }
    } catch {
      // 판독 실패 시 조용히 통과
    }
  }

  /** 고른 확장자의 현재 소유 재확인 — 여기서 다음 확장자 탐색 금지 */
  async function refresh() {
    if (!ext) return;
    try {
      const s = await getSettings();
      if (s.assoc_banner_dismissed) {
        show = false;
        return;
      }
      const list = await fileAssocStatus([ext]);
      show = list.find((x) => x.ext === ext)?.ours === false;
    } catch {
      show = false;
    }
  }

  /** [연결] — 설정에 확장자를 넣고 등록한 뒤, 그래도 우리가 아니면 선택 창으로 보낸다, */
  async function onConnect() {
    if (working) return;
    working = true;
    try {
      const s = await getSettings();
      const has = new Set(s.file_assoc ?? []);
      has.add(ext);
      // 목록 순서로 다시 세운다 — 설정 창도 같은 순서로 쓰므로, 어느 쪽에서 바꾸든 파일 안의 줄이
      // 흔들리지 않는다
      const all = await defaultAssocExts();
      const exts = all.filter((e) => has.has(e));
      // 사용자 직접 지정 표시, 부재 시 다음 설치가 기본값으로 덮어쓰기
      const next = { ...s, file_assoc: exts, file_assoc_initialized: true };
      await saveSettings(next);
      await emitSettingsChanged(next);
      await syncFileAssoc(exts);

      // 등록만으로 전환됐는지 확인, 전환됐으면 여기서 종료
      const after = await fileAssocStatus([ext]);
      if (after.find((x) => x.ext === ext)?.ours !== true) {
        // UserChoice 는 프로그램이 못 바꾼다, 비문서화 경로라 실패 시 기본 앱 설정으로 폴백
        if (await openDefaultAppPicker(ext)) pickerOpened = true;
        else await openDefaultApps();
      }
    } catch {
      // 안내 줄에서 오류 창 표시 금지, 상태 재읽기 후 결과로 제시
    } finally {
      working = false;
      await refresh();
    }
  }

  /** [✕] — 다시 띄우지 않는다, 설정에 남기므로 다음 실행에서도 조용하다, */
  async function onDismiss() {
    show = false;
    try {
      const s = await getSettings();
      const next = { ...s, assoc_banner_dismissed: true };
      await saveSettings(next);
      await emitSettingsChanged(next);
    } catch {
      // 저장에 실패해도 이번 실행에서는 닫힌 채로 둔다
    }
  }

  let unlistenFocus = null;
  let unlistenSettings = null;

  onMount(async () => {
    // 확장자 결정은 여기 1회뿐, 이후 refresh() 는 고른 것만 재확인
    await pick();

    // 선택 창의 종료 시점 불명, 이 창 복귀 시 정리 + 결과 재읽기
    unlistenFocus = await onWindowFocus(async (focused) => {
      if (!focused) return;
      if (pickerOpened) {
        pickerOpened = false;
        try {
          await finishDefaultAppPicker();
        } catch {
          // 정리 실패로 화면을 막을 이유 없음
        }
      }
      await refresh();
    });

    // 환경설정 창에서 연결 시 이 줄도 즉시 소거 필요(설정은 저장 즉시 방송)
    unlistenSettings = await onSettingsChanged(() => refresh());
  });

  onDestroy(() => {
    unlistenFocus?.();
    unlistenSettings?.();
  });
</script>

{#if show}
  <div class="assoc-banner" data-ui="assoc-banner" role="status">
    <button class="close" type="button" on:click={onDismiss} title={$t("common.close")}>✕</button>
    <button class="msg" type="button" disabled={working} on:click={onConnect}>
      {working
        ? $t("app.assocBannerWorking")
        : $t("app.assocBannerAsk", { ext: ext.toUpperCase() })}
    </button>
  </div>
{/if}

<style>
  /* 오류(빨강 --alert-*)와 다른 층위다 — 알려 주는 것이지 잘못된 것이 아니라서 경고색을
     쓴다. 스킨이 이미 가진 토큰이므로 새로 만들지 않는다(다크 대응도 딸려 온다). */
  .assoc-banner {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    background: var(--warn-bg, #fdf3e0);
    color: var(--warn-text, #8a5a12);
    border-bottom: 1px solid var(--border);
    font-size: 13px;
  }
  /* 문구 전체가 버튼, 링크처럼 보이지 않아도 커서로 클릭 가능 표시 */
  .msg {
    flex: 1;
    text-align: left;
    border: none;
    background: transparent;
    color: inherit;
    font: inherit;
    padding: 2px 0;
    cursor: pointer;
  }
  .msg:hover:not(:disabled) {
    text-decoration: underline;
  }
  .msg:disabled {
    cursor: default;
    opacity: 0.7;
  }
  .close {
    order: -1; /* 닫기를 왼쪽에 둔다(반디집과 같은 배치) */
    border: none;
    background: transparent;
    color: inherit;
    cursor: pointer;
    font-size: 13px;
    padding: 2px 4px;
    line-height: 1;
  }
</style>
