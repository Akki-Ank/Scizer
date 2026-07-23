// Suppresses the extra console window Windows would otherwise open
// alongside the app window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod progress;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            commands::detect_format,
            commands::compress_archive,
            commands::decompress_archive,
            commands::compress_image,
            commands::compress_image_to_target_size,
            commands::compress_video,
            commands::compress_document,
            commands::convert_image,
            commands::images_to_pdf,
            commands::merge_pdfs,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Sizer desktop app");
}
