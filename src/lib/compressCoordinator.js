// 압축 창 요청 처리 조율자, 창 상태와 백엔드 큐 사이의 전이만 담당(Svelte, Tauri 미참조, 주입)
// 규칙 = compressPlan.js, 전이 = 여기, 적용 = 창
// 규약: lease(id,gen) → dispatch(id,gen) → 적용 → ack(id,gen), dispatch 는 적용보다 먼저
// 신호는 세는 값(signalEpoch), 빌린 요청은 마감까지 처리 대상(held), 단계(held.stage), 답은 셋, (D3.5)
//
// held.stage — leased: 아무에게도 안 알림(죽으면 되돌아감), dispatched: 알림(되돌아가지 않음), applied: 반영 완료
// 전이 — idle: 할 일 없음, taking: 빌리고 알리는 중(창은 busy), applying: 창 상태 반영 중

import { planLaunch, isFormDirty } from "./compressPlan.js";

/**
 * @param {object} deps
 * @param {{lease:Function, dispatch:Function, ack:Function, peekStandalone:Function}} deps.api
 *  dispatch 와 ack 의 반환 = "ok"|"already"|"stale"
 * @param {{getState:Function, apply:Function}} deps.host 창
 *  getState() = {phase, inputs, busy} — 호출 시점의 현재 값
 *  busy = 작업 중(시작 중, 압축 중)만 의미(회수 중은 조율자 자신의 상태)
   *  apply(plan) = 성패 반환, 참이 아닌 것은 전부 실패
 * @param {(e:any)=>void} [deps.onError] 진단용(비치명적 — 다음 신호에 재시도)
 * @param {(s:string)=>void} [deps.onState] 전이 통지 — 창의 busy 갱신 근거
 */
