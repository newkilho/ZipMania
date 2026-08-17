// 압축 창 요청 규칙 회귀 테스트 — node --test(의존성 없음)
//
//  npm test
//
// 여기 있는 것은 전부 실제로 사용자 요청 유실이나 옵션 승계를 낸 결함
// 창 표시로는 재현 곤란(타이밍), 규칙이 순수 함수라 창 없이 고정 가능

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

import {
  planLaunch,
  isFormDirty,
  isStandalone,
  batchIssueAfter,
  FORM_DEFAULTS,
  runPlan,
} from "../src/lib/compressPlan.js";

/** 탐색기 "즉시 zip" 한 건, */
function auto(name) {
  return {
    inputs: [`C:/${name}.txt`],
    format: "zip",
    output: `C:/${name}.zip`,
    autoStart: true,
    batch: [],
  };
}

/** 메인 창 "선택 압축" 같은 일반 요청(경로만), */
function plain(...paths) {
  return { inputs: paths, format: null, output: null, autoStart: false, batch: [] };
}

const FORM_EMPTY = { phase: "form", inputs: [] };

test("즉시 ZIP A→B 는 각각 자기 아카이브로 간다", () => {
  // A 적용 시 자기 출력으로 시작
  const a = planLaunch(FORM_EMPTY, auto("A"));
  assert.equal(a.action, "apply");
  assert.equal(a.mode, "auto");
  assert.equal(a.output, "C:/A.zip");
  assert.equal(a.reset, true, "독립 요청은 앞 작업의 흔적을 지우고 시작한다");

  // A 종료(완료 화면) 뒤 도착한 B 도 자기 출력으로 — A 입력 미승계
  const afterA = { phase: "done", inputs: ["C:/A.txt"] };
  const b = planLaunch(afterA, auto("B"));
  assert.equal(b.action, "apply");
  assert.equal(b.output, "C:/B.zip");
  assert.equal(b.reset, true, "완료 화면의 입력을 물려주면 A+B → B.zip 이 된다");
  assert.deepEqual(b.paths, ["C:/B.txt"]);
});

test("일반 폼에 입력이 있으면 자동 요청이 그것을 지우지 않는다", () => {
  const dirty = { phase: "form", inputs: ["C:/사용자가-모은-것.txt"] };
  assert.equal(isFormDirty(dirty), true);

  const held = planLaunch(dirty, auto("B"));
  assert.equal(held.action, "hold", "폼을 지우지도, 요청을 버리지도 않는다");

  // 폼 정리(시작, 취소, 비움) 시점에 적용
  const applied = planLaunch(FORM_EMPTY, auto("B"));
  assert.equal(applied.action, "apply");
});

test("배치 요청도 미처리 폼을 밀어내지 않는다", () => {
  const dirty = { phase: "form", inputs: ["C:/a.txt"] };
  const batch = {
    inputs: [],
    format: "zip",
    output: null,
    autoStart: false,
    batch: [{ input: "C:/b", output: "C:/b.zip" }],
  };
  assert.equal(isStandalone(batch), true);
  assert.equal(planLaunch(dirty, batch).action, "hold");

  const plan = planLaunch(FORM_EMPTY, batch);
  assert.equal(plan.mode, "batch");
  assert.equal(plan.batch.length, 1);
});

test("일반 요청은 폼에 얹는다(목록 추가)", () => {
  const dirty = { phase: "form", inputs: ["C:/a.txt"] };
  const plan = planLaunch(dirty, plain("C:/b.txt"));
  assert.equal(plan.action, "apply", "일반 요청은 기다릴 이유가 없다");
  assert.equal(plan.mode, "form");
  assert.equal(plan.reset, false, "사용자가 모아 둔 목록을 지우지 않는다");
});

