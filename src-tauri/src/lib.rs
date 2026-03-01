pub mod tmux;
pub mod server;
pub mod config;
pub mod fs;

use config::Config;

#[tauri::command]
fn get_local_config() -> serde_json::Value {
    config::get_config_json()
}

#[tauri::command]
fn save_download(name: String, data: String) -> Result<String, String> {
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &data)
        .map_err(|e| format!("base64 decode: {}", e))?;

    // Use Downloads dir on all platforms
    let dir = if cfg!(target_os = "android") {
        std::path::PathBuf::from("/storage/emulated/0/Download")
    } else {
        dirs::download_dir().unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join("Downloads"))
    };
    std::fs::create_dir_all(&dir).ok();
    let path = dir.join(&name);
    std::fs::write(&path, &bytes).map_err(|e| format!("write: {}", e))?;
    Ok(path.to_string_lossy().to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![get_local_config, save_download])
        .setup(|_app| {
            let cfg = Config::load();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = server::start_with_socket(&cfg.host, cfg.port, &cfg.token, cfg.tmux_socket).await {
                    eprintln!("Server error: {}", e);
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
