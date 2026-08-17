import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// @tauri-apps/cli 가 설정하는 환경변수, 데스크톱 개발 시 사용
const host = process.env.TAURI_DEV_HOST;

// https://vitejs.dev/config/
export default defineConfig({
  plugins: [svelte()],

  // Tauri 감시용 고정 포트
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // src-tauri 폴더는 Vite 감시에서 제외 (Rust 는 cargo 가 감시)
      ignored: ["**/src-tauri/**"],
    },
  },
});
