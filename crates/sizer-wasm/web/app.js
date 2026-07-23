// Main-thread UI only. No compression logic here -- everything CPU-bound
// happens in worker.js, off this thread, so the page stays responsive.
// Plain vanilla JS, no bundler, no framework: matches sizer-desktop/ui's
// approach for the same reason (see docs/ARCHITECTURE.md).

const worker = new Worker("worker.js", { type: "module" });

let nextId = 0;
const pending = new Map();

worker.onmessage = (event) => {
  const { id, result, error, progress } = event.data;
  const entry = pending.get(id);
  if (!entry) return;

  if (progress) {
    entry.onProgress?.(progress.processed, progress.total);
    return;
  }
  pending.delete(id);
  if (error) {
    entry.reject(new Error(error));
  } else {
    entry.resolve(result);
  }
};

function call(action, payload, onProgress) {
  const id = nextId++;
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject, onProgress });
    worker.postMessage({ id, action, payload });
  });
}

const state = { fileName: null, fileBytes: null, mode: "archive" };

const el = (id) => document.getElementById(id);
const dropZone = el("drop-zone");
const workspace = el("workspace");
const dropOverlay = el("drop-overlay");
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

async function loadFile(file) {
  state.fileName = file.name;
  state.fileBytes = new Uint8Array(await file.arrayBuffer());

  fileNameEl.textContent = state.fileName;
  fileMetaEl.textContent = "detecting format…";
  dropZone.classList.add("hidden");
  workspace.classList.remove("hidden");
  resultCard.classList.add("hidden");
  progressCard.classList.add("hidden");

  try {
    const detected = await call("detect", { bytes: state.fileBytes });
    fileMetaEl.textContent = `${detected.format} (${detected.kind})`;
  } catch {
    fileMetaEl.textContent = "format unknown";
  }
}

el("browse-btn").addEventListener("click", () => el("file-input").click());
el("file-input").addEventListener("change", (event) => {
  const file = event.target.files?.[0];
  if (file) loadFile(file);
});

el("clear-btn").addEventListener("click", () => {
  state.fileBytes = null;
  workspace.classList.add("hidden");
  dropZone.classList.remove("hidden");
});

["dragenter", "dragover"].forEach((evt) =>
  window.addEventListener(evt, (e) => {
    e.preventDefault();
    dropOverlay.classList.remove("hidden");
  }),
);
["dragleave", "drop"].forEach((evt) =>
  window.addEventListener(evt, (e) => {
    e.preventDefault();
    if (evt === "dragleave" && e.target !== document.documentElement) return;
    dropOverlay.classList.add("hidden");
  }),
);
window.addEventListener("drop", (e) => {
  e.preventDefault();
  const file = e.dataTransfer?.files?.[0];
  if (file) loadFile(file);
});

document.querySelectorAll(".tab").forEach((tab) => {
  tab.addEventListener("click", () => {
    document.querySelectorAll(".tab").forEach((t) => t.classList.remove("active"));
    tab.classList.add("active");
    state.mode = tab.dataset.mode;
    el("archive-controls").classList.toggle("hidden", state.mode !== "archive");
    el("image-controls").classList.toggle("hidden", state.mode !== "image");
  });
});

const archiveEffort = el("archive-effort");
archiveEffort.addEventListener("input", () => {
  el("archive-effort-value").textContent = archiveEffort.value;
});

const imageQuality = el("image-quality");
imageQuality.addEventListener("input", () => {
  el("image-quality-value").textContent = imageQuality.value;
});

function showProgress() {
  progressCard.classList.remove("hidden");
  progressFill.style.width = "0%";
  progressLabel.textContent = "Working…";
}

function hideProgress() {
  progressCard.classList.add("hidden");
}

