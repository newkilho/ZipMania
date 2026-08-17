// 압축 창 요청 처리 전이 회귀 테스트 — node --test(의존성 없음)
//
// 백엔드 큐(lease/dispatch/ack/peek)를 실제 Rust 와 같은 규약으로 흉내 내고 창을 가짜로
// 세워, 순수 함수 테스트가 잡지 못하는 순서와 타이밍 결함 고정
// 전부 실제로 요청 유실, 압축 누락, IPC 폭주를 낸 것들
//
// 가짜는 인터페이스가 아니라 판정까지 닮아야 유효, 가짜 창이 실제 시작 판정을
// 흉내 내지 않아서, 자동 시작이 자기 자신에게 막히는 결함을 초록불 열넷이 통과시켰다
//
// 정상 흐름만 흉내 내면 무의미, 남은 결함은 전부 실패와 응답 유실 쪽 —
// 그래서 이 파일의 큐는 호출마다 세 가지로 고장 가능
//
//  before — 서버 실행 전 실패(호출 미도달)
//  after — 서버는 실행, 응답만 유실(효과는 잔존)
//  value — 서버가 already/stale 반환
//
// 앞의 둘을 구분하지 못하면 요청 정지(성공을 실패로) 또는 이중 압축(실패를 성공으로)

import { test } from "node:test";
import assert from "node:assert/strict";

import { createCoordinator } from "../src/lib/compressCoordinator.js";
import { isStandalone } from "../src/lib/compressPlan.js";

/**
 * 백엔드 큐 흉내 — Rust PendingCompress 와 같은 규약
 *
 * lease 는 큐에서 빼지 않는다(마감해야 빠진다), 같은 세션의 재시도에는 같은 번호, 세대를
 * 주고, 살아 있는 다른 세션의 것은 뺏지 않는다, dispatch, ack 는 ok/already/stale
 */
function makeQueue(items = []) {
  let nextId = 0;
  let nextGen = 0;
  const q = items.map((launch) => ({ id: ++nextId, launch, lease: null }));
  const recent = [];
  const calls = { lease: 0, ack: 0, peek: 0, dispatch: 0 };
  /** {call, mode} — 다음 그 호출 한 번만 고장 낸다, */
  let fault = null;
  let session = "compress#1";

  /** 고장 소비, before 면 서버 미실행 후 throw */
  function pre(call) {
    if (fault?.call !== call) return null;
    const f = fault;
    fault = null;
    if (f.mode === "before") throw new Error(`IPC 실패(${call}, 실행 전)`);
    return f;
  }
  /** 서버 실행 뒤 — "after" 면 결과를 남긴 채 응답만 잃는다, */
  function post(f, result) {
    if (f?.mode === "after") throw new Error(`IPC 실패(${f.call}, 응답 유실)`);
    if (f?.mode === "value") return f.value;
    return result;
  }

  const api = {
    async lease() {
      calls.lease++;
      const f = pre("lease");
      const front = q[0];
      let out = { id: 0, gen: 0, launch: null, more: false };
      if (front) {
        const more = q.length > 1;
        if (front.lease && front.lease.session === session) {
          // 같은 창 재시도 — 그대로 재반환(멱등)
          out = { id: front.id, gen: front.lease.gen, launch: front.launch, more };
        } else if (front.lease) {
          // 살아 있는 다른 창이 보유 — 탈취 금지
          out = { id: 0, gen: 0, launch: null, more: false };
        } else {
          front.lease = { gen: ++nextGen, session, dispatched: false };
          out = { id: front.id, gen: front.lease.gen, launch: front.launch, more };
        }
      }
      return post(f, out);
    },
    async dispatch(id, gen) {
      calls.dispatch++;
      const f = pre("dispatch");
      const front = q[0];
      let r;
      if (front && front.id === id && front.lease?.gen === gen) {
        r = front.lease.dispatched ? "already" : "ok";
        front.lease.dispatched = true;
      } else if (recent.some(([i, g]) => i === id && g === gen)) {
        r = "already";
      } else {
        r = "stale";
      }
      return post(f, r);
    },
    async ack(id, gen) {
      calls.ack++;
      const f = pre("ack");
      const front = q[0];
      let r;
      if (front && front.id === id && front.lease?.gen === gen) {
        q.shift();
        recent.push([id, gen]);
        r = "ok";
      } else if (recent.some(([i, g]) => i === id && g === gen)) {
        r = "already";
      } else {
        r = "stale";
      }
      return post(f, r);
    },
    async peekStandalone() {
      calls.peek++;
      const f = pre("peek");
      return post(f, q[0] ? isStandalone(q[0].launch) : false);
    },
  };

  return {
    q,
    calls,
    api,
    /** 다음 그 호출 한 번을 고장 낸다, mode: before|after|value, */
    failOnce(call, mode = "before", value) {
      fault = { call, mode, value };
    },
    /** 창 사망 — 넘기기 전 요청은 되돌림, 넘긴 것은 폐기(Rust release_session) */
    windowDied() {
      for (let i = q.length - 1; i >= 0; i--) {
        const l = q[i].lease;
        if (!l || l.session !== session) continue;
        if (l.dispatched) q.splice(i, 1);
        else q[i].lease = null;
      }
    },
    /** 새 창이 열렸다(세션이 바뀐다 — label 은 같다), */
    newWindow() {
      session = `compress#${Number(session.split("#")[1]) + 1}`;
      return session;
    },
    /** 새 요청 도착(Rust push), */
    push(launch) {
      q.push({ id: ++nextId, launch, lease: null });
    },
    /** 아무도 들고 있지 않은 요청이 있나(Rust has_unleased — 재오픈 판단), */
    hasUnleased() {
      return q.some((x) => !x.lease);
    },
  };
}

