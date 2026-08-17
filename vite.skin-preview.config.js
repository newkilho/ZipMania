import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

export default defineConfig({
  plugins: [svelte()],
  build: {
    outDir: "skin/default/preview",
    emptyOutDir: true,
    lib: {
      entry: "src/skin-preview.js",
      name: "ZipManiaSkinPreview",
      formats: ["iife"],
      fileName: () => "preview.js",
      cssFileName: "preview",
    },
  },
});
