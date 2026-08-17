import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

export default {
  // JavaScript 전용 — TypeScript 전처리 없음
  preprocess: vitePreprocess(),
};
