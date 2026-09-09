// 빌드 때 모든 스킨을 싣는다, 스킨 이름 = 폴더 이름
// [data-skin] 한정은 skin-scope 플러그인이 폴더 이름으로 채운다
const skinModules = import.meta.glob("../../skin/*/skin.css", { eager: true });

const availableSkins = new Set(
  Object.keys(skinModules).map((path) => path.split("/").at(-2)),
);

export const DEFAULT_SKIN = "default";

export function getAvailableSkins() {
  return [...availableSkins].sort();
}

/** 없는 이름을 골랐을 때의 대체 — default, 없으면 남은 것 중 첫 번째 */
function fallbackSkin() {
  if (availableSkins.has(DEFAULT_SKIN)) return DEFAULT_SKIN;
  return getAvailableSkins()[0] ?? DEFAULT_SKIN;
}

export function applySkin(name = DEFAULT_SKIN) {
  const selected = availableSkins.has(name) ? name : fallbackSkin();
  if (typeof document === "undefined") return selected;
  document.documentElement.dataset.skin = selected;
  return selected;
}