function onProgress(processed, total) {
  if (total) {
    const pct = Math.min(100, Math.round((processed / total) * 100));
    progressFill.style.width = `${pct}%`;
    progressLabel.textContent = `${pct}% · ${formatBytes(processed)} / ${formatBytes(total)}`;
  } else {
    progressLabel.textContent = `${formatBytes(processed)} processed`;
  }
}

function renderResult(title, ok, rows) {
  resultCard.classList.remove("hidden");
  const rowsHtml = rows
    .map(([label, value]) => `<div class="result-row"><span>${label}</span><strong>${value}</strong></div>`)
    .join("");
  resultCard.innerHTML = `<div class="result-title ${ok ? "ok" : "err"}">${title}</div>${rowsHtml}`;
}

function downloadBytes(bytes, name) {
  const blob = new Blob([bytes]);
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  a.click();
  URL.revokeObjectURL(url);
}

function setButtonsDisabled(disabled) {
  document.querySelectorAll(".actions .btn").forEach((btn) => (btn.disabled = disabled));
}

el("archive-compress-btn").addEventListener("click", async () => {
  if (!state.fileBytes) return;
  const effort = Number(archiveEffort.value);
  const outName = `${state.fileName}.gz`;

  setButtonsDisabled(true);
  showProgress();
  try {
    const output = await call(
      "compressGzip",
      { bytes: state.fileBytes, effort },
      onProgress,
    );
    downloadBytes(output, outName);
    renderResult("Compressed", true, [
      ["Output", outName],
      ["Size", `${formatBytes(state.fileBytes.length)} → ${formatBytes(output.length)}`],
      ["Ratio", `${(state.fileBytes.length / output.length).toFixed(2)}x`],
    ]);
  } catch (err) {
    renderResult("Compression failed", false, [["Error", String(err)]]);
  } finally {
    hideProgress();
    setButtonsDisabled(false);
  }
});

el("archive-decompress-btn").addEventListener("click", async () => {
  if (!state.fileBytes) return;
  const outName = state.fileName.replace(/\.gz$/, "") || `${state.fileName}.out`;

  setButtonsDisabled(true);
  showProgress();
  try {
    const output = await call(
      "decompressGzip",
      {
        bytes: state.fileBytes,
        maxDecompressedBytes: Math.max(state.fileBytes.length * 100, 50_000_000),
      },
      onProgress,
    );
    downloadBytes(output, outName);
    renderResult("Decompressed", true, [
      ["Output", outName],
      ["Size", `${formatBytes(state.fileBytes.length)} → ${formatBytes(output.length)}`],
    ]);
  } catch (err) {
    renderResult("Decompression failed", false, [["Error", String(err)]]);
  } finally {
    hideProgress();
    setButtonsDisabled(false);
  }
});

el("image-compress-btn").addEventListener("click", async () => {
  if (!state.fileBytes) return;
  const quality = Number(imageQuality.value);
  const checkFidelity = el("image-fidelity").checked;
  const outName = state.fileName.replace(/\.\w+$/, "") + ".jpg";

  setButtonsDisabled(true);
  try {
    const output = await call("recompressJpeg", {
      bytes: state.fileBytes,
      quality,
    });
    downloadBytes(output, outName);

    const rows = [
      ["Output", outName],
      ["Size", `${formatBytes(state.fileBytes.length)} → ${formatBytes(output.length)}`],
      ["Ratio", `${(state.fileBytes.length / output.length).toFixed(2)}x`],
    ];
    let ok = true;
    if (checkFidelity) {
      const fidelity = await call("comparePixels", {
        original: state.fileBytes,
        recompressed: output,
      });
      rows.push([
        "Pixel delta",
        `max ${fidelity.max_channel_delta}, mean ${fidelity.mean_channel_delta.toFixed(2)}`,
      ]);
    }
    renderResult(ok ? "Recompressed" : "Recompressed, but fidelity check failed", ok, rows);
  } catch (err) {
    renderResult("Recompression failed", false, [["Error", String(err)]]);
  } finally {
    setButtonsDisabled(false);
  }
});
