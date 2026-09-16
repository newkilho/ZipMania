// 분할 크기 해석 회귀 테스트 — node --test(의존성 없음), Rust cmdline::parse_size 와 같은 규칙

import { test } from "node:test";
import assert from "node:assert/strict";
import { parseSize, resolveVolume, SPLIT_CUSTOM, SPLIT_PRESETS } from "../src/lib/size.js";

test("단위 접미사, 대소문자, 뒤의 b 를 받는다", () => {
  assert.equal(parseSize("4GB"), 4 * 1024 ** 3);
  assert.equal(parseSize("700m"), 700 * 1024 ** 2);
  assert.equal(parseSize(" 100 K "), 100 * 1024);
  assert.equal(parseSize("12345"), 12345);
});

test("0, 빈 값, 모르는 단위, 음수는 null", () => {
  assert.equal(parseSize("0"), null);
  assert.equal(parseSize(""), null);
  assert.equal(parseSize(undefined), null);
  assert.equal(parseSize("4tb"), null);
  assert.equal(parseSize("-5m"), null);
  assert.equal(parseSize("gb"), null);
});

test("셀렉트 값과 직접 입력이 요청 값으로 모인다", () => {
  assert.equal(resolveVolume(0, "700M"), 0);
  assert.equal(resolveVolume(SPLIT_PRESETS[0].bytes, ""), SPLIT_PRESETS[0].bytes);
  assert.equal(resolveVolume(SPLIT_CUSTOM, "700M"), 700 * 1024 ** 2);
  assert.equal(resolveVolume(SPLIT_CUSTOM, ""), null);
  assert.equal(resolveVolume(SPLIT_CUSTOM, "x"), null);
});

test("프리셋은 0 보다 크고 오름차순이다", () => {
  for (let i = 0; i < SPLIT_PRESETS.length; i++) {
    assert.ok(SPLIT_PRESETS[i].bytes > 0);
    if (i > 0) assert.ok(SPLIT_PRESETS[i].bytes > SPLIT_PRESETS[i - 1].bytes);
  }
});
