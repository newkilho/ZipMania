// 앱 마운트 진입점 — 창 label 로 분기(URL 쿼리 미사용): compress, extract, settings,
// 그 외 메인 창, mount 전 테마와 언어 적용으로 FOUC 차단, settings:changed 로 재적용
import "./styles/global.css";
import App from "./App.svelte";
import CompressWindow from "./components/CompressWindow.svelte";
import ExtractWindow from "./components/ExtractWindow.svelte";
import SettingsWindow from "./components/SettingsWindow.svelte";
import {
  currentWindowLabel,
  getSettings,
  onSettingsChanged,
  onWindowFocus,
} from "./lib/api.js";
import { applyTheme } from "./lib/theme.js";
import { applySkin } from "./lib/skin.js";
import { applyLanguage } from "./lib/i18n.js";

const label = currentWindowLabel();
const Root =
  label === "compress"
    ? CompressWindow
    : label === "extract"
      ? ExtractWindow
      : label === "settings"
        ? SettingsWindow
        : App;

async function boot() {
  applySkin("default");
  // (제목 표시줄 다크 강제 = Rust 의 창 생성 시점 적용, wintheme::apply_window_chrome)
  // 설정을 mount 전 적용(WebView 콘텐츠 테마, 언어), 실패해도 기본값(시스템)으로 진행
  try {
    const s = await getSettings();
    applyTheme(s.theme);
    applyLanguage(s.language);
  } catch {
    /* 기본값 유지 */
  }

  const app = new Root({ target: document.getElementById("app") });

  // 환경설정 창의 저장을 실시간 반영(WebView 콘텐츠 테마와 언어만, 크롬은 고정)
  // 환경설정은 즉시 저장 → 이 이벤트가 곧 선택 즉시 반영
  onSettingsChanged((s) => {
    applyTheme(s.theme);
    applyLanguage(s.language);
  }).catch(() => {});

  // 창 활성/비활성에 따른 상단 강조선 색 전환(활성=파랑 #3D84DD, 비활성=회색 #6D6D6D)
  onWindowFocus((focused) => {
    document.documentElement.classList.toggle("win-inactive", !focused);
  }).catch(() => {});

  return app;
}

boot();
