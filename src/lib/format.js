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

/**
 * 빠진 항목(job:done 의 missing) → 로그 줄 배열, 문장 조립은 여기 한 곳
 * 백엔드는 사실만 준다 — path, reason(missing.<reason> 키), detail(OS 원문, 번역 대상 아님)
 * @param {(k:string,p?:object)=>string} tr 번역 함수
 * @param {Array<{path:string,reason:string,detail?:string}>} missing 빠진 항목(상한만큼)
 * @param {number} [total] 자르기 전 개수, missing.length 보다 크면 나머지를 마지막 줄로
 * @returns {string[]}
 */
export function missingLines(tr, missing, total) {
  const items = Array.isArray(missing) ? missing : [];
  const count = typeof total === "number" && total > items.length ? total : items.length;
  if (count === 0) return [];
  const lines = [tr("missing.title", { count })];
  for (const m of items) {
    const why = tr(`missing.${m.reason}`);
    const detail = m && m.detail ? ` (${m.detail})` : "";
    lines.push(`  ${m.path} — ${why}${detail}`);
  }
  if (count > items.length) {
    lines.push(`  ${tr("missing.more", { count: count - items.length })}`);
  }
  return lines;
}
