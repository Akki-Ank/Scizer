// Dedicated Web Worker: the wasm module lives here, off the main/UI
// thread, so a compression job never freezes the page. See sizer-wasm's
// lib.rs doc comment for why this is what provides "off the main
// thread" rather than any threading inside the wasm module itself.

import init, {
  detectFormat,
  compressGzip,
  decompressGzip,
  recompressJpeg,
  comparePixels,
} from "../pkg/sizer_wasm.js";

const ready = init();

self.onmessage = async (event) => {
  await ready;
  const { id, action, payload } = event.data;

  const onProgress = (processed, total) => {
    self.postMessage({ id, progress: { processed, total } });
  };

  try {
    let result;
    switch (action) {
      case "detect":
        result = detectFormat(payload.bytes);
        break;
      case "compressGzip":
        result = await compressGzip(payload.bytes, payload.effort, onProgress);
        break;
      case "decompressGzip":
        result = await decompressGzip(
          payload.bytes,
          payload.maxDecompressedBytes,
          onProgress,
        );
        break;
      case "recompressJpeg":
        result = await recompressJpeg(payload.bytes, payload.quality);
        break;
      case "comparePixels":
        result = comparePixels(payload.original, payload.recompressed);
        break;
      default:
        throw new Error(`unknown action ${action}`);
    }
    const transfer = result instanceof Uint8Array ? [result.buffer] : [];
    self.postMessage({ id, result }, transfer);
  } catch (err) {
    self.postMessage({ id, error: String(err) });
  }
};
