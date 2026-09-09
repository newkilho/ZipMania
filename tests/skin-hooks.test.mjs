// 스킨 훅 정합성 테스트 — node --test(의존성 없음)
//
//  npm test
//
// 스킨은 data-ui 선택자로만 앱 화면을 잡는다, 정본은 컴포넌트 마크업이고
// 사본이 skin/default/README.md 의 "CSS 적용 대상" 목록 하나
// 갈리면 스킨 제작자가 없는 선택자에 규칙을 쓰거나 새 화면을 영영 못 잡는다

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

const SRC = new URL("../src/", import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, "$1");
const README = new URL("../skin/default/README.md", import.meta.url).pathname.replace(
  /^\/([A-Za-z]:)/,
  "$1",
);

/** src/ 아래 .svelte 전부 */
function svelteFiles(dir) {
  const out = [];
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, e.name);
    if (e.isDirectory()) out.push(...svelteFiles(p));
    else if (e.name.endsWith(".svelte")) out.push(p);
  }
  return out;
}

/** 마크업의 data-ui 값 집합 */
function hooksInSource() {
  const found = new Set();
  for (const f of svelteFiles(SRC)) {
    const raw = readFileSync(f, "utf-8");
    for (const m of raw.matchAll(/data-ui="([^"{]+)"/g)) found.add(m[1]);
  }
  return found;
}

/** README "CSS 적용 대상" 코드 블록의 [data-ui="..."] 집합 */
function hooksInReadme() {
  const raw = readFileSync(README, "utf-8");
  const head = raw.indexOf("## CSS 적용 대상");
  assert.notEqual(head, -1, "README 에 'CSS 적용 대상' 절이 없다");
  const s = raw.indexOf("```css", head);
  const e = raw.indexOf("```", s + 6);
  assert.ok(s !== -1 && e !== -1, "선택자 코드 블록이 없다");
  const block = raw.slice(s, e);
  const found = new Set();
  for (const m of block.matchAll(/\[data-ui="([^"]+)"\]/g)) found.add(m[1]);
  return found;
}

test("스킨 훅 목록이 마크업과 같다", () => {
  const src = hooksInSource();
  const doc = hooksInReadme();

  const missing = [...src].filter((h) => !doc.has(h)).sort();
  const extra = [...doc].filter((h) => !src.has(h)).sort();

  assert.deepEqual(
    missing,
    [],
    `마크업에만 있는 훅 — skin/default/README.md 의 목록에 추가할 것: ${missing.join(", ")}`,
  );
  assert.deepEqual(
    extra,
    [],
    `README 에만 있는 훅 — 마크업에서 사라졌으므로 목록에서 뺄 것: ${extra.join(", ")}`,
  );
});

test("압축 창과 압축 풀기 창의 진행 화면에 훅이 있다", () => {
  const src = hooksInSource();
  // 두 창이 공유하는 JobView, 훅 없이 두면 스킨이 진행, 결과 화면을 잡지 못한다
  for (const h of [
    "compress-window",
    "extract-window",
    "job-view",
    "job-result",
    "job-actions",
    "progress-bar",
    "window-actions",
  ]) {
    assert.ok(src.has(h), `data-ui="${h}" 가 마크업에 없다`);
  }
});
