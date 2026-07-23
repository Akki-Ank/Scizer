// Plain vanilla JS, no bundler, no framework, no npm dependency: this
// webview only ever calls into Rust via `invoke` and renders the result.
// No compression logic lives here -- see ../src/commands.rs.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { open: openDialog } = window.__TAURI__.dialog;

const state = {
  filePath: null,
  fileName: null,
  fileSizeBytes: null,
  mode: "archive",
  convertMode: "image-format",
  convertSingleFile: null,
  convertFiles: [],
};

const el = (id) => document.getElementById(id);

const dropZone = el("drop-zone");
const workspace = el("workspace");
const dropOverlay = el("drop-overlay");
const fileHint = el("file-hint");
const fileNameEl = el("file-name");
const fileMetaEl = el("file-meta");
const progressCard = el("progress-card");
const progressFill = el("progress-fill");
const progressLabel = el("progress-label");
const resultCard = el("result-card");

function formatBytes(n) {
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = n / 1024;
  let unitIndex = 0;
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }
  return `${value.toFixed(value >= 10 ? 0 : 1)} ${units[unitIndex]}`;
}

function formatMs(ms) {
  return `${(ms / 1000).toFixed(1)}s`;
}

function suggestedOutputPath(inputPath, extension) {
  const dot = inputPath.lastIndexOf(".");
  const base = dot > inputPath.lastIndexOf("/") && dot > inputPath.lastIndexOf("\\")
    ? inputPath.slice(0, dot)
    : inputPath;
  return `${base}.${extension}`;
}

async function loadFile(path) {
  state.filePath = path;
  state.fileName = path.split(/[\\/]/).pop();

  fileNameEl.textContent = state.fileName;
  fileMetaEl.textContent = "detecting format…";
  dropZone.classList.add("hidden");
  workspace.classList.remove("hidden");
  resultCard.classList.add("hidden");
  progressCard.classList.add("hidden");

  try {
    const detected = await invoke("detect_format", { path });
    fileMetaEl.textContent = `${detected.format} (${detected.kind})`;
  } catch (err) {
    fileMetaEl.textContent = "format unknown";
  }
}

el("browse-btn").addEventListener("click", async () => {
  const selected = await openDialog({ multiple: false, directory: false });
  if (selected) {
    await loadFile(Array.isArray(selected) ? selected[0] : selected);
  }
});

el("clear-btn").addEventListener("click", () => {
  state.filePath = null;
  workspace.classList.add("hidden");
  dropZone.classList.remove("hidden");
  fileHint.textContent = "No file selected";
});

// Native OS drag-and-drop (Tauri webview event, not HTML5 drag-drop --
// this is real file paths from the OS, not a File object we'd have to
// read into JS memory).
listen("tauri://drag-enter", () => dropOverlay.classList.remove("hidden"));
listen("tauri://drag-leave", () => dropOverlay.classList.add("hidden"));
listen("tauri://drag-drop", async (event) => {
  dropOverlay.classList.add("hidden");
  const paths = event.payload?.paths ?? [];
  if (paths.length > 0) {
    await loadFile(paths[0]);
  }
});

document.querySelectorAll(".tab").forEach((tab) => {
  tab.addEventListener("click", () => {
    document.querySelectorAll(".tab").forEach((t) => t.classList.remove("active"));
    tab.classList.add("active");
    state.mode = tab.dataset.mode;
    el("archive-controls").classList.toggle("hidden", state.mode !== "archive");
    el("image-controls").classList.toggle("hidden", state.mode !== "image");
    el("video-controls").classList.toggle("hidden", state.mode !== "video");
    el("document-controls").classList.toggle("hidden", state.mode !== "document");
  });
});

document.querySelectorAll(".page-switch-btn").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll(".page-switch-btn").forEach((b) => b.classList.remove("active"));
    btn.classList.add("active");
    const page = btn.dataset.page;
    el("compress-page").classList.toggle("hidden", page !== "compress");
    el("convert-page").classList.toggle("hidden", page !== "convert");
  });
});

const archiveEffort = el("archive-effort");
archiveEffort.addEventListener("input", () => {
  el("archive-effort-value").textContent = archiveEffort.value;
});

const imageEffort = el("image-effort");
const imageCodecSelect = el("image-codec");
imageEffort.addEventListener("input", () => {
  el("image-effort-value").textContent = imageEffort.value;
});
imageCodecSelect.addEventListener("change", () => {
  const isJpeg = imageCodecSelect.value === "jpeg";
  el("image-effort-label").firstChild.textContent = isJpeg
    ? "JPEG quality "
    : "Optimization effort ";
  if (!isJpeg) {
    // PNG is lossless -- there's no size dial, so force target-size mode off
    // rather than leave a control checked that silently does nothing.
    imageTargetSizeEnabled.checked = false;
    el("image-target-size-row").classList.add("hidden");
  }
});

