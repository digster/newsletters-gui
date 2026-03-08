pub mod db;
pub mod parser;
pub mod commands;

use db::Database;
use tauri::Manager;
use log::info;
use std::sync::atomic::AtomicBool;

/// Global flag to prevent concurrent indexing operations
pub static INDEXING_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

/// Initialize and run the Tauri application
pub fn run() {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info")
    ).init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            // Determine the database path in the app's data directory
            let app_data = app.path().app_data_dir()
                .expect("Failed to resolve app data directory");
            std::fs::create_dir_all(&app_data)
                .expect("Failed to create app data directory");

            let db_path = app_data.join("newsletters.db");
            info!("Database path: {:?}", db_path);

            // Open the database and manage it as Tauri state
            let database = Database::open(&db_path)
                .expect("Failed to open database");
            app.manage(database);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Settings
            commands::emails::get_newsletters_path,
            commands::emails::set_newsletters_path,
            // Scanning / indexing
            commands::scan::scan_and_index,
            // Email browsing
            commands::emails::get_labels,
            commands::emails::get_emails_by_label,
            commands::emails::get_email_count,
            commands::emails::get_email_html,
            commands::emails::get_email,
            // Search
            commands::search::search_emails,
            // User state
            commands::state::toggle_bookmark,
            commands::state::toggle_read,
            commands::state::mark_read,
            commands::state::get_bookmarked_emails,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