/**
 * 창 흉내 — 실제 창과 같은 시작 판정 사용
 *
 * 사용자 버튼은 canStart(= !busy)를 보고, 조율자가 부르는 자동 시작은 보지 않는다
 * 하나로 두면 자동 시작이 자기 자신의 taking 에 막혀 무동작 마감
 *
 * apply = 성패 반환, 실제 applyPlan 과 동일, 시작 실패를 성공으로
 * 돌려주면 요청이 마감돼 사용자가 요청한 압축이 사라진다
 */
function makeWindow() {
  const w = {
    phase: "form",
    inputs: [],
    jobBusy: false,
    taking: false,
    applied: [],
    jobs: [],
    /** 참이면 작업 등록 실패(job_busy 흉내) */
    startBlocked: false,
  };
  w.canRun = () => w.inputs.length > 0;
  w.canStart = () => w.canRun() && !(w.taking || w.jobBusy);
  /** 실제 컴포넌트의 onStart({internal}) 과 같은 골격(불변 snapshot 포함), */
  w.start = (opts) => {
    if (opts?.internal ? !w.canRun() : !w.canStart()) return false;
    const req = { output: w.output, inputs: [...w.inputs] };
    if (w.startBlocked) return false;
    w.jobs.push(req);
    w.phase = "running";
    w.jobBusy = true;
    return true;
  };
  w.host = {
    getState: () => ({ phase: w.phase, inputs: w.inputs, busy: w.jobBusy }),
    async apply(plan) {
      w.applied.push(plan);
      if (plan.reset) w.inputs = [];
      w.inputs = [...w.inputs, ...plan.paths];
      w.output = plan.output;
      if (plan.mode !== "form") return w.start({ internal: true });
      return true;
    },
  };
  /** 작업이 끝났다(창이 한가해진다), */
  w.finish = () => {
    w.jobBusy = false;
    w.phase = "done";
  };
  return w;
}

function coordFor(q, w, onError) {
  return createCoordinator({
    api: q.api,
    host: w.host,
    onError,
    onState: (s) => {
      w.taking = s !== "idle";
    },
  });
}

function autoLaunch(name) {
  return {
    inputs: [`C:/${name}.txt`],
    format: "zip",
    output: `C:/${name}.zip`,
    autoStart: true,
    batch: [],
  };
}
function plainLaunch(...paths) {
  return { inputs: paths, format: null, output: null, autoStart: false, batch: [] };
}

// ─────────────────────── 정상 흐름 ───────────────────────

test("자동 시작은 조율자 자신의 taking 에 막히지 않는다", async () => {
  // 사용자 버튼과 같은 판정을 쓰면 applying 중의 taking=true 때문에 아무것도 시작하지
  // 않고 성공으로 마감 — 탐색기 즉시 zip 이 폼만 채우고 종료
  const q = makeQueue([autoLaunch("A")]);
  const w = makeWindow();
  const coord = coordFor(q, w);

  coord.signal();
  await coord.settled();
  assert.equal(w.jobs.length, 1, "즉시 압축이 시작되지 않았다");
  assert.equal(w.jobs[0].output, "C:/A.zip");
  assert.equal(q.q.length, 0, "마감되지 않았다");
});