export function createCoordinator({ api, host, onError, onState }) {
  let state = "idle";
  // 신호 계수(참/거짓 아님), 회수 중 도착 신호가 응답에 덮이지 않게
  let signalEpoch = 0;
  // 마지막으로 처리에 반영한 신호 번호
  let seenEpoch = 0;
  // 큐 잔여 여부에 대한 백엔드 통지 값
  let more = false;
  // 대여 후 미마감 요청 {id, gen, launch, stage}, 존재 동안은 처리 대상 있음
  let held = null;
  // 폼의 미처리 입력으로 독립 요청 보류, 폼 정리나 새 신호에 해제
  let blocked = false;
  // 마지막 시도 실패 표시, 즉시 재시도 시 무한 재시도
  let failed = false;
  // 진행 중인 처리(창이 mount 때 종료까지 대기하는 대상)
  let running = Promise.resolve();

  function fail(e) {
    failed = true;
    if (onError) onError(e);
  }

  /** 전이, 창의 busy 갱신 근거 — 회수 중에는 [압축 시작]도 차단 필요 */
  function go(next) {
    state = next;
    if (onState) onState(next);
  }

  /** 지금 창 상태, 캐시하지 않는다 — await 를 건넌 뒤에도 이것으로 다시 판정, */
  function now() {
    const s = host.getState() || {};
    return { state: s, busy: !!s.busy, formDirty: isFormDirty(s) };
  }

  /** 처리 잔여 여부 — 보유 요청, 큐 잔여, 미반영 신호 */
  function hasWork() {
    return !!held || more || seenEpoch !== signalEpoch;
  }

  /** 창 상태 변화마다 호출 — 막힘은 폼 정리 순간 해제, 그때만 재시도 */
  function poke() {
    if (blocked && !now().formDirty) blocked = false;
    failed = false; // 상태 변화 — 실패했던 것 재시도
    void pump();
  }

  /** 새 요청 신호(백엔드 이벤트), 막힘도 함께 해제 — 큐 변화로 재판정 */
  function signal() {
    signalEpoch += 1;
    blocked = false;
    failed = false;
    void pump();
  }

  /** 할 일이 있으면 시작, 진행 중인 처리 반환 */
  function pump() {
    if (state !== "idle") return running;
    if (!hasWork() || blocked || failed) return running;
    if (now().busy) return running; // 작업 중 — 끝나면 창이 다시 부른다
    running = doPump();
    return running;
  }

  /**
   * 지금 시작한 처리의 종료까지 대기(mount 의 최초 시작 판단 대기), 연속되면 그것까지
   */
  async function settled() {
    let prev;
    do {
      prev = running;
      await prev.catch(() => {});
    } while (prev !== running);
  }

  /** 대여, 성공 시 held 설정 */
  async function doLease() {
    const epoch = signalEpoch; // 응답 대기 중 도착 신호가 덮이지 않게 사전 확보
    // 폼이 정리되기 전에는 독립 요청을 꺼내지 않는다, 소유권은 큐에 두고 순서만 확인
    if (now().formDirty) {
      let standalone;
      try {
        standalone = await api.peekStandalone();
      } catch (e) {
        fail(e); // 다음 신호에 재시도(요청은 큐에 잔존)
        return false;
      }
      if (standalone) {
        blocked = true;
        return false;
      }
    }

    let take;
    try {
      take = await api.lease();
    } catch (e) {
      fail(e); // 대여 실패 — 큐에 잔존
      return false;
    }
    seenEpoch = epoch;
    more = !!take?.more;
    if (!take?.launch) return false;
    held = { id: take.id, gen: take.gen, launch: take.launch, stage: "leased" };
    return true;
  }

  /** 백엔드의 ok/already/stale 해석, stale 이면 들고 있던 것을 놓는다, */
  function accept(result, what) {
    if (result === "ok" || result === "already") return true;
    held = null;
    fail(new Error(`압축 요청 ${what}이 거부됐습니다(다른 창이 가져갔을 수 있습니다).`));
    return false;
  }

  async function doPump() {
    go("taking");
    try {
      // ── 1. 보유 요청 없으면 1개 대여 ──
      if (!held && !(await doLease())) return;

      // ── 2. 적용 직전 넘김 통지 ──
      if (held.stage === "leased") {
        // await 를 건넜다 → 다시 판정, 그 사이 작업이 시작됐으면 적용하지 않는다(들고만 있는다)
        const cur = now();
        if (cur.busy) return;

        const plan = planLaunch(cur.state, held.launch);
        if (plan.action === "hold") {
          blocked = true; // 미마감 — 폼 정리 후 속행
          return;
        }
        // 무동작 요청은 넘길 것도 없음 — 곧바로 마감으로
        if (plan.action === "ignore") {
          held.plan = null;
          held.stage = "applied";
        } else {
          let r;
          try {
            r = await api.dispatch(held.id, held.gen);
          } catch (e) {
            // 여기서 실패하면 아직 아무것도 안 했다, 그대로 두고 다음 신호에 처음부터
            fail(e);
            return;
          }
          if (!accept(r, "전달")) return;
          held.plan = plan;
          held.stage = "dispatched";
        }
      }

      // ── 3. 적용 ──
      if (held.stage === "dispatched") {
        if (now().busy) return; // 그 사이에 작업이 시작됐다 — 끝난 뒤에 이어서
        go("applying");
        let ok;
        try {
          ok = await host.apply(held.plan);
        } catch (e) {
          fail(e);
          return; // 미적용 — 미마감이므로 재시도
        }
        if (ok !== true) {
          // 작업 시작 실패(예: job_busy), 참이 아닌 것은 전부 실패
          fail(new Error("압축을 시작하지 못했습니다."));
          return;
        }
        held.stage = "applied";
      }

      // ── 4. 마감 ──
      let acked;
      try {
        acked = await api.ack(held.id, held.gen);
      } catch (e) {
        fail(e);
        return; // 마감만 재시도(stage 유지)
      }
      if (!accept(acked, "마감")) return;
      held = null;
    } finally {
      go("idle");
      // 잔여분 속행(한가할 때만 — pump 가 재판정)
      if (hasWork() && !blocked && !failed && !now().busy) void pump();
    }
  }

  return {
    poke,
    signal,
    settled,
    /** 현재 전이 상태(테스트, 진단용), */
    state: () => state,
    /** 처리 잔여 여부(테스트, 진단용) */
    isPending: () => hasWork(),
    /** 폼 때문에 막혀 있나(테스트, 진단용), */
    isBlocked: () => blocked,
    /** 빌려서 아직 마감하지 못한 요청의 단계(leased/dispatched/applied, 없으면 null), */
    heldStage: () => (held ? held.stage : null),
  };
}
