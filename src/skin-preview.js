import "./styles/global.css";
import SkinPreview from "./SkinPreview.svelte";

const parts = location.pathname.replace(/\\/g, "/").split("/").filter(Boolean);
const skinName = decodeURIComponent(parts.at(-2) || "default");
document.documentElement.dataset.skin = skinName;

const target = document.getElementById("app");
new SkinPreview({ target });

// 미리보기에서 실제 백엔드 동작 미실행 + hover/focus 디자인 확인
target.addEventListener(
  "click",
  (event) => {
    event.preventDefault();
    event.stopPropagation();
  },
  true,
);
