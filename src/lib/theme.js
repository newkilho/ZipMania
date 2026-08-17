// 테마 적용 — settings.toml 의 theme 를 문서에 반영(TOML 이 유일 소스, localStorage 없음)
// , "light"/"dark" → :root 의 data-theme 속성으로 CSS 강제
// , "system" → data-theme 제거로 CSS @media (prefers-color-scheme) 에 위임

/**
 * 테마를 현재 문서에 적용
 * @param {"system"|"light"|"dark"} theme
 */
export function applyTheme(theme) {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  if (theme === "light" || theme === "dark") {
    root.setAttribute("data-theme", theme);
  } else {
    root.removeAttribute("data-theme"); // system
  }
}
