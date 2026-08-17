// Load every skin at build time, Each stylesheet scopes itself with data-skin
const skinModules = import.meta.glob("../../skin/*/skin.css", { eager: true });

const availableSkins = new Set(
  Object.keys(skinModules).map((path) => path.split("/").at(-2)),
);

export const DEFAULT_SKIN = "default";

export function getAvailableSkins() {
  return [...availableSkins].sort();
}

export function applySkin(name = DEFAULT_SKIN) {
  if (typeof document === "undefined") return DEFAULT_SKIN;
  const selected = availableSkins.has(name) ? name : DEFAULT_SKIN;
  document.documentElement.dataset.skin = selected;
  return selected;
}
