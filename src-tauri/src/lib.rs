pub mod agent_notifications;
pub mod config;
pub mod fs;
pub mod pptx;
pub mod server;
pub mod tasks;
pub mod tmux;

// team multi-agent bus bridge + in-process team supervisor — desktop only
// (Android/iOS never build team).
#[cfg(all(desktop, not(any(target_os = "android", target_os = "ios"))))]
pub mod team_bridge;
#[cfg(all(desktop, not(any(target_os = "android", target_os = "ios"))))]
pub mod team;

// Declarative projects (state.db). Desktop-only for the same reason as team:
// the phone is a client of a desktop server, so it never needs SQLite. The gate
// matches the rusqlite dependency gate in Cargo.toml exactly.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod projects;

#[cfg(desktop)]
use config::Config;

#[derive(serde::Serialize)]
struct DownloadEntry {
    name: String,
    modified: u64,
}

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
fn list_downloads() -> Result<Vec<DownloadEntry>, String> {
    let dir = std::path::PathBuf::from("/storage/emulated/0/Download/TmuxMobile");
    std::fs::create_dir_all(&dir).ok();
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                if let Some(name) = entry.file_name().to_str() {
                    let modified = entry
                        .metadata()
                        .ok()
                        .and_then(|meta| meta.modified().ok())
                        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|duration| duration.as_millis() as u64)
                        .unwrap_or(0);
                    files.push(DownloadEntry {
                        name: name.to_string(),
                        modified,
                    });
                }
            }
        }
    }
    files.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| a.name.cmp(&b.name))
    });
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
                // Start the in-process team bus + MCP daemon (best-effort; a
                // failure here just disables the Team tab, the terminal server
                // still runs). Desktop-only — never built on Android/iOS.
                let team: server::OptTeam = {
                    #[cfg(not(any(target_os = "android", target_os = "ios")))]
                    {
                        match team_bridge::TeamManager::start(&cfg.team_db, &cfg.team_room, &cfg.team_bind, &cfg.team_model) {
                            Ok(b) => Some(b as std::sync::Arc<dyn server::TeamBridge>),
                            Err(e) => { eprintln!("⚠️  team bus failed to start: {}", e); None }
                        }
                    }
                    #[cfg(any(target_os = "android", target_os = "ios"))]
                    { None }
                };
                if let Err(e) = server::start_with_socket(&cfg.host, cfg.port, &cfg.token, &cfg.machine_id, cfg.tmux_socket, cfg.tls_cert, cfg.tls_key, cfg.disconnect_grace_secs, team).await {
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
        .setup(|app| {
            // Desktop: build a custom menu WITHOUT the default View → Zoom
            // items.
            //
            // Tauri's default macOS menu includes Zoom entries bound to ⌘+/⌘-
            // that drive WKWebView's NATIVE magnification (scales the whole
            // page — nav/tab bar included). That runs at the AppKit level, so
            // a JS keydown preventDefault can't stop it, and it fights the
            // app's own font-size shortcut (you'd get both at once). We can't
            // remove just that item from the auto-generated menu, so we
            // rebuild a minimal menu that keeps the essentials (copy / paste /
            // cut / select-all / undo / redo / quit + window controls) but
            // omits Zoom. ⌘+/⌘-/⌘0 then reach only the frontend, which
            // applies one persisted native WebView scale and refits xterm to
            // the resulting container without changing terminal font size.
            #[cfg(target_os = "macos")]
            {
                use tauri::menu::{Menu, Submenu, PredefinedMenuItem as P};
                let pkg = &app.package_info().name;
                let app_menu = Submenu::with_items(app, pkg, true, &[
                    &P::about(app, None, None)?,
                    &P::separator(app)?,
                    &P::services(app, None)?,
                    &P::separator(app)?,
                    &P::hide(app, None)?,
                    &P::hide_others(app, None)?,
                    &P::show_all(app, None)?,
                    &P::separator(app)?,
                    &P::quit(app, None)?,
                ])?;
                let edit_menu = Submenu::with_items(app, "Edit", true, &[
                    &P::undo(app, None)?,
                    &P::redo(app, None)?,
                    &P::separator(app)?,
                    &P::cut(app, None)?,
                    &P::copy(app, None)?,
                    &P::paste(app, None)?,
                    &P::select_all(app, None)?,
                ])?;
                let window_menu = Submenu::with_items(app, "Window", true, &[
                    &P::minimize(app, None)?,
                    &P::maximize(app, None)?,
                    &P::separator(app)?,
                    &P::close_window(app, None)?,
                ])?;
                let menu = Menu::with_items(app, &[&app_menu, &edit_menu, &window_menu])?;
                app.set_menu(menu)?;
            }
            // Non-macOS desktop: an empty menu bar is fine (zoom accelerators
            // there come from the menu too; copy/paste work without it).
            #[cfg(all(desktop, not(target_os = "macos")))]
            {
                use tauri::menu::Menu;
                app.set_menu(Menu::new(app)?)?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