const imageTargetSizeEnabled = el("image-target-size-enabled");
imageTargetSizeEnabled.addEventListener("change", () => {
  el("image-target-size-row").classList.toggle("hidden", !imageTargetSizeEnabled.checked);
});

const videoEffort = el("video-effort");
videoEffort.addEventListener("input", () => {
  el("video-effort-value").textContent = videoEffort.value;
});

const documentEffort = el("document-effort");
documentEffort.addEventListener("input", () => {
  el("document-effort-value").textContent = documentEffort.value;
});

function showProgress() {
  progressCard.classList.remove("hidden");
  progressFill.style.width = "0%";
  progressLabel.textContent = "Working…";
}

function hideProgress() {
  progressCard.classList.add("hidden");
}

listen("compress-progress", (event) => renderProgress(event.payload, formatBytes));
listen("decompress-progress", (event) => renderProgress(event.payload, formatBytes));
listen("video-compress-progress", (event) => renderProgress(event.payload, formatMs));

function renderProgress({ processed, total }, format) {
  if (total) {
    const pct = Math.min(100, Math.round((processed / total) * 100));
    progressFill.style.width = `${pct}%`;
    progressLabel.textContent = `${pct}% · ${format(processed)} / ${format(total)}`;
  } else {
    progressLabel.textContent = `${format(processed)} processed`;
  }
}

const { revealItemInDir } = window.__TAURI__.opener;

/// Renders a result card, optionally with a working "Show in folder"
/// button when `outputPath` is given (only ever passed on success --
/// there's nothing to show after a failed run). Uses `revealItemInDir`
/// (opens the OS file explorer with the file selected) rather than
/// opening the file directly: it needs no extra permission scope beyond
/// the opener plugin's default set, unlike launching the file itself
/// (which can run arbitrary executables and would need explicit path
/// scoping -- see capabilities/default.json).
function renderResultCard(cardEl, title, ok, rows, outputPath) {
  cardEl.classList.remove("hidden");
  const rowsHtml = rows
    .map(([label, value]) => `<div class="result-row"><span>${label}</span><strong>${value}</strong></div>`)
    .join("");
  const showBtnId = `${cardEl.id}-show-btn`;
  const buttonHtml = outputPath
    ? `<div class="actions"><button class="btn btn-secondary" id="${showBtnId}">Show in folder</button></div>`
    : "";
  cardEl.innerHTML = `<div class="result-title ${ok ? "ok" : "err"}">${title}</div>${rowsHtml}${buttonHtml}`;

  if (outputPath) {
    el(showBtnId).addEventListener("click", async (e) => {
      const btn = e.currentTarget;
      btn.disabled = true;
      btn.textContent = "Opening…";
      try {
        await revealItemInDir(outputPath);
        btn.textContent = "Show in folder";
        btn.disabled = false;
      } catch (err) {
        btn.textContent = `Couldn't open folder: ${String(err)}`;
      }
    });
  }
}

function renderResult(title, ok, rows, outputPath) {
  renderResultCard(resultCard, title, ok, rows, outputPath);
}

function setButtonsDisabled(disabled) {
  document
    .querySelectorAll(".actions .btn")
    .forEach((btn) => (btn.disabled = disabled));
}

el("archive-compress-btn").addEventListener("click", async () => {
  if (!state.filePath) return;
  const codec = el("archive-codec").value;
  const effort = Number(archiveEffort.value);
  const verify = el("archive-verify").checked;
  const output = suggestedOutputPath(state.filePath, codec === "zstd" ? "zst" : "gz");

  setButtonsDisabled(true);
  showProgress();
  try {
    const result = await invoke("compress_archive", {
      input: state.filePath,
      output,
      codec,
      effort,
      verify,
    });
    const rows = [
      ["Output", output],
      ["Size", `${formatBytes(result.input_bytes)} → ${formatBytes(result.output_bytes)}`],
      ["Ratio", `${result.ratio.toFixed(2)}x`],
    ];
    if (result.verified !== null) {
      rows.push(["Integrity", result.verified ? "verified (sha256 match)" : "FAILED"]);
    }
    renderResult(
      result.verified === false ? "Compressed, but verification failed" : "Compressed",
      result.verified !== false,
      rows,
      output,
    );
  } catch (err) {
    renderResult("Compression failed", false, [["Error", String(err)]]);
  } finally {
    hideProgress();
    setButtonsDisabled(false);
  }
});

