// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
mod commands;

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {name}! You've been greeted from Rust!")
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .invoke_handler(tauri::generate_handler![
            greet,
            commands::select_directory,
            commands::validate_working_directory,
            commands::set_working_directory,
            commands::get_working_directory,
            commands::execute_duck_cli_sidecar,
            commands::execute_duck_cli_system,
            commands::execute_duck_cli_smart,
            commands::get_cli_version,
            commands::check_cli_available,
            commands::check_and_cleanup_duck_processes,
            commands::check_database_lock,
            commands::debug_environment
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
