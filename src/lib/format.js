// 크기, 날짜 포맷 유틸

/**
 * 바이트 → 사람이 읽는 크기 문자열
 * @param {number} bytes
 * @returns {string} 예: "2.1 MB"
 */
export function formatSize(bytes) {
  if (bytes === null || bytes === undefined) return "—";
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  const value = bytes / Math.pow(1024, i);
  return `${value.toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
}

/**
 * 날짜 → 로컬 문자열
 * @param {Date|string|number} date
 * @returns {string}
 */
export function formatDate(date) {
  if (!date) return "—";
  const d = date instanceof Date ? date : new Date(date);
  if (isNaN(d.getTime())) return "—";
  return d.toLocaleString("ko-KR");
}

/**
 * 7z 수정일 문자열 → YYYY-MM-DD HH:MM:SS, 없으면 —
 * @param {string|null|undefined} modified
 * @returns {string}
 */
export function formatModified(modified) {
  if (!modified) return "—";
  // 소수점 이하(고정밀 초)는 잘라낸다, 표준 형식이면 앞 19자가 날짜+시각
  const trimmed = String(modified).split(".")[0].trim();
  return trimmed || "—";
}

/**
 * 압축률(%) = 원본 대비 절감률
 * @param {number} original
 * @param {number} packed
 * @returns {string} 예: "87%"
 */
export function formatRatio(original, packed) {
  if (!original || original === 0) return "—";
  const ratio = Math.round((1 - packed / original) * 100);
  return `${ratio}%`;
}