test("독립 요청은 앞 폼의 옵션을 물려받지 않는다", () => {
  // 암호를 걸어 둔 폼 뒤에 즉시 ZIP 도착 시 새 ZIP 의 암호화 금지
  // 그 보증 = reset, 되돌릴 값 = FORM_DEFAULTS
  assert.equal(planLaunch({ phase: "done", inputs: [] }, auto("B")).reset, true);
  assert.equal(FORM_DEFAULTS.password, "");
  assert.equal(FORM_DEFAULTS.encryptNames, false);
  assert.equal(FORM_DEFAULTS.eachMode, false, "남으면 지정된 단일 출력이 무시된다");
  assert.equal(FORM_DEFAULTS.showPasswordPanel, false);
  assert.equal(FORM_DEFAULTS.level, 5);
});

test("빈 회수는 아무것도 건드리지 않는다", () => {
  // 회수는 전역 1회라 이미 소비된 뒤일 수 있음, 그 빈 값으로 상태를 초기화하면
  // 배치, 자동 요청이 일반 폼으로 바뀐다
  assert.equal(planLaunch(FORM_EMPTY, null).action, "ignore");
  assert.equal(planLaunch(FORM_EMPTY, { inputs: [], batch: [] }).action, "ignore");
  assert.equal(planLaunch(FORM_EMPTY, {}).action, "ignore");
});

test("배치는 warning 도 흠집으로 센다", () => {
  // error 만 계수 시 일부 항목이 빠진 배치가 마지막 항목의 성공으로 ok
  // ok = 앱의 원본 삭제 허용 신호
  assert.equal(batchIssueAfter(false, "ok"), false);
  assert.equal(batchIssueAfter(false, "warning"), true);
  assert.equal(batchIssueAfter(false, "error"), true);
  assert.equal(batchIssueAfter(true, "ok"), true, "앞의 흠집이 뒤의 성공에 덮이면 안 된다");
});

test("자동 시작이라도 출력·입력이 없으면 독립 작업이 아니다", () => {
  assert.equal(isStandalone({ inputs: ["C:/a"], autoStart: true, output: null }), false);
  assert.equal(isStandalone({ inputs: [], autoStart: true, output: "C:/a.zip" }), false);
  assert.equal(isStandalone(plain("C:/a")), false);
});

// ─────────────── 계획 실행 — 성패 전달 고정 ───────────────
//
// 이 자리에서 실제로 값을 흘렸다, 창 안의 applyPlan 이 await onStart(...) 의 반환을
// 빠뜨려 undefined 로 끝났고, 조율자가 그것을 성공으로 읽어 **시작하지도 못한 요청을
// 마감했다.** 그때 조율자 테스트는 초록불이었다 — 가짜 창이 값을 돌려주도록 만들어져
// 있었던 탓(가짜가 실물보다 올발랐다), 그래서 그 경로를 창 밖으로 빼 여기서 확인

/** 동작 흉내 — 호출 순서와 시작 성패 기록 */
function makeActions({ batchOk = true, autoOk = true } = {}) {
  const log = [];
  return {
    log,
    resetForm: () => log.push("reset"),
    setFormat: (f) => log.push(`format:${f}`),
    clearBatchIssue: () => log.push("clearIssue"),
    setBatch: (items) => log.push(`batch:${items.length}`),
    clearBatch: () => log.push("clearBatch"),
    setOutput: (o) => log.push(`output:${o}`),
    addInputs: (p) => log.push(`inputs:${p.join(",")}`),
    settle: async () => log.push("settle"),
    runBatch: async () => {
      log.push("runBatch");
      return batchOk;
    },
    startAuto: async () => {
      log.push("startAuto");
      return autoOk;
    },
  };
}

test("자동 시작이 실패하면 실행도 실패로 끝난다", async () => {
  const a = makeActions({ autoOk: false });
  const plan = planLaunch(
    { phase: "form", inputs: [] },
    { inputs: ["C:/a.txt"], output: "C:/a.zip", autoStart: true, batch: [] },
  );
  assert.equal(await runPlan(plan, a), false, "시작 못 했는데 성공으로 끝났다");
  assert.ok(a.log.includes("startAuto"));
});

