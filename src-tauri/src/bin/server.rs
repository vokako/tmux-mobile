use tmux_mobile::{config::Config, server, tmux};

#[tokio::main]
async fn main() {
    let cfg = Config::load();
    tmux::set_scrollback(cfg.scrollback);
    if let Err(e) =
        server::start_with_socket(&cfg.host, cfg.port, &cfg.token, &cfg.machine_id, cfg.tmux_socket, cfg.tls_cert, cfg.tls_key).await
    {
        eprintln!("❌ Server error: {}", e);
        std::process::exit(1);
    }
}
