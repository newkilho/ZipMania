// 누락 줄 조립 회귀 테스트 — node --test(의존성 없음)
//
// 백엔드는 사실(path, reason, detail)만 준다, 문장은 여기서만 만들어진다

import { test } from "node:test";
import assert from "node:assert/strict";
import { missingLines } from "../src/lib/format.js";

/** 키를 그대로 돌려주는 가짜 번역, 보간값은 뒤에 붙인다 */
const tr = (k, p) => (p ? `${k}:${Object.values(p).join(",")}` : k);

test("빠진 것이 없으면 줄도 없다", () => {
  assert.deepEqual(missingLines(tr, [], 0), []);
  assert.deepEqual(missingLines(tr, undefined, undefined), []);
});

test("항목마다 경로와 사유가 들어간다", () => {
  const lines = missingLines(
    tr,
    [
      { path: "docs/a.txt", reason: "createFile", detail: "Access is denied." },
      { path: "b.txt", reason: "link", detail: null },
    ],
    2,
  );
  assert.equal(lines[0], "missing.title:2");
  assert.ok(lines[1].includes("docs/a.txt"), lines[1]);
  assert.ok(lines[1].includes("missing.createFile"), lines[1]);
  assert.ok(lines[1].includes("Access is denied."), lines[1]);
  // detail 없는 항목에 빈 괄호를 붙이지 않는다
  assert.ok(!lines[2].includes("()"), lines[2]);
});

// 상한을 넘긴 만큼을 감추면 "그만큼만 빠졌다" 로 읽힌다
test("상한을 넘으면 나머지 개수를 마지막 줄로 알린다", () => {
  const items = Array.from({ length: 3 }, (_, i) => ({ path: `f${i}`, reason: "read" }));
  const lines = missingLines(tr, items, 10);
  assert.equal(lines[0], "missing.title:10");
  assert.equal(lines.at(-1).trim(), "missing.more:7");
});

// total 이 없거나 작으면 실린 개수가 곧 전체
test("total 이 없으면 실린 개수를 쓴다", () => {
  const lines = missingLines(tr, [{ path: "a", reason: "read" }]);
  assert.equal(lines[0], "missing.title:1");
  assert.equal(lines.length, 2);
});