test("배치 시작이 실패하면 실행도 실패로 끝난다", async () => {
  const a = makeActions({ batchOk: false });
  const plan = planLaunch(
    { phase: "form", inputs: [] },
    { inputs: [], batch: [{ input: "C:/x", output: "C:/x.zip" }] },
  );
  assert.equal(await runPlan(plan, a), false, "시작 못 했는데 성공으로 끝났다");
  assert.ok(a.log.includes("runBatch"));
});

test("시작에 성공하면 참을 돌려준다", async () => {
  const a = makeActions();
  const plan = planLaunch(
    { phase: "form", inputs: [] },
    { inputs: ["C:/a.txt"], output: "C:/a.zip", autoStart: true, batch: [] },
  );
  assert.equal(await runPlan(plan, a), true);
});

test("목록에 얹기만 하는 요청은 명시적으로 참이다", async () => {
  // 조율자는 참이 아닌 것을 전부 실패로 판정 — 누락 시 일반 요청이 영영
  // 미마감 상태로 남아 폼 정리마다 같은 실패 반복
  const a = makeActions();
  const plan = planLaunch({ phase: "form", inputs: [] }, { inputs: ["C:/a.txt"], batch: [] });
  assert.equal(plan.mode, "form");
  assert.equal(await runPlan(plan, a), true, "얹기만 하는 요청이 실패로 끝났다");
  assert.ok(!a.log.includes("startAuto"), "시작할 것이 없는데 시작했다");
  assert.ok(!a.log.includes("runBatch"));
});

test("배치가 아닌 요청은 앞 배치의 흔적을 지운다", async () => {
  // 남기면 첫 항목만 압축되고 나머지가 사라진다
  const a = makeActions();
  const plan = planLaunch(
    { phase: "form", inputs: [] },
    { inputs: ["C:/a.txt"], output: "C:/a.zip", autoStart: true, batch: [] },
  );
  await runPlan(plan, a);
  assert.ok(a.log.includes("clearBatch"), "앞 배치를 물려받는다");
  assert.ok(a.log.includes("output:C:/a.zip"));
});

test("독립 요청은 폼을 되돌린 뒤 얹는다", async () => {
  // 순서가 뒤집히면 방금 넣은 입력을 되돌리기가 지운다
  const a = makeActions();
  const plan = planLaunch(
    { phase: "form", inputs: [] },
    { inputs: ["C:/a.txt"], output: "C:/a.zip", autoStart: true, batch: [] },
  );
  await runPlan(plan, a);
  assert.ok(a.log.indexOf("reset") < a.log.indexOf("inputs:C:/a.txt"), "되돌리기가 뒤에 왔다");
  assert.ok(a.log.indexOf("inputs:C:/a.txt") < a.log.indexOf("startAuto"), "빈 목록으로 시작했다");
});

test("창의 applyPlan 은 값을 그대로 올려 보낸다", () => {
  // 실물의 반환 경로를 여기서 못 박는다, runPlan 을 아무리 잘 만들어도, 창 쪽
  // 어댑터가 블록 본문으로 바뀌며 return 누락 시 같은 사고 재발
  // (실제로 그렇게 났고, 가짜 창이 값을 돌려주도록 만들어져 있어 초록불이었다)
  // 화살표 식(expression body)은 값을 버릴 자리 없음
  const src = readFileSync(new URL("../src/components/CompressWindow.svelte", import.meta.url), "utf8");
  assert.match(
    src,
    /const applyPlan = \(plan\) => runPlan\(plan, planActions\);/,
    "applyPlan 이 `runPlan` 을 그대로 돌려주는 화살표 한 줄이 아니다",
  );
  // 순서, 분기가 창으로 되돌아오면(= 창 안에서 다시 시작을 부르면) 이 잠금이 무의미해진다
  assert.ok(
    !/async function applyPlan/.test(src),
    "applyPlan 이 다시 창 안의 함수가 됐다 — 성패 전달을 놓칠 자리가 생긴다",
  );
});
