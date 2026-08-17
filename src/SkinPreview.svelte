<script>
  import { onMount, onDestroy } from "svelte";
  import App from "./App.svelte";
  import { applyLanguage } from "./lib/i18n.js";
  import {
    archivePath,
    entries,
    currentPath,
    flatView,
    selectedPaths,
    previewImage,
    activeJob,
    jobResult,
    uiError,
  } from "./lib/stores.js";

  const sampleEntries = [
    { path: "Documents", size: 0, packedSize: 0, modified: "2026-08-02 14:30:00", isDir: true },
    { path: "Documents/Project", size: 0, packedSize: 0, modified: "2026-08-02 14:30:00", isDir: true },
    { path: "Images", size: 0, packedSize: 0, modified: "2026-08-01 18:04:00", isDir: true },
    { path: "README.txt", size: 12_288, packedSize: 4_096, modified: "2026-08-02 14:21:00", isDir: false },
    { path: "preview.png", size: 843_776, packedSize: 808_960, modified: "2026-08-01 18:04:00", isDir: false },
    { path: "setup.exe", size: 2_516_582, packedSize: 1_887_437, modified: "2026-07-28 09:42:00", isDir: false },
  ];

  function resetTransientState() {
    currentPath.set("");
    flatView.set(false);
    previewImage.set(null);
    activeJob.set(null);
    jobResult.set(null);
    uiError.set(null);
  }

  function showArchive() {
    resetTransientState();
    archivePath.set("C:\\Samples\\sample.zip");
    entries.set(sampleEntries);
    selectedPaths.set(new Set(["README.txt"]));
  }

  function showEmpty() {
    resetTransientState();
    archivePath.set(null);
    entries.set([]);
    selectedPaths.set(new Set());
  }

  function onPreviewMode(event) {
    if (event.detail === "empty") showEmpty();
    else showArchive();
  }

  applyLanguage("ko");
  if (new URLSearchParams(location.search).get("mode") === "empty") showEmpty();
  else showArchive();

  onMount(() => window.addEventListener("zipmania-preview-mode", onPreviewMode));
  onDestroy(() => window.removeEventListener("zipmania-preview-mode", onPreviewMode));
</script>

<App preview />
