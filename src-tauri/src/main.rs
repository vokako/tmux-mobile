use tmux_mobile::server;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|_app| {
            let host = std::env::var("HOST").unwrap_or("0.0.0.0".to_string());
            let port: u16 = std::env::var("PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(9876);
            let token = std::env::var("TOKEN")
                .unwrap_or_else(|_| uuid::Uuid::new_v4().to_string());

            tauri::async_runtime::spawn(async move {
                if let Err(e) = server::start(&host, port, &token).await {
                    eprintln!("❌ Server error: {}", e);
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn main() {
    run();
}

#[cfg(test)]
mod tests {
    use tmux_mobile::tmux;
    use std::thread;
    use std::time::Duration;

    const TEST_SESSION: &str = "_tmux_mobile_test";

    fn cleanup() {
        let _ = tmux::kill_session(TEST_SESSION);
    }

    #[test]
    fn t01_server_running() {
        assert!(tmux::is_server_running(), "tmux server is not running!");
        println!("✅ tmux server is running");
    }

    #[test]
    fn t02_list_sessions() {
        let sessions = tmux::list_sessions().expect("Failed to list sessions");
        println!("✅ Found {} sessions:", sessions.len());
        for s in &sessions {
            println!("   - {} ({} windows, attached={})", s.name, s.windows, s.attached);
        }
        assert!(!sessions.is_empty(), "No sessions found");
    }

    #[test]
    fn t03_create_and_kill_session() {
        cleanup();
        tmux::new_session(TEST_SESSION).expect("Failed to create session");
        let sessions = tmux::list_sessions().unwrap();
        assert!(sessions.iter().any(|s| s.name == TEST_SESSION), "Test session not found");
        println!("✅ Created session: {}", TEST_SESSION);

        tmux::kill_session(TEST_SESSION).expect("Failed to kill session");
        let sessions = tmux::list_sessions().unwrap();
        assert!(!sessions.iter().any(|s| s.name == TEST_SESSION), "Test session still exists");
        println!("✅ Killed session: {}", TEST_SESSION);
    }

    #[test]
    fn t04_list_panes() {
        cleanup();
        tmux::new_session(TEST_SESSION).unwrap();
        let panes = tmux::list_panes(TEST_SESSION).expect("Failed to list panes");
        println!("✅ Session {} has {} pane(s):", TEST_SESSION, panes.len());
        for p in &panes {
            println!("   - window:{} pane:{} ({}x{}) cmd={}", p.window, p.pane, p.width, p.height, p.current_command);
        }
        assert!(!panes.is_empty(), "No panes found");
        cleanup();
    }

    #[test]
    fn t05_send_command_and_capture() {
        cleanup();
        tmux::new_session(TEST_SESSION).unwrap();
        thread::sleep(Duration::from_millis(200));

        let marker = "TMUX_MOBILE_TEST_12345";
        tmux::send_command(TEST_SESSION, &format!("echo {}", marker)).unwrap();
        thread::sleep(Duration::from_millis(500));

        let output = tmux::capture_pane(TEST_SESSION, Some(50)).expect("Failed to capture pane");
        println!("✅ Captured pane output ({} chars)", output.len());
        assert!(output.contains(marker), "Marker not found in output");
        println!("✅ Command output verified!");
        cleanup();
    }

    #[test]
    fn t06_send_special_keys() {
        cleanup();
        tmux::new_session(TEST_SESSION).unwrap();
        thread::sleep(Duration::from_millis(200));

        tmux::send_keys(TEST_SESSION, "echo partial", true).unwrap();
        thread::sleep(Duration::from_millis(100));
        tmux::send_keys(TEST_SESSION, "C-c", false).unwrap();
        thread::sleep(Duration::from_millis(200));

        let marker = "AFTER_CTRL_C_OK";
        tmux::send_command(TEST_SESSION, &format!("echo {}", marker)).unwrap();
        thread::sleep(Duration::from_millis(500));

        let output = tmux::capture_pane(TEST_SESSION, Some(20)).unwrap();
        assert!(output.contains(marker), "Pane should work after Ctrl-C");
        println!("✅ Special keys (C-c) work correctly");
        cleanup();
    }

    #[test]
    fn t07_capture_scrollback() {
        cleanup();
        tmux::new_session(TEST_SESSION).unwrap();
        thread::sleep(Duration::from_millis(200));

        tmux::send_command(TEST_SESSION, "for i in $(seq 1 100); do echo \"line_$i\"; done").unwrap();
        thread::sleep(Duration::from_millis(1000));

        let output = tmux::capture_pane(TEST_SESSION, Some(50)).unwrap();
        assert!(output.contains("line_100"), "Should capture line_100");
        println!("✅ Scrollback capture works");
        cleanup();
    }
}
