# ZipMania CSS 스킨 제작 안내

ZipMania 스킨은 기존 앱의 HTML 구조와 레이아웃을 그대로 사용하고, CSS로
색상·글꼴·테두리·그림자·배경 이미지를 변경하는 방식입니다.

스킨에서는 Svelte 컴포넌트, 버튼 구성, 이벤트 연결, 파일 목록 구조를 변경하지
않습니다. 따라서 스킨 제작자는 앱 소스나 미리보기 번들을 수정할 필요가 없습니다.

## 기본 구조

```text
skin/default/
├─ skin.css          # 모든 색상·그림자·배경 이미지 설정
├─ images/           # 선택 사항: 배경 이미지 저장
├─ preview/
│  ├─ preview.js     # 실제 앱 컴포넌트 미리보기 번들
│  └─ style.css      # 실제 앱 컴포넌트 기본 스타일
└─ index.html        # Chrome에서 직접 여는 테스트 화면
```

## 바로 테스트하기

`skin/default/index.html`을 Chrome에서 직접 엽니다. 개발 서버는 필요하지
않습니다. 상단에서 `파일 열림`과 `파일 없음`, 시스템·라이트·다크 테마를
전환할 수 있습니다.

`skin.css` 또는 이미지 파일을 수정한 뒤 Chrome을 새로고침하면
바로 반영됩니다.

```text
CSS 또는 이미지 수정 → Chrome 새로고침
실제 앱 배포본 갱신  → npm run build
```

`npm run skin`은 앱 개발자가 Svelte 컴포넌트를 수정했을 때 미리보기 번들을
갱신하는 명령입니다. 일반 스킨 제작자는 실행하지 않아도 됩니다.

## 색상 변경

`skin.css`의 변수를 수정합니다.

```css
:root[data-skin="default"] {
  --bg: #ffffff;
  --surface: #f5f5f7;
  --border: #e0e0e0;
  --text: #1a1a1a;
  --text-muted: #888888;
  --accent: #3b82f6;
  --accent-contrast: #ffffff;
}
```

주요 변수는 다음과 같습니다.

| 변수 | 적용 영역 |
|---|---|
| `--bg` | 앱과 창의 기본 배경 |
| `--surface` | 툴바, 상태줄, 헤더 |
| `--btn-bg` | 버튼과 입력 요소 |
| `--border` | 구분선과 테두리 |
| `--text`, `--text-muted` | 기본·보조 글자 |
| `--accent`, `--accent-contrast` | 선택 상태와 강조색 |
| `--tree-bg`, `--preview-bg` | 트리와 미리보기 영역 |
| `--ok-*`, `--warn-*`, `--alert-*` | 결과와 오류 상태 |
| `--dialog-shadow`, `--toast-shadow` | 창과 알림 그림자 |

라이트·다크 값을 각각 제공하려면 `skin.css`에 있는
`prefers-color-scheme` 및 `data-theme="dark"` 블록도 함께 수정합니다.

## 이 스킨의 툴바 — 여름 바다

`ocean-test` 툴바는 단색 대신 바다 그라데이션과 물결 한 줄을 씁니다. 색은
`--sea-*` 변수 다섯 개로만 정해지므로, 이 값만 바꾸면 툴바 전체 톤이
따라옵니다.

| 변수 | 쓰이는 곳 |
|---|---|
| `--sea-deep` | 툴바 위쪽(수평선) 색 |
| `--sea-mid` | 툴바 중간 색 |
| `--sea-shallow` | 툴바 아래쪽(모래에 가까운) 색 |
| `--sea-foam` | 툴바 아래 테두리(물거품) |
| `--sea-ink` | 아이콘 안쪽 획, 눌린 버튼의 글자색 |

툴바 버튼은 바다 위에 놓이므로 글자와 아이콘이 흰색이고, 누르면 흰
타일로 반전됩니다. 아이콘 안쪽 획은 앱이 인라인 SVG 에서
`var(--accent-contrast)` 를 쓰기 때문에, 툴바 범위에서만 그 변수를
`--sea-ink` 로 덮어 흰 아이콘에 파낸 것처럼 보이게 했습니다.
`--skin-toolbar-background` 로 이미지를 지정하면 물결 위에 얹힙니다.

## 배경 이미지 변경

이미지를 `skin/<이름>/images/`에 넣고 `skin.css`에서 연결합니다.

```css
:root[data-skin="default"] {
  --skin-app-background: url("./images/app.webp");
  --skin-toolbar-background: url("./images/toolbar.png");
  --skin-sidebar-background: none;
  --skin-content-background: none;
  --skin-empty-background: url("./images/empty.svg");
  --skin-dialog-background: none;
  --skin-background-size: cover;
  --skin-background-position: center;
}
```

배경 이미지를 사용하지 않는 항목은 `none`으로 둡니다. 반복이나 영역별 크기가
필요하면 `skin.css`에서 `background-repeat`, `background-size` 등을
해당 `data-ui` 선택자에 추가합니다.

## 새 스킨 만들기

1. `skin/default` 폴더 전체를 `skin/<새 이름>`으로 복사합니다.
2. `skin.css`의 `data-skin="default"`를 **폴더 이름과 똑같이** 바꿉니다.
   미리보기는 폴더 이름으로 `data-skin` 을 정하므로, 어긋나면 변수가 하나도
   적용되지 않아 화면이 통째로 깨집니다.
3. 색상과 배경 이미지를 수정합니다.
4. 복사한 `index.html`을 Chrome에서 열어 확인합니다.
5. 실제 앱 배포본은 `npm run build`로 갱신합니다.

앱은 `skin/*/skin.css`를 빌드할 때 자동으로 찾습니다. `skin.css` 파일명은
변경하면 안 됩니다.

## CSS 적용 대상

`skin.css`에서 사용할 수 있는 안정적인 선택자는 다음과 같습니다.

```css
[data-ui="toolbar"]
[data-ui="app-main"]
[data-ui="workspace"]
[data-ui="sidebar"]
[data-ui="folder-tree"]
[data-ui="preview-pane"]
[data-ui="file-table"]
[data-ui="empty-state"]
[data-ui="progress-panel"]
[data-ui="status-bar"]
[data-ui="dialog-card"]
[data-ui="compress-window"]
[data-ui="extract-window"]
[data-ui="settings-window"]
```

스킨은 외형만 담당합니다. 앱의 요소를 추가·삭제하거나 순서를 바꾸고 기능을
연결하는 작업은 스킨 범위가 아니라 애플리케이션 개발 작업입니다.
