// DOM 판정 유틸, 창 종류 무관

/**
 * 글자를 입력/편집하는 요소인지 — 그 자리에서는 웹뷰 기본 동작을 그대로 둔다
 * (Ctrl+A = 글자 전체 선택, 우클릭 = 잘라내기/붙여넣기 메뉴)
 * @param {EventTarget|null} el 이벤트 대상
 * @returns {boolean}
 */
export function isEditing(el) {
  if (!el || !el.tagName) return false;
  const tag = el.tagName;
  return (
    tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || el.isContentEditable === true
  );
}