el("archive-decompress-btn").addEventListener("click", async () => {
  if (!state.filePath) return;
  const output = suggestedOutputPath(state.filePath, "out");

  setButtonsDisabled(true);
  showProgress();
  try {
    const result = await invoke("decompress_archive", {
      input: state.filePath,
      output,
      maxDecompressedBytes: null,
    });
    renderResult(
      "Decompressed",
      true,
      [
        ["Output", output],
        ["Size", `${formatBytes(result.input_bytes)} → ${formatBytes(result.output_bytes)}`],
      ],
      output,
    );
  } catch (err) {
    renderResult("Decompression failed", false, [["Error", String(err)]]);
  } finally {
    hideProgress();
    setButtonsDisabled(false);
  }
});

el("image-compress-btn").addEventListener("click", async () => {
  if (!state.filePath) return;
  const codec = imageCodecSelect.value;
  const effort = Number(imageEffort.value);
  const checkFidelity = el("image-fidelity").checked;
  const output = suggestedOutputPath(state.filePath, codec === "png" ? "small.png" : "jpg");
  const targetSizeMode = codec === "jpeg" && imageTargetSizeEnabled.checked;

  setButtonsDisabled(true);
  try {
    if (targetSizeMode) {
      const targetKb = Number(el("image-target-size").value);
      const result = await invoke("compress_image_to_target_size", {
        input: state.filePath,
        output,
        targetBytes: Math.max(1, Math.round(targetKb * 1024)),
      });
      const rows = [
        ["Output", output],
        ["Size", `${formatBytes(result.input_bytes)} → ${formatBytes(result.output_bytes)}`],
        ["Ratio", `${result.ratio.toFixed(2)}x`],
        ["Quality used", `${result.achieved_quality} (${result.iterations} tries)`],
      ];
      if (!result.hit_target) {
        rows.push(["Note", "couldn't get under target even at quality 1 -- this is the smallest possible"]);
      }
      renderResult(result.hit_target ? "Recompressed to target size" : "Recompressed (target not reachable)", true, rows, output);
      return;
    }

    const result = await invoke("compress_image", {
      input: state.filePath,
      output,
      codec,
      effort,
      checkFidelity,
    });
    const rows = [
      ["Output", output],
      ["Size", `${formatBytes(result.input_bytes)} → ${formatBytes(result.output_bytes)}`],
      ["Ratio", `${result.ratio.toFixed(2)}x`],
    ];
    let ok = true;
    if (result.fidelity) {
      if (result.is_lossless) {
        ok = result.fidelity.exact_match;
        rows.push(["Pixel match", ok ? "exact" : `FAILED (max delta ${result.fidelity.max_channel_delta})`]);
      } else {
        rows.push([
          "Pixel delta",
          `max ${result.fidelity.max_channel_delta}, mean ${result.fidelity.mean_channel_delta.toFixed(2)}`,
        ]);
      }
    }
    renderResult(ok ? "Recompressed" : "Recompressed, but fidelity check failed", ok, rows, output);
  } catch (err) {
    renderResult("Recompression failed", false, [["Error", String(err)]]);
  } finally {
    setButtonsDisabled(false);
  }
});

el("video-compress-btn").addEventListener("click", async () => {
  if (!state.filePath) return;
  const effort = Number(videoEffort.value);
  const output = suggestedOutputPath(state.filePath, "compressed.mp4");

  setButtonsDisabled(true);
  showProgress();
  try {
    const result = await invoke("compress_video", {
      input: state.filePath,
      output,
      codec: "ffmpeg",
      effort,
    });
    renderResult(
      "Recompressed",
      true,
      [
        ["Output", output],
        ["Size", `${formatBytes(result.input_bytes)} → ${formatBytes(result.output_bytes)}`],
        ["Ratio", `${result.ratio.toFixed(2)}x`],
      ],
      output,
    );
  } catch (err) {
    renderResult("Video recompression failed", false, [["Error", String(err)]]);
  } finally {
    hideProgress();
    setButtonsDisabled(false);
  }
});

el("document-compress-btn").addEventListener("click", async () => {
  if (!state.filePath) return;
  const effort = Number(documentEffort.value);
  const output = suggestedOutputPath(state.filePath, "compressed.pdf");

  setButtonsDisabled(true);
  try {
    const result = await invoke("compress_document", {
      input: state.filePath,
      output,
      codec: "pdf",
      effort,
    });
    renderResult(
      "Recompressed",
      true,
      [
        ["Output", output],
        ["Size", `${formatBytes(result.input_bytes)} → ${formatBytes(result.output_bytes)}`],
        ["Ratio", `${result.ratio.toFixed(2)}x`],
        ["Images", `${result.images_recompressed} recompressed, ${result.images_skipped} skipped`],
      ],
      output,
    );
  } catch (err) {
    renderResult("Document recompression failed", false, [["Error", String(err)]]);
  } finally {
    setButtonsDisabled(false);
  }
});

