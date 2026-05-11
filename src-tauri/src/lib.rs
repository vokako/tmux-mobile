pub mod config;
pub mod fs;
pub mod server;
pub mod tmux;

#[cfg(desktop)]
use config::Config;

#[tauri::command]
fn get_local_config() -> serde_json::Value {
    config::get_config_json()
}

fn sanitize_filename(name: &str) -> Result<String, String> {
    // Extract just the filename, stripping any directory components
    let fname = std::path::Path::new(name)
        .file_name()
        .and_then(|f| f.to_str())
        .ok_or_else(|| "invalid filename".to_string())?;
    if fname.is_empty() || fname == "." || fname == ".." {
        return Err("invalid filename".to_string());
    }
    Ok(fname.to_string())
}

#[tauri::command]
fn save_to_downloads(name: String, data: Vec<u8>) -> Result<String, String> {
    // Used to take base64 String — the round trip
    //   Rust raw → JS base64 → JSON IPC → Rust base64-decode → write
    // dominated download time on Android for any non-trivial file.
    // Now Tauri's IPC carries Vec<u8> directly via its binary channel,
    // so the bytes pass through without a single copy or transcode.
    let safe_name = sanitize_filename(&name)?;
    let dir = std::path::PathBuf::from("/storage/emulated/0/Download/TmuxMobile");
    std::fs::create_dir_all(&dir).ok();
    let path = dir.join(&safe_name);
    std::fs::write(&path, &data).map_err(|e| format!("write: {}", e))?;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
fn list_downloads() -> Result<Vec<String>, String> {
    let dir = std::path::PathBuf::from("/storage/emulated/0/Download/TmuxMobile");
    std::fs::create_dir_all(&dir).ok();
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                if let Some(name) = entry.file_name().to_str() {
                    files.push(name.to_string());
                }
            }
        }
    }
    files.sort();
    Ok(files)
}

#[tauri::command]
fn delete_download(name: String) -> Result<(), String> {
    let safe_name = sanitize_filename(&name)?;
    let path = std::path::PathBuf::from("/storage/emulated/0/Download/TmuxMobile").join(&safe_name);
    std::fs::remove_file(&path).map_err(|e| format!("delete: {}", e))
}

#[tauri::command]
fn get_download_path(name: String) -> Result<String, String> {
    let safe_name = sanitize_filename(&name)?;
    Ok(format!("/storage/emulated/0/Download/TmuxMobile/{}", safe_name))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(desktop)]
    {
        let cfg = Config::load();
        tmux::set_scrollback(cfg.scrollback);
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                if let Err(e) = server::start_with_socket(&cfg.host, cfg.port, &cfg.token, &cfg.machine_id, cfg.tmux_socket, cfg.tls_cert, cfg.tls_key, cfg.disconnect_grace_secs).await {
                    eprintln!("Server error: {}", e);
                }
            });
        });
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![get_local_config, save_to_downloads, list_downloads, delete_download, get_download_path])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
