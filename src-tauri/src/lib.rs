pub mod tmux;
pub mod server;
pub mod config;
pub mod fs;

use config::Config;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|_app| {
            let cfg = Config::load();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = server::start(&cfg.host, cfg.port, &cfg.token).await {
                    eprintln!("Server error: {}", e);
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