test("사용자 버튼은 회수 중 막힌다", () => {
  const w = makeWindow();
  w.inputs = ["C:/A.txt"];
  w.taking = true;
  assert.equal(w.canStart(), false, "회수 중인데 시작할 수 있다");
  assert.equal(w.start(), false);
  w.taking = false;
  assert.equal(w.canStart(), true);
});

test("넘김은 적용보다 먼저 알린다", async () => {
  // 순서가 뒤집히면, 넘김 알림이 실패한 순간 창이 죽었을 때 이미 압축을 시작한 요청이
  // 되돌아가 이중 압축
  const q = makeQueue([autoLaunch("A")]);
  const w = makeWindow();
  const order = [];
  const realDispatch = q.api.dispatch;
  q.api.dispatch = async (...a) => {
    order.push("dispatch");
    return realDispatch(...a);
  };
  const realApply = w.host.apply;
  w.host.apply = async (p) => {
    order.push("apply");
    return realApply(p);
  };
  const coord = coordFor(q, w);
  coord.signal();
  await coord.settled();
  assert.deepEqual(order, ["dispatch", "apply"], "적용이 넘김보다 먼저 갔다");
});

test("회수 중 도착한 다음 신호가 사라지지 않는다", async () => {
  // A 응답 대기 중 B 도착 시, 응답의 more=false 가 그 신호를 덮으면 안 됨
  const q = makeQueue([autoLaunch("A")]);
  const w = makeWindow();
  const coord = coordFor(q, w);

  const realLease = q.api.lease;
  q.api.lease = async () => {
    const r = await realLease(); // 이 시점에 큐에는 A 하나뿐 → more=false
    q.push(autoLaunch("B")); // 응답 직후 B 도착
    coord.signal();
    return r;
  };

  coord.signal();
  await coord.settled();
  q.api.lease = realLease;

  assert.equal(w.jobs.length, 1, "A 가 처리되지 않았다");
  assert.equal(coord.isPending(), true, "B 신호가 사라졌다");
});

// ─────────────────── 넘김(dispatch) 실패 ───────────────────

test("넘김이 실행 전에 실패하면 아무것도 하지 않고 처음부터 다시 한다", async () => {
  // 서버에 닿지 않았으므로 되돌아가도 안전하다, 여기서 적용까지 해 버리면 그 요청은
  // 서버 눈에 미전달이라, 창 사망 순간 되돌아가 이중 압축
  const q = makeQueue([autoLaunch("A")]);
  const w = makeWindow();
  const errs = [];
  const coord = coordFor(q, w, (e) => errs.push(e));

  q.failOnce("dispatch", "before");
  coord.signal();
  await coord.settled();

  assert.equal(w.jobs.length, 0, "넘기지도 못했는데 압축을 시작했다");
  assert.equal(q.q.length, 1, "요청이 사라졌다");
  assert.equal(q.q[0].lease.dispatched, false, "서버가 넘긴 것으로 알고 있다");
  assert.equal(errs.length, 1);

  // 다음 신호에 처음부터 재시도 — 정확히 1회만 압축
  coord.signal();
  await coord.settled();
  assert.equal(w.jobs.length, 1, "다시 시도하지 않았다");
  assert.equal(q.q.length, 0, "마감되지 않았다");
});

test("넘김 응답만 유실되면 재시도가 already 로 이어진다", async () => {
  // 서버는 이미 넘긴 것으로 안다, 재시도를 실패로 보면 요청이 영영 멈춘다
  const q = makeQueue([autoLaunch("A")]);
  const w = makeWindow();
  const coord = coordFor(q, w, () => {});

  q.failOnce("dispatch", "after");
  coord.signal();
  await coord.settled();
  assert.equal(q.q[0].lease.dispatched, true, "서버 쪽 효과가 남지 않았다");
  assert.equal(w.jobs.length, 0);

  coord.signal();
  await coord.settled();
  assert.equal(w.jobs.length, 1, "재시도가 이어지지 않았다");
  assert.equal(q.q.length, 0, "마감되지 않았다");
});

test("넘김이 stale 이면 들고 있던 것을 놓는다", async () => {
  const q = makeQueue([autoLaunch("A")]);
  const w = makeWindow();
  const errs = [];
  const coord = coordFor(q, w, (e) => errs.push(e));

  q.failOnce("dispatch", "value", "stale");
  coord.signal();
  await coord.settled();

  assert.equal(coord.heldStage(), null, "남의 요청을 붙들고 있다");
  assert.equal(w.jobs.length, 0, "남의 요청을 압축했다");
  assert.equal(errs.length, 1);
});

