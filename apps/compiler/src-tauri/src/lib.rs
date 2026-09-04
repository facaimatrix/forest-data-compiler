mod commands;

use commands::{
    compile_selection, export_author_directory, inspect_folder_metadata, load_manifest,
    scan_folder, suggest_output_name, write_dataset_metadata,
};

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .invoke_handler(tauri::generate_handler![
            load_manifest,
            scan_folder,
            compile_selection,
            suggest_output_name,
            inspect_folder_metadata,
            write_dataset_metadata,
            export_author_directory,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Forest Data Compiler");
}
