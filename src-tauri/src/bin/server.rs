use tmux_mobile::{config::Config, server, tmux};

#[tokio::main]
async fn main() {
    let cfg = Config::load();
    tmux::set_scrollback(cfg.scrollback);

    // Standalone server is desktop-only by nature; start the in-process agora
    // bus + MCP daemon so a headless server still backs the Team tab.
    let agora: server::OptAgora =
        match tmux_mobile::agora_bridge::AgoraBus::start(&cfg.agora_db, &cfg.agora_room, &cfg.agora_bind, &cfg.agora_model) {
            Ok(b) => Some(b as std::sync::Arc<dyn server::AgoraBridge>),
            Err(e) => {
                eprintln!("⚠️  agora bus failed to start: {}", e);
                None
            }
        };

    if let Err(e) =
        server::start_with_socket(&cfg.host, cfg.port, &cfg.token, &cfg.machine_id, cfg.tmux_socket, cfg.tls_cert, cfg.tls_key, cfg.disconnect_grace_secs, agora).await
    {
        eprintln!("❌ Server error: {}", e);
        std::process::exit(1);
    }
}