// ───────────────────── 마감(ack) 실패 ─────────────────────

test("마감이 pop 전에 실패하면 마감만 다시 한다(두 번 압축하지 않는다)", async () => {
  const q = makeQueue([autoLaunch("A")]);
  const w = makeWindow();
  const coord = coordFor(q, w, () => {});

  q.failOnce("ack", "before");
  coord.signal();
  await coord.settled();
  assert.equal(w.jobs.length, 1, "압축은 됐어야 한다");
  assert.equal(q.q.length, 1, "마감되지 않은 요청이 남아야 한다");
  assert.equal(coord.heldStage(), "applied", "적용까지 간 상태를 잃었다");

  w.finish();
  coord.poke();
  await coord.settled();
  assert.equal(w.jobs.length, 1, "같은 요청을 두 번 압축했다");
  assert.equal(q.q.length, 0, "마감되지 않았다");
});

test("마감 응답만 유실되면 재시도가 already 로 끝난다", async () => {
  // 서버는 이미 뺐다, 재시도의 stale 을 오류로 보면 창이 정상 처리된 요청을 잃은 줄 안다
  const q = makeQueue([autoLaunch("A")]);
  const w = makeWindow();
  const errs = [];
  const coord = coordFor(q, w, (e) => errs.push(e));

  q.failOnce("ack", "after");
  coord.signal();
  await coord.settled();
  assert.equal(q.q.length, 0, "서버 쪽 효과가 남지 않았다");
  assert.equal(errs.length, 1, "유실 자체는 알려야 한다");

  // 재시도는 오류가 아니다, already 를 stale 과 같이 보면 정상 처리된 요청을
  // 유실로 오인해 사용자에게 실패 통지
  w.finish();
  coord.poke();
  await coord.settled();
  assert.equal(coord.heldStage(), null, "마감을 마치지 못했다");
  assert.equal(errs.length, 1, "정상 처리된 요청을 오류로 봤다");
  assert.equal(w.jobs.length, 1, "두 번 압축했다");
});

test("마감이 stale 이면 한 번만 알리고 멈춘다", async () => {
  const q = makeQueue([autoLaunch("A")]);
  const w = makeWindow();
  const errs = [];
  const coord = coordFor(q, w, (e) => errs.push(e));

  q.failOnce("ack", "value", "stale");
  coord.signal();
  await coord.settled();

  assert.equal(errs.length, 1, "거절을 알리지 않았다");
  assert.equal(coord.heldStage(), null, "거절당한 것을 붙들고 있다");
  const before = q.calls.ack;
  coord.signal();
  await coord.settled();
  assert.ok(q.calls.ack >= before, "재시도 자체는 막지 않는다");
});

// ─────────────────── 창 세대 경합 ───────────────────

test("넘기기 전에 죽은 창의 요청은 새 창이 다시 받는다", async () => {
  const q = makeQueue([autoLaunch("A")]);
  const w1 = makeWindow();
  const c1 = coordFor(q, w1, () => {});

  // 적용 직전(넘김 응답 대기 중)에 창이 죽는다
  q.failOnce("dispatch", "before");
  c1.signal();
  await c1.settled();
  assert.equal(w1.jobs.length, 0);

  q.windowDied();
  assert.equal(q.hasUnleased(), true, "새 창을 열 이유가 사라졌다");

  // 새 창(세션이 다르다)이 이어받는다
  q.newWindow();
  const w2 = makeWindow();
  const c2 = coordFor(q, w2, () => {});
  c2.signal();
  await c2.settled();
  assert.equal(w2.jobs.length, 1, "새 창이 이어받지 못했다");
  assert.equal(q.q.length, 0);
});

test("넘긴 뒤 죽은 창의 요청은 다시 실행되지 않는다", async () => {
  const q = makeQueue([autoLaunch("A")]);
  const w1 = makeWindow();
  const c1 = coordFor(q, w1, () => {});

  q.failOnce("ack", "before"); // 압축은 시작했고 마감만 못 했다
  c1.signal();
  await c1.settled();
  assert.equal(w1.jobs.length, 1);

  q.windowDied();
  assert.equal(q.q.length, 0, "이미 압축한 요청이 큐에 되돌아왔다");
  assert.equal(q.hasUnleased(), false, "그것 때문에 창을 다시 연다");
});

