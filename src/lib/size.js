// 분할 크기 입력 해석 — 순수 함수만 둔다(Svelte, Tauri 미참조), Rust cmdline::parse_size 와 같은 규칙

/** 분할 프리셋(바이트), 라벨은 번역 불필요한 숫자 표기 */
export const SPLIT_PRESETS = [
  { bytes: 10 * 1024 * 1024, label: "10 MB" },
  { bytes: 25 * 1024 * 1024, label: "25 MB" },
  { bytes: 100 * 1024 * 1024, label: "100 MB" },
  { bytes: 700 * 1024 * 1024, label: "700 MB" },
  { bytes: 1024 * 1024 * 1024, label: "1 GB" },
  { bytes: 4 * 1024 * 1024 * 1024, label: "4 GB" },
];

/** 셀렉트의 [직접 입력] 값 */
export const SPLIT_CUSTOM = -1;

/**
 * "700M", "4GB", "12345" → 바이트, 단위 없음 = 바이트, 0 이나 해석 불가 = null
 * @param {string} text
 * @returns {number|null}
 */
export function parseSize(text) {
  const m = /^\s*(\d+)\s*([kmg]?)b?\s*$/i.exec(text ?? "");
  if (!m) return null;
  const n = Number(m[1]);
  if (!Number.isSafeInteger(n) || n === 0) return null;
  const mul = { "": 1, k: 1024, m: 1024 ** 2, g: 1024 ** 3 }[m[2].toLowerCase()];
  const bytes = n * mul;
  return Number.isSafeInteger(bytes) ? bytes : null;
}

/**
 * 폼 상태 → 요청에 실을 분할 크기, 분할 없음 = 0, 직접 입력이 틀리면 null(시작 불가)
 * @param {number} choice 셀렉트 값(0 = 없음, SPLIT_CUSTOM = 직접 입력, 그 외 = 바이트)
 * @param {string} text 직접 입력 문자열
 * @returns {number|null}
 */
export function resolveVolume(choice, text) {
  if (choice === SPLIT_CUSTOM) return parseSize(text);
  return choice > 0 ? choice : 0;
}
