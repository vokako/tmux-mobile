use tmux_mobile::{config::Config, server};

#[tokio::main]
async fn main() {
    let cfg = Config::load();
    if let Err(e) = server::start_with_socket(&cfg.host, cfg.port, &cfg.token, cfg.tmux_socket).await {
        eprintln!("❌ Server error: {}", e);
        std::process::exit(1);
    }
}
