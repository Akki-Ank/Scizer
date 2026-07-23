use sizer_core::Progress;
use wasm_bindgen::JsValue;

/// Calls an optional JS callback with `(processedBytes, totalBytesOrNull)`
/// -- the fourth concrete `sizer_core::Progress` sink alongside the CLI's
/// terminal line, the desktop app's Tauri event, and the eventual cloud
/// worker's Redis-backed one.
pub struct JsProgress {
    callback: Option<js_sys::Function>,
}

impl JsProgress {
    pub fn new(callback: Option<js_sys::Function>) -> Self {
        Self { callback }
    }
}

impl Progress for JsProgress {
    fn on_progress(&self, processed: u64, total: Option<u64>) {
        let Some(callback) = &self.callback else {
            return;
        };
        let total_js = total.map_or(JsValue::NULL, |t| JsValue::from_f64(t as f64));
        // Best-effort: a JS-side callback throwing shouldn't fail the
        // compression job itself.
        let _ = callback.call2(
            &JsValue::UNDEFINED,
            &JsValue::from_f64(processed as f64),
            &total_js,
        );
    }
}
