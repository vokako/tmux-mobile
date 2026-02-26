use tmux_mobile::server;

#[tokio::main]
async fn main() {
    let host = std::env::var("HOST").unwrap_or("0.0.0.0".to_string());
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(9876);
    let token = std::env::var("TOKEN")
        .unwrap_or_else(|_| uuid::Uuid::new_v4().to_string());

    if let Err(e) = server::start(&host, port, &token).await {
        eprintln!("❌ Server error: {}", e);
        std::process::exit(1);
    }
}