// ---------------------------------------------------------------------
// Convert page: format conversion (image<->image, images->PDF, PDF
// merge). Deliberately separate state from the Compress page's single
// `state.filePath` -- these panels need multi-file selection, which the
// drop-zone/workspace model above was never built for.
// ---------------------------------------------------------------------

const convertResultCard = el("convert-result-card");

function renderConvertResult(title, ok, rows, outputPath) {
  renderResultCard(convertResultCard, title, ok, rows, outputPath);
}

function fileBaseName(path) {
  return path.split(/[\\/]/).pop();
}

document.querySelectorAll(".convert-tab").forEach((tab) => {
  tab.addEventListener("click", () => {
    document.querySelectorAll(".convert-tab").forEach((t) => t.classList.remove("active"));
    tab.classList.add("active");
    state.convertMode = tab.dataset.convertMode;
    el("image-format-panel").classList.toggle("hidden", state.convertMode !== "image-format");
    el("images-to-pdf-panel").classList.toggle("hidden", state.convertMode !== "images-to-pdf");
    el("merge-pdfs-panel").classList.toggle("hidden", state.convertMode !== "merge-pdfs");
    convertResultCard.classList.add("hidden");
  });
});

el("image-format-browse-btn").addEventListener("click", async () => {
  const selected = await openDialog({ multiple: false, directory: false });
  if (!selected) return;
  state.convertSingleFile = Array.isArray(selected) ? selected[0] : selected;
  el("image-format-file-name").textContent = fileBaseName(state.convertSingleFile);
  el("image-format-convert-btn").disabled = false;
});

el("image-format-convert-btn").addEventListener("click", async () => {
  if (!state.convertSingleFile) return;
  const targetFormat = el("image-format-target").value;
  const output = suggestedOutputPath(state.convertSingleFile, targetFormat === "jpeg" ? "jpg" : targetFormat);

  setButtonsDisabled(true);
  try {
    const result = await invoke("convert_image", {
      input: state.convertSingleFile,
      output,
      targetFormat,
    });
    renderConvertResult(
      "Converted",
      true,
      [
        ["Output", output],
        ["Size", `${formatBytes(result.input_bytes)} → ${formatBytes(result.output_bytes)}`],
      ],
      output,
    );
  } catch (err) {
    renderConvertResult("Conversion failed", false, [["Error", String(err)]]);
  } finally {
    setButtonsDisabled(false);
  }
});

el("images-to-pdf-browse-btn").addEventListener("click", async () => {
  const selected = await openDialog({ multiple: true, directory: false });
  if (!selected) return;
  state.convertFiles = Array.isArray(selected) ? selected : [selected];
  el("images-to-pdf-file-name").textContent =
    state.convertFiles.length === 1
      ? fileBaseName(state.convertFiles[0])
      : `${state.convertFiles.length} images selected`;
  el("images-to-pdf-convert-btn").disabled = state.convertFiles.length === 0;
});

el("images-to-pdf-convert-btn").addEventListener("click", async () => {
  if (state.convertFiles.length === 0) return;
  const output = suggestedOutputPath(state.convertFiles[0], "pdf");

  setButtonsDisabled(true);
  try {
    const result = await invoke("images_to_pdf", {
      inputs: state.convertFiles,
      output,
    });
    renderConvertResult(
      "PDF created",
      true,
      [
        ["Output", output],
        ["Pages", result.page_count],
        ["Size", formatBytes(result.output_bytes)],
      ],
      output,
    );
  } catch (err) {
    renderConvertResult("PDF creation failed", false, [["Error", String(err)]]);
  } finally {
    setButtonsDisabled(false);
  }
});

el("merge-pdfs-browse-btn").addEventListener("click", async () => {
  const selected = await openDialog({ multiple: true, directory: false, filters: [{ name: "PDF", extensions: ["pdf"] }] });
  if (!selected) return;
  state.convertFiles = Array.isArray(selected) ? selected : [selected];
  el("merge-pdfs-file-name").textContent =
    state.convertFiles.length === 1
      ? fileBaseName(state.convertFiles[0])
      : `${state.convertFiles.length} PDFs selected`;
  el("merge-pdfs-convert-btn").disabled = state.convertFiles.length < 2;
});

el("merge-pdfs-convert-btn").addEventListener("click", async () => {
  if (state.convertFiles.length < 2) return;
  const output = suggestedOutputPath(state.convertFiles[0], "merged.pdf");

  setButtonsDisabled(true);
  try {
    const result = await invoke("merge_pdfs", {
      inputs: state.convertFiles,
      output,
    });
    renderConvertResult(
      "PDFs merged",
      true,
      [
        ["Output", output],
        ["Files merged", result.input_count],
        ["Size", formatBytes(result.output_bytes)],
      ],
      output,
    );
  } catch (err) {
    renderConvertResult("Merge failed", false, [["Error", String(err)]]);
  } finally {
    setButtonsDisabled(false);
  }
});
