mod commands;

use commands::{compile_selection, load_manifest, scan_folder, suggest_output_name};

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .invoke_handler(tauri::generate_handler![
            load_manifest,
            scan_folder,
            compile_selection,
            suggest_output_name,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Forest Data Compiler");
}
