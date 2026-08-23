// 진행 속도, 남은 시간 회귀 테스트 — node --test(의존성 없음)
//
// 측정기는 시각을 인자로 받는다 — 여기서 시간을 만들어 검증한다

import { test } from "node:test";
import assert from "node:assert/strict";
import { newMeter, sample, etaSec, formatDuration, formatSpeed } from "../src/lib/progress.js";

const MB = 1024 * 1024;

test("표본 간격 미달이면 속도를 갱신하지 않는다", () => {
  let m = newMeter(0);
  m = sample(m, 10 * MB, 100);
  assert.equal(m.bps, 0, "100ms 만에 속도를 단정했다");
});

test("속도는 초당 바이트다", () => {
  let m = newMeter(0);
  m = sample(m, 10 * MB, 1000);
  assert.ok(Math.abs(m.bps - 10 * MB) < 1, `${m.bps}`);
});

// 배치 다음 항목으로 넘어가면 처리량이 0 부터 다시 센다
test("처리량이 줄면 기준만 옮기고 속도는 유지한다", () => {
  let m = newMeter(0);
  m = sample(m, 10 * MB, 1000);
  const before = m.bps;
  m = sample(m, 0, 1200);
  assert.equal(m.bps, before, "다음 항목 시작에서 속도를 잃었다");
  assert.equal(m.lastDone, 0, "기준이 옮겨지지 않았다");
});

test("남은 시간 = 남은 바이트 / 속도", () => {
  let m = newMeter(0);
  m = sample(m, 10 * MB, 1000); // 10 MB/s
  assert.ok(Math.abs(etaSec(m, 10 * MB, 30 * MB) - 2) < 0.01);
});

// 모르는 것을 0 으로 표시하면 "곧 끝난다" 로 읽힌다
test("속도나 총량을 모르면 남은 시간은 null", () => {
  const m = newMeter(0);
  assert.equal(etaSec(m, 0, 100), null, "속도를 모르는데 예상을 냈다");
  let m2 = sample(newMeter(0), 10 * MB, 1000);
  assert.equal(etaSec(m2, 5 * MB, 0), null, "총량을 모르는데 예상을 냈다");
  assert.equal(etaSec(m2, 100, 100), null, "다 끝났는데 남은 시간을 냈다");
});

test("시간 표기는 한 시간을 넘으면 시:분:초", () => {
  assert.equal(formatDuration(0), "0:00");
  assert.equal(formatDuration(34), "0:34");
  assert.equal(formatDuration(62), "1:02");
  assert.equal(formatDuration(3723), "1:02:03");
  assert.equal(formatDuration(null), "—");
  assert.equal(formatDuration(Infinity), "—");
});

test("속도 표기는 모르면 대시", () => {
  assert.equal(formatSpeed(0), "—");
  assert.equal(formatSpeed(Infinity), "—");
  assert.ok(formatSpeed(1536).endsWith("/s"));
});
