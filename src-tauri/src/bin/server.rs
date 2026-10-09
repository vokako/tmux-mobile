// The `server` binary is a thin alias of `tmm gateway start` (board #323):
// the dev watcher and existing service files still launch it by this name.
use tmux_mobile::{config::Config, gateway};

#[tokio::main]
async fn main() {
    if let Err(e) = gateway::start(Config::load()).await {
        eprintln!("❌ Server error: {}", e);
        std::process::exit(1);
    }
}
