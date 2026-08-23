// 진행 속도, 남은 시간 계산 + 표시 문자열
//
// 측정기는 순수 함수다 — 시각(ms)을 호출측이 넘긴다(테스트에서 시간을 만들 수 있게)
// 속도는 지수이동평균 — 순간값은 파일 경계마다 튀고, 누적 평균은 뒤늦게 따라온다

import { formatSize } from "./format.js";

// 표본 최소 간격(ms), 이보다 잦은 이벤트는 앞 표본에 합친다
const MIN_SAMPLE_MS = 400;
// 지수이동평균 가중치 — 최근 표본 비중
const ALPHA = 0.3;

/**
 * 측정기 생성
 * @param {number} nowMs 시작 시각
 * @param {number} [done] 시작 시점 처리량(바이트)
 */
export function newMeter(nowMs, done = 0) {
  return { startMs: nowMs, lastMs: nowMs, lastDone: done, bps: 0 };
}

/**
 * 표본 추가 → 새 측정기(원본 불변), 간격 미달이면 그대로
 * 처리량이 줄면(배치 다음 항목) 속도를 유지한 채 기준만 옮긴다
 * @param {{startMs:number,lastMs:number,lastDone:number,bps:number}} meter
 * @param {number} done 누적 처리량(바이트)
 * @param {number} nowMs 현재 시각
 */
export function sample(meter, done, nowMs) {
  if (!meter) return newMeter(nowMs, done);
  if (done < meter.lastDone) return { ...meter, lastMs: nowMs, lastDone: done };
  const dt = nowMs - meter.lastMs;
  if (dt < MIN_SAMPLE_MS) return meter;
  const inst = ((done - meter.lastDone) * 1000) / dt;
  const bps = meter.bps > 0 ? ALPHA * inst + (1 - ALPHA) * meter.bps : inst;
  return { startMs: meter.startMs, lastMs: nowMs, lastDone: done, bps };
}

/**
 * 남은 시간(초), 속도를 모르거나 총량이 없으면 null
 * @param {{bps:number}} meter
 * @param {number} done
 * @param {number} total
 * @returns {number|null}
 */
export function etaSec(meter, done, total) {
  if (!meter || !(meter.bps > 0)) return null;
  if (!total || total <= done) return null;
  return (total - done) / meter.bps;
}

/**
 * 초 → 시:분:초, 한 시간 미만이면 분:초
 * @param {number|null} sec
 * @returns {string} 모르면 "—"
 */
export function formatDuration(sec) {
  if (sec == null || !isFinite(sec) || sec < 0) return "—";
  const total = Math.floor(sec);
  const s = total % 60;
  const m = Math.floor(total / 60) % 60;
  const h = Math.floor(total / 3600);
  const pad = (n) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}

/**
 * 초당 바이트 → 표시 문자열
 * @param {number} bps
 * @returns {string} 예: "12.3 MB/s", 모르면 "—"
 */
export function formatSpeed(bps) {
  if (!bps || !isFinite(bps) || bps <= 0) return "—";
  return `${formatSize(bps)}/s`;
}
