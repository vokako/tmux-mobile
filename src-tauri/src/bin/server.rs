use tmux_mobile::{config::Config, server, tmux};

#[tokio::main]
async fn main() {
    let cfg = Config::load();
    tmux::set_scrollback(cfg.scrollback);

    // Standalone server is desktop-only by nature; start the in-process crew
    // bus + MCP daemon so a headless server still backs the Team tab.
    let crew: server::OptCrew =
        match tmux_mobile::crew_bridge::CrewBus::start(&cfg.crew_db, &cfg.crew_room, &cfg.crew_bind, &cfg.crew_model) {
            Ok(b) => Some(b as std::sync::Arc<dyn server::CrewBridge>),
            Err(e) => {
                eprintln!("⚠️  crew bus failed to start: {}", e);
                None
            }
        };

    if let Err(e) =
        server::start_with_socket(&cfg.host, cfg.port, &cfg.token, &cfg.machine_id, cfg.tmux_socket, cfg.tls_cert, cfg.tls_key, cfg.disconnect_grace_secs, crew).await
    {
        eprintln!("❌ Server error: {}", e);
        std::process::exit(1);
    }
}
