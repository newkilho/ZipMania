// 번역 사전 회귀 테스트 — node --test(의존성 없음)
//
//  npm test
//
// 사전은 생성물 하나(src/locales/strings.json)이고 정본은 zipmania-i18n 의 strings.rs 다
// 표 자체의 온전함(키 중복·빈 번역·치환자·생성물 최신)은 cargo test -p zipmania-i18n 이 본다
// 여기서 보는 것은 프런트가 실제로 쓰는 모양 — 언어 목록, 코드가 부르는 키의 존재

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const strings = JSON.parse(
  readFileSync(new URL("../src/locales/strings.json", import.meta.url), "utf8"),
);

const CODES = strings.langs.map((l) => l.code);
const REFERENCE = CODES[0];

test("언어 목록과 사전이 짝을 이룬다", () => {
  assert.ok(CODES.length >= 2, "언어가 하나뿐이다");
  for (const { code, label } of strings.langs) {
    assert.ok(strings.strings[code], `${code} 사전이 없다`);
    assert.notEqual(String(label).trim(), "", `${code} 표기가 비었다`);
  }
  assert.deepEqual(
    Object.keys(strings.strings).sort(),
    [...CODES].sort(),
    "사전에만 있거나 목록에만 있는 언어가 있다",
  );
});

test("모든 언어의 키 집합이 참조 언어와 같다", () => {
  const ref = Object.keys(strings.strings[REFERENCE]).sort();
  for (const code of CODES) {
    const keys = Object.keys(strings.strings[code]).sort();
    assert.deepEqual(keys, ref, `${code}: 키 집합이 다르다`);
  }
});

/** src 아래 모든 .svelte/.js 를 훑는다, */
function sourceFiles(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) sourceFiles(p, out);
    else if (name.endsWith(".svelte") || name.endsWith(".js")) out.push(p);
  }
  return out;
}

test("코드가 부르는 $t 키가 사전에 있다", () => {
  const dict = strings.strings[REFERENCE];
  const root = fileURLToPath(new URL("../src", import.meta.url));
  const missing = new Set();
  for (const file of sourceFiles(root)) {
    const src = readFileSync(file, "utf8");
    // $t("키") / t("키"), 앞 글자 검사로 sort( · set( 같은 이름은 걸러낸다
    // errText 의 errors.<code> 는 런타임 조합이라 여기서 잡히지 않는다
    for (const m of src.matchAll(/(?<![\w$])\$?t\(\s*"([a-zA-Z][\w.]*)"/g)) {
      if (!Object.prototype.hasOwnProperty.call(dict, m[1])) missing.add(m[1]);
    }
  }
  assert.deepEqual([...missing], [], "사전에 없는 키를 부른다");
});
