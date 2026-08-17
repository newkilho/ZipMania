// Svelte <script> 블록에서 선언 없이 쓰인 식별자를 찾는다 — Vite 는 통과시키고 런타임에야
// ReferenceError 발생, 사용법: node scripts/check-refs.mjs (문제 시 종료 코드 1)

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import * as acorn from "acorn";

/** 브라우저, 표준 전역 + Svelte 템플릿에서 오는 것들, */
const GLOBALS = new Set([
  "window", "document", "console", "setTimeout", "clearTimeout", "setInterval", "clearInterval",
  "requestAnimationFrame", "cancelAnimationFrame", "fetch", "navigator", "location", "history",
  "localStorage", "sessionStorage", "alert", "confirm", "prompt", "structuredClone",
  "Promise", "Object", "Array", "String", "Number", "Boolean", "Math", "JSON", "Date", "RegExp",
  "Map", "Set", "WeakMap", "WeakSet", "Symbol", "Error", "TypeError", "RangeError", "Intl",
  "parseInt", "parseFloat", "isNaN", "isFinite", "encodeURIComponent", "decodeURIComponent",
  "Uint8Array", "ArrayBuffer", "Blob", "File", "FileReader", "URL", "URLSearchParams",
  "globalThis", "undefined", "NaN", "Infinity", "queueMicrotask", "AbortController", "Event",
]);

function walkFiles(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walkFiles(p, out);
    else if (name.endsWith(".svelte")) out.push(p);
  }
  return out;
}

/** 선언된 이름을 모은다(변수, 함수, 클래스, import, 매개변수, 구조분해), */
function collectDeclared(node, declared) {
  const bindPattern = (pat) => {
    if (!pat) return;
    switch (pat.type) {
      case "Identifier":
        declared.add(pat.name);
        break;
      case "ObjectPattern":
        for (const p of pat.properties) bindPattern(p.value ?? p.argument);
        break;
      case "ArrayPattern":
        for (const el of pat.elements) bindPattern(el);
        break;
      case "AssignmentPattern":
        bindPattern(pat.left);
        break;
      case "RestElement":
        bindPattern(pat.argument);
        break;
    }
  };

  const visit = (n) => {
    if (!n || typeof n.type !== "string") return;
    switch (n.type) {
      case "VariableDeclarator":
        bindPattern(n.id);
        break;
      case "FunctionDeclaration":
      case "FunctionExpression":
      case "ArrowFunctionExpression":
        if (n.id) declared.add(n.id.name);
        for (const p of n.params) bindPattern(p);
        break;
      case "ClassDeclaration":
        if (n.id) declared.add(n.id.name);
        break;
      case "ImportDefaultSpecifier":
      case "ImportNamespaceSpecifier":
      case "ImportSpecifier":
        declared.add(n.local.name);
        break;
      case "CatchClause":
        bindPattern(n.param);
        break;
      case "LabeledStatement":
        // Svelte 반응형 선언: $: x = ... / $: ({a, b} = ...) → x, a, b 선언
        if (n.label.name === "$" && n.body.type === "ExpressionStatement") {
          const ex = n.body.expression;
          if (ex.type === "AssignmentExpression") bindPattern(ex.left);
          if (ex.type === "SequenceExpression") {
            for (const e of ex.expressions) if (e.type === "AssignmentExpression") bindPattern(e.left);
          }
        }
        break;
    }
    for (const key of Object.keys(n)) {
      const v = n[key];
      if (Array.isArray(v)) v.forEach(visit);
      else if (v && typeof v.type === "string") visit(v);
    }
  };
  visit(node);
}

/** 참조된 식별자를 모은다(속성명, 객체 키는 제외), */
function collectUsed(node, used) {
  const visit = (n, parent, key) => {
    if (!n || typeof n.type !== "string") return;
    if (n.type === "Identifier") {
      const isProp = parent && parent.type === "MemberExpression" && key === "property" && !parent.computed;
      const isKey = parent && parent.type === "Property" && key === "key" && !parent.computed;
      const isLabel = parent && (parent.type === "LabeledStatement" || parent.type === "BreakStatement");
      // import { a as b } 의 a, export { x as y } 의 y 는 이 모듈의 참조가 아니다
      const isSpecifierName =
        parent &&
        (parent.type === "ImportSpecifier" || parent.type === "ExportSpecifier") &&
        key !== "local";
      if (!isProp && !isKey && !isLabel && !isSpecifierName) used.add(n.name);
      return;
    }
    for (const k of Object.keys(n)) {
      const v = n[k];
      if (Array.isArray(v)) v.forEach((c) => visit(c, n, k));
      else if (v && typeof v.type === "string") visit(v, n, k);
    }
  };
  visit(node, null, null);
}

let problems = 0;
for (const file of walkFiles("src")) {
  const src = readFileSync(file, "utf8");
  const open = src.indexOf("<script");
  if (open < 0) continue;
  const bodyStart = src.indexOf(">", open) + 1;
  const end = src.indexOf("</script>", bodyStart);
  if (end < 0) continue;
  // Svelte 의 $: 라벨 = 표준 문법 → acorn 이 그대로 파싱
  const code = src.slice(bodyStart, end);

  let ast;
  try {
    ast = acorn.parse(code, { ecmaVersion: "latest", sourceType: "module", allowAwaitOutsideFunction: true });
  } catch (e) {
    console.error(`[구문 오류] ${file}: ${e.message}`);
    problems++;
    continue;
  }

  const declared = new Set();
  const used = new Set();
  collectDeclared(ast, declared);
  collectUsed(ast, used);

  for (const name of used) {
    if (declared.has(name) || GLOBALS.has(name)) continue;
    // Svelte 스토어 자동 구독($foo) = foo 선언 시 정상
    if (name.startsWith("$") && declared.has(name.slice(1))) continue;
    console.error(`[미선언 참조] ${file}: ${name}`);
    problems++;
  }
}

if (problems > 0) {
  console.error(`\n미선언 참조 ${problems}건 — 런타임에 ReferenceError 가 난다.`);
  process.exit(1);
}
console.log("미선언 참조 없음.");
