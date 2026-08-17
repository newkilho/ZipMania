// 경량 i18n, 의존성 없이 Svelte 스토어, settings.toml 의 language 가 유일 소스, system 이면 시스템 언어 감지
//
// 사전은 생성물 locales/strings.json 하나, 정본은 zipmania-i18n 의 strings.rs
//
// LANGUAGES 지원 언어(코드 + 자국어 표기), 선택 목록 순서
// locale 현재 언어 코드 스토어
// t $t('키', { 보간값 }) 로 번역 문자열을 돌려주는 파생 스토어
// errText 백엔드 오류 코드(ZipManiaError.code)를 번역 메시지로 바꾼다
// applyLanguage 설정의 언어 pref("system" 또는 언어 코드)를 현재 창에 반영
import { writable, derived } from "svelte/store";
import strings from "../locales/strings.json";

/** 지원 언어, 코드와 선택 목록에 쓰는 자국어 표기, */
export const LANGUAGES = strings.langs;

/** 언어 코드 → 평면 키 사전, */
const DICTS = strings.strings;

/** 누락 키를 채우는 참조(완전) 언어, 정본의 첫 언어, */
const REFERENCE = LANGUAGES[0].code;
/** 기본 언어 — 시스템 언어 미감지 시 영어로 시작 */
const DEFAULT = "en";

/**
 * 시스템, 브라우저 언어 → 언어 코드, 앞 두 자만 사용(ko-KR = ko, zh-TW = zh), 지원 언어면 그것, 아니면 영어
 */
function detectSystemLocale() {
  const langs =
    (typeof navigator !== "undefined" &&
      (navigator.languages || [navigator.language])) ||
    [];
  for (const raw of langs) {
    if (!raw) continue;
    const two = String(raw).toLowerCase().slice(0, 2);
    if (DICTS[two]) return two;
  }
  return DEFAULT;
}

// 시작 기본값 = 시스템 감지, 실제 언어는 main.js 가 applyLanguage 로 덮는다(설정이 유일 소스)
export const locale = writable(detectSystemLocale());

// 언어를 <html lang> 에 반영(접근성/폰트 힌트)
locale.subscribe((code) => {
  if (typeof document !== "undefined") {
    document.documentElement.lang = code;
  }
});

/**
 * 설정의 언어 pref 를 현재 창에 적용
 * @param {string} pref 언어 코드, "system" 이거나 지원하지 않는 값이면 시스템 언어 감지
 */
export function applyLanguage(pref) {
  const code = pref && DICTS[pref] ? pref : detectSystemLocale();
  locale.set(code);
}

/** 사전에서 평면 키를 찾는다, */
function lookup(dict, key) {
  if (dict == null) return undefined;
  return Object.prototype.hasOwnProperty.call(dict, key) ? dict[key] : undefined;
}

/** {name} 자리표시자를 params 로 치환, */
function interpolate(str, params) {
  if (!params) return str;
  return str.replace(/\{(\w+)\}/g, (m, k) =>
    k in params ? String(params[k]) : m,
  );
}

/**
 * 현재 언어에 묶인 번역 함수, $t(키) 또는 $t(키, { pct: 42 }), 없는 키는 참조 언어 → 키 문자열
 */
export const t = derived(locale, ($locale) => {
  const dict = DICTS[$locale] || DICTS[REFERENCE];
  const ref = DICTS[REFERENCE];
  return (key, params) => {
    let val = lookup(dict, key);
    if (val == null) val = lookup(ref, key);
    if (val == null) return key;
    return interpolate(val, params);
  };
});

/**
 * 백엔드 오류 → 번역 메시지, errors.<code> 가 있으면 그 번역, 없으면 fallback
 * @param {(k:string, p?:object)=>string} tr 번역 함수($t 값 또는 get(t))
 * @param {string} code ZipManiaError.code
 * @param {string} fallback 매핑이 없을 때 쓸 메시지
 */
export function errText(tr, code, fallback) {
  const key = "errors." + (code || "unknown");
  const msg = tr(key);
  if (msg !== key) return msg; // 번역이 존재
  return fallback || tr("errors.unknown");
}
