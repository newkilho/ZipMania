// 압축 창이 요청 하나를 지금 어떻게 다룰지 정하는 규칙 — 순수 함수만 둔다(Svelte, Tauri 미참조)
// 창 없이 확인(tests/compress-plan.test.mjs)

/** 새 작업 수신 시 복원할 폼 기본값, 옵션도 초기화 — 잔존 시 다음 아카이브가 승계 */
export const FORM_DEFAULTS = {
  format: "zip",
  level: 5,
  password: "",
  encryptNames: false,
  eachMode: false,
  showPasswordPanel: false,
};

/** 배열이 아닌 값(누락, null)도 빈 배열로 받는다, */
function list(v) {
  return Array.isArray(v) ? v : [];
}

/** 폼에 아직 시작하지 않은 사용자 입력이 있나 — 있으면 독립 요청을 적용하지 않는다, */
export function isFormDirty(state) {
  return state?.phase === "form" && list(state?.inputs).length > 0;
}

/**
   * 이 요청이 독립 작업인가(자기 출력, 배치 보유), 그렇다면 폼을 통째로 되돌리고 시작
   * 일반 요청(경로만)은 기존 폼 위에 적층, Rust is_standalone 과 동일 필요
 */
export function isStandalone(launch) {
  const paths = list(launch?.inputs);
  const batch = list(launch?.batch);
  return batch.length > 0 || !!(launch?.autoStart && launch?.output && paths.length > 0);
}

/**
   * 요청 하나의 처리 계획, ignore(빈 요청), hold(독립인데 폼에 미처리 입력), apply(적용, reset 이면 폼 되돌림)
 */
export function planLaunch(state, launch) {
  const paths = list(launch?.inputs);
  const batch = list(launch?.batch);
  if (paths.length === 0 && batch.length === 0) return { action: "ignore" };

  const standalone = isStandalone(launch);
  if (standalone && isFormDirty(state)) return { action: "hold" };

  return {
    action: "apply",
    // 독립 요청이거나 완료 화면이면 앞 작업의 흔적(입력, 출력, 암호, 레벨, 각각압축)을 지운다
    reset: standalone || state?.phase === "done",
    mode: batch.length > 0 ? "batch" : standalone ? "auto" : "form",
    paths,
    batch,
    format: launch?.format || null,
    output: launch?.output || null,
  };
}

/** 배치 결과 누적, ok 아닌 것은 전부 흠집(error 만 세면 warning 이 덮인다), */
export function batchIssueAfter(issue, status) {
  return issue || status !== "ok";
}

/**
   * 계획 하나 실행, 상태 변경은 actions 에 맡기고 여기서는 순서와 성패 전달만
   * 창 밖 배치의 이유 = 그 성패 전달(D3.5)
 * @param {object} plan planLaunch 가 만든 계획(action === "apply")
 * @param {object} a 창이 주는 동작들
 * @param {() => void} a.resetForm 폼을 기본값으로 복원
 * @param {(f:string) => void} a.setFormat 포맷 지정
 * @param {() => void} a.clearBatchIssue 앞 배치의 흠집 표시를 지운다
 * @param {(items:Array) => void} a.setBatch 배치 목록을 얹는다(자동 제안 억제 포함)
 * @param {() => void} a.clearBatch 앞 배치의 흔적을 지운다
 * @param {(out:string) => void} a.setOutput 출력 경로 지정(자동 제안 억제 포함)
 * @param {(paths:string[]) => void} a.addInputs 입력을 목록에 추가
 * @param {() => Promise<void>} a.settle 반응성 반영 대기(Svelte tick)
 * @param {() => Promise<boolean>} a.runBatch 배치 시작 — 시작 성공 시 참
 * @param {() => Promise<boolean>} a.startAuto 자동 압축 시작 — 시작 성공 시 참
 * @returns {Promise<boolean>} 이 계획을 끝냈나(시작해야 하는데 못 했으면 거짓)
 */
export async function runPlan(plan, a) {
  if (plan.reset) a.resetForm();
  if (plan.format) a.setFormat(plan.format);
  a.clearBatchIssue();
  // 배치 아닌 요청은 앞 배치의 흔적 미승계(잔존 시 첫 항목만 압축)
  if (plan.mode === "batch") a.setBatch(plan.batch);
  else a.clearBatch();
  if (plan.mode === "auto") a.setOutput(plan.output);
  a.addInputs(plan.paths);

  // 반응성 반영(settle) 뒤 시작, 성패를 그대로 올려 보낸다
  if (plan.mode === "batch") {
    await a.settle();
    return await a.runBatch();
  }
  if (plan.mode === "auto") {
    await a.settle();
    return await a.startAuto();
  }
  // 목록 적층만 하는 요청(form)은 시작 대상 없음, 명시적 참 반환
  return true;
}