test("죽은 창의 지연 호출이 새 창의 요청을 건드리지 못한다", async () => {
  const q = makeQueue([autoLaunch("A")]);
  const w1 = makeWindow();
  const c1 = coordFor(q, w1, () => {});

  q.failOnce("dispatch", "before");
  c1.signal();
  await c1.settled();
  const stale = { ...q.q[0], gen: q.q[0].lease.gen };

  q.windowDied();
  q.newWindow();
  const w2 = makeWindow();
  const c2 = coordFor(q, w2, () => {});
  c2.signal();
  await c2.settled();
  assert.equal(w2.jobs.length, 1);

  // 그제야 도착한 W1 의 지연 호출 — 새 창의 것 미간섭 필요
  assert.equal(await q.api.dispatch(stale.id, stale.gen), "stale");
  assert.equal(await q.api.ack(stale.id, stale.gen), "stale");
  assert.equal(w2.jobs.length, 1, "지연 호출이 무언가를 더 만들었다");
});

test("같은 창의 회수 재시도는 같은 요청을 다시 받는다", async () => {
  const q = makeQueue([autoLaunch("A")]);
  const w = makeWindow();
  const coord = coordFor(q, w, () => {});

  q.failOnce("lease", "after"); // 서버는 빌려줬는데 응답만 잃었다
  coord.signal();
  await coord.settled();
  assert.equal(w.jobs.length, 0);
  assert.equal(q.q[0].lease !== null, true, "서버 쪽 대여가 남지 않았다");

  coord.signal();
  await coord.settled();
  assert.equal(w.jobs.length, 1, "같은 창이 다시 받지 못했다");
  assert.equal(q.q.length, 0);
});

test("살아 있는 다른 창의 요청은 가져가지 않는다", async () => {
  const q = makeQueue([autoLaunch("A")]);
  // W1 보유 상태 생성(응답 유실로 창은 미인지)
  q.failOnce("lease", "after");
  const w1 = makeWindow();
  const c1 = coordFor(q, w1, () => {});
  c1.signal();
  await c1.settled();

  // 다른 창이 같은 큐 조회 — 탈취 시 두 창이 같은 것을 압축
  q.newWindow();
  const take = await q.api.lease();
  assert.equal(take.launch, null, "남이 들고 있는 것을 가져갔다");
});

// ─────────────────── 작업 등록 실패 ───────────────────

test("작업을 시작하지 못하면 성공으로 마감하지 않는다", async () => {
  // 재오픈 직후 앞 작업 미퇴장이면 job_busy 발생, 그것을 적용 성공으로
  // 마감하면 사용자가 요청한 압축이 아무 일도 없이 사라진다
  const q = makeQueue([autoLaunch("A")]);
  const w = makeWindow();
  w.startBlocked = true;
  const errs = [];
  const coord = coordFor(q, w, (e) => errs.push(e));

  coord.signal();
  await coord.settled();
  assert.equal(w.jobs.length, 0);
  assert.equal(q.q.length, 1, "시작하지 못한 요청이 마감됐다");
  assert.equal(errs.length, 1, "실패를 알리지 않았다");

  // 앞 작업 퇴장 후 속행(넘김은 이미 전달, 재실행 없음)
  w.startBlocked = false;
  coord.poke();
  await coord.settled();
  assert.equal(w.jobs.length, 1, "물러난 뒤에도 시작하지 않았다");
  assert.equal(q.q.length, 0);
});

// ─────────────────── 폼, 타이밍 ───────────────────

test("적용 중 도착한 입력이 실행 인자를 바꾸지 못한다", async () => {
  // await 사이에 사용자가 파일을 더 떨어뜨려도 그 요청은 자기 것만 압축
  const q = makeQueue([autoLaunch("A")]);
  const w = makeWindow();
  const coord = coordFor(q, w);

  const realStart = w.start;
  w.start = (opts) => {
    const r = realStart(opts);
    w.inputs = [...w.inputs, "C:/늦게떨어진.txt"]; // 시작 직후 추가
    return r;
  };

  coord.signal();
  await coord.settled();
  assert.deepEqual(w.jobs[0].inputs, ["C:/A.txt"], "실행 인자가 나중 입력에 오염됐다");
});

