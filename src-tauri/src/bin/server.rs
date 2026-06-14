use tmux_mobile::{config::Config, server, tmux};

#[tokio::main]
async fn main() {
    let cfg = Config::load();
    tmux::set_scrollback(cfg.scrollback);

    // Standalone server is desktop-only by nature; start the in-process team
    // bus + MCP daemon so a headless server still backs the Team tab.
    let team: server::OptTeam =
        match tmux_mobile::team_bridge::TeamManager::start(&cfg.team_db, &cfg.team_room, &cfg.team_bind, &cfg.team_model) {
            Ok(b) => Some(b as std::sync::Arc<dyn server::TeamBridge>),
            Err(e) => {
                eprintln!("⚠️  team bus failed to start: {}", e);
                None
            }
        };

    if let Err(e) =
        server::start_with_socket(&cfg.host, cfg.port, &cfg.token, &cfg.machine_id, cfg.tmux_socket, cfg.tls_cert, cfg.tls_key, cfg.disconnect_grace_secs, team).await
    {
        eprintln!("❌ Server error: {}", e);
        std::process::exit(1);
    }
}
