// 스킨 CSS 의 :root 를 그 파일이 든 폴더 이름으로 한정
//
// 스킨 이름 = 폴더 이름 하나, CSS 안에는 이름을 적지 않는다
// 미리보기 페이지는 자기 skin.css 하나만 읽으므로 한정 없이도 맞고,
// 앱은 모든 스킨을 한 번에 싣기 때문에 여기서 [data-skin] 을 붙인다
// 이미 붙어 있는 [data-skin=...] 도 폴더 이름으로 덮는다(멱등)

const SKIN_CSS = /[\/]skin[\/]([^\/]+)[\/]skin\.css$/;
const ROOT = /:root(\[data-skin=(?:"[^"]*"|'[^']*'|[^\]]*)\])?/g;

export default function skinScope() {
  return {
    name: "zipmania-skin-scope",
    // 내장 CSS 플러그인보다 먼저 — 원본 CSS 텍스트를 봐야 한다
    enforce: "pre",
    transform(code, id) {
      const m = id.split("?")[0].match(SKIN_CSS);
      if (!m) return null;
      const scoped = code.replace(ROOT, `:root[data-skin="${m[1]}"]`);
      return scoped === code ? null : { code: scoped, map: null };
    },
  };
}