test("폼에 입력이 있으면 독립 요청을 꺼내지 않고 물어만 본다", async () => {
  const q = makeQueue([autoLaunch("A")]);
  const w = makeWindow();
  w.inputs = ["C:/사용자가모아둔.txt"];
  const coord = coordFor(q, w);

  coord.signal();
  await coord.settled();
  assert.equal(q.calls.lease, 0, "폼이 지저분한데 꺼냈다");
  assert.equal(coord.isBlocked(), true);
  assert.equal(q.q[0].lease, null, "소유권이 큐를 떠났다");

  // 막힌 동안 반응문이 여러 번 돌아도 조회를 폭주시키지 않는다
  const peeks = q.calls.peek;
  coord.poke();
  coord.poke();
  await coord.settled();
  assert.equal(q.calls.peek, peeks, "막힌 상태에서 다시 조회했다");

  // 폼 정리 후 회수
  w.inputs = [];
  w.phase = "done";
  coord.poke();
  await coord.settled();
  assert.equal(w.jobs.length, 1, "폼이 정리됐는데 이어서 하지 않았다");
});

test("회수 중 사용자가 시작하면 그 작업이 끝난 뒤에 적용한다", async () => {
  const q = makeQueue([autoLaunch("A")]);
  const w = makeWindow();
  const coord = coordFor(q, w, () => {});

  const realLease = q.api.lease;
  q.api.lease = async () => {
    const r = await realLease();
    // 응답을 기다리는 사이에 사용자가 자기 목록으로 압축을 시작했다
    w.inputs = ["C:/사용자.txt"];
    w.jobBusy = true;
    w.phase = "running";
    w.jobs.push({ output: "C:/사용자.zip", inputs: ["C:/사용자.txt"] });
    return r;
  };
  coord.signal();
  await coord.settled();
  q.api.lease = realLease;

  assert.equal(w.jobs.length, 1, "진행 중인데 새 작업을 시작했다");
  assert.equal(q.q.length, 1, "요청이 사라졌다");

  w.finish();
  w.inputs = [];
  coord.poke();
  await coord.settled();
  assert.equal(w.jobs.length, 2, "한가해진 뒤에도 처리하지 않았다");
  assert.equal(w.jobs[1].output, "C:/A.zip");
});

test("IPC 실패는 요청을 보존하고 무한 재시도하지 않는다", async () => {
  const q = makeQueue([autoLaunch("A")]);
  const w = makeWindow();
  const errs = [];
  const coord = coordFor(q, w, (e) => errs.push(e));

  q.failOnce("lease", "before");
  coord.signal();
  await coord.settled();
  assert.equal(q.q.length, 1, "실패했는데 요청이 사라졌다");
  assert.equal(errs.length, 1);

  // 표시 없이 되돌아가면 반응문이 곧바로 다시 돌아 lease 를 끝없이 부른다
  const leases = q.calls.lease;
  await coord.settled();
  assert.equal(q.calls.lease, leases, "실패 직후 스스로 다시 걸었다");

  coord.signal();
  await coord.settled();
  assert.equal(w.jobs.length, 1, "새 신호에도 다시 하지 않았다");
});

test("일반 요청 뒤의 독립 요청은 순서대로 처리된다", async () => {
  const q = makeQueue([plainLaunch("C:/p1.txt"), autoLaunch("B")]);
  const w = makeWindow();
  const coord = coordFor(q, w, () => {});

  coord.signal();
  await coord.settled();
  // 일반 요청 = 목록 추가만 → 폼이 지저분해져 독립 요청 보류
  assert.deepEqual(w.inputs, ["C:/p1.txt"]);
  assert.equal(coord.isBlocked(), true, "독립 요청을 곧바로 꺼냈다");

  // 사용자가 그 목록으로 압축을 시작하고 끝냈다
  w.start();
  w.finish();
  w.inputs = [];
  coord.poke();
  await coord.settled();
  assert.equal(w.jobs.length, 2, "미뤄 둔 요청이 처리되지 않았다");
  assert.equal(w.jobs[1].output, "C:/B.zip");
  assert.equal(q.q.length, 0);
});

test("연달아 온 독립 요청은 하나씩 다 처리된다", async () => {
  const q = makeQueue([autoLaunch("A"), autoLaunch("B")]);
  const w = makeWindow();
  const coord = coordFor(q, w, () => {});

  coord.signal();
  await coord.settled();
  assert.equal(w.jobs.length, 1, "첫 요청이 시작되지 않았다");
  assert.equal(coord.isPending(), true, "남은 요청을 잊었다");

  w.finish();
  coord.poke();
  await coord.settled();
  assert.equal(w.jobs.length, 2, "둘째 요청이 처리되지 않았다");
  assert.deepEqual(
    w.jobs.map((j) => j.output),
    ["C:/A.zip", "C:/B.zip"],
  );
  assert.equal(q.q.length, 0);
});
