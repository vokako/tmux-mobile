//! The gateway: the one way this machine's server is started (board #323,
//! owner 2026-10-09: "通过输入 tmm gateway 就可以启动后台的 server…做成 Linux 或
//! macOS 下的一个后台进程任务").
//!
//! - `start` is the foreground entry — what `tmm gateway start`, the `server`
//!   binary (a thin alias kept for the dev watcher and existing units) and the
//!   installed service run.
//! - `probe` answers "is OUR gateway already answering here?" with a real
//!   loopback auth compared on machine id, so the desktop app and `status`
//!   never start a second one and no unauthenticated method is added.
//! - `service` renders and manages the per-user launchd agent / systemd user
//!   unit, by identity (name + executable + config root), never by name alone.
//!
//! Desktop-only, like `projects`: a phone is a client of a desktop server.

pub mod probe;
pub mod service;

use crate::config::Config;

/// Run the server for `cfg` in the foreground until it fails.
pub async fn start(cfg: Config) -> Result<(), String> {
    crate::tmux::set_scrollback(cfg.scrollback);
    crate::server::start_with_socket(
        &cfg.host,
        cfg.port,
        &cfg.token,
        &cfg.machine_id,
        cfg.tmux_socket,
        cfg.tls_cert,
        cfg.tls_key,
        cfg.disconnect_grace_secs,
    )
    .await
    .map_err(|e| e.to_string())
}
