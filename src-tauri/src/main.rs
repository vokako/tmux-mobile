use tmux_mobile::run;

fn main() {
    run();
}

#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::Duration;
    use tmux_mobile::tmux;

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
            println!(
                "   - {} ({} windows, attached={})",
                s.name, s.windows, s.attached
            );
        }
        assert!(!sessions.is_empty(), "No sessions found");
    }

    #[test]
    fn t03_create_and_kill_session() {
        cleanup();
        tmux::new_session(TEST_SESSION, None, None).expect("Failed to create session");
        let sessions = tmux::list_sessions().unwrap();
        assert!(
            sessions.iter().any(|s| s.name == TEST_SESSION),
            "Test session not found"
        );
        println!("✅ Created session: {}", TEST_SESSION);

        tmux::kill_session(TEST_SESSION).expect("Failed to kill session");
        let sessions = tmux::list_sessions().unwrap();
        assert!(
            !sessions.iter().any(|s| s.name == TEST_SESSION),
            "Test session still exists"
        );
        println!("✅ Killed session: {}", TEST_SESSION);
    }

    #[test]
    fn t04_list_panes() {
        cleanup();
        tmux::new_session(TEST_SESSION, None, None).unwrap();
        let panes = tmux::list_panes(TEST_SESSION).expect("Failed to list panes");
        println!("✅ Session {} has {} pane(s):", TEST_SESSION, panes.len());
        for p in &panes {
            println!(
                "   - window:{} pane:{} ({}x{}) cmd={}",
                p.window, p.pane, p.width, p.height, p.current_command
            );
        }
        assert!(!panes.is_empty(), "No panes found");
        cleanup();
    }

    #[test]
    fn t05_send_command_and_capture() {
        cleanup();
        tmux::new_session(TEST_SESSION, None, None).unwrap();
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
        tmux::new_session(TEST_SESSION, None, None).unwrap();
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
        tmux::new_session(TEST_SESSION, None, None).unwrap();
        thread::sleep(Duration::from_millis(200));

        tmux::send_command(
            TEST_SESSION,
            "for i in $(seq 1 100); do echo \"line_$i\"; done",
        )
        .unwrap();
        thread::sleep(Duration::from_millis(1000));

        let output = tmux::capture_pane(TEST_SESSION, Some(50)).unwrap();
        assert!(output.contains("line_100"), "Should capture line_100");
        println!("✅ Scrollback capture works");
        cleanup();
    }

    #[test]
    fn t08_literal_ctrl_bytes_reach_extended_keys_pane() {
        // With `extended-keys on`, tmux DROPS raw C0 bytes sent via
        // `send-keys -l` to panes in extended key mode (modifyOtherKeys /
        // kitty — what every modern agent TUI enables). send_keys must
        // translate them to named keys so they survive. The probe puts its
        // tty in raw mode, enables modifyOtherKeys level 1 (tmux shows
        // `Ext 1`, same as kiro-cli), and echoes the repr of every byte read.
        cleanup();
        let probe = "import sys, tty, os, time\n\
                     tty.setraw(0)\n\
                     time.sleep(0.3)\n\
                     sys.stdout.write('\\x1b[>4;1m'); sys.stdout.flush()\n\
                     sys.stdout.write('PROBE_READY\\r\\n'); sys.stdout.flush()\n\
                     [sys.stdout.write('GOT ' + repr(os.read(0, 64)) + '\\r\\n') or sys.stdout.flush() for _ in iter(int, 1)]";
        std::fs::write("/tmp/tmm_kbprobe_test.py", probe).unwrap();
        std::process::Command::new("tmux")
            .args(["new-session", "-d", "-s", TEST_SESSION, "python3 /tmp/tmm_kbprobe_test.py"])
            .status()
            .unwrap();
        thread::sleep(Duration::from_millis(1500));

        // Mixed literal payload: text + Ctrl-C + text, plus a lone Ctrl-F and
        // a Ctrl+Alt combo, exactly what the frontend key path produces.
        tmux::send_keys(TEST_SESSION, "ab\x03cd", true).unwrap();
        tmux::send_keys(TEST_SESSION, "\x06", true).unwrap();
        tmux::send_keys(TEST_SESSION, "\x1b\x14", true).unwrap();
        thread::sleep(Duration::from_millis(600));

        let output = tmux::capture_pane(TEST_SESSION, Some(50)).unwrap();
        println!("probe output:\n{}", output);
        assert!(output.contains("PROBE_READY"), "probe did not start");
        assert!(output.contains("'ab'"), "leading literal text lost");
        assert!(output.contains("\\x03"), "Ctrl-C byte dropped by extended-keys pane");
        assert!(output.contains("'cd'"), "trailing literal text lost");
        assert!(output.contains("\\x06"), "Ctrl-F byte dropped by extended-keys pane");
        assert!(output.contains("\\x1b\\x14"), "Ctrl+Alt-T (ESC + C0) dropped or split");
        println!("✅ literal ctrl bytes reach an extended-keys pane");
        cleanup();
        let _ = std::fs::remove_file("/tmp/tmm_kbprobe_test.py");
    }

    #[test]
    fn t09_paste_text_brackets_iff_pane_requested() {
        // paste_text must reproduce real terminal paste semantics: apps that
        // enabled bracketed paste (mode ?2004) receive \x1b[200~ … \x1b[201~
        // around the block (so pasted newlines are NOT executed line by
        // line); apps that didn't get the raw text.
        cleanup();
        let probe = "import sys, tty, os, time\n\
                     tty.setraw(0)\n\
                     time.sleep(0.3)\n\
                     if os.environ.get('BRACKET'): sys.stdout.write('\\x1b[?2004h'); sys.stdout.flush()\n\
                     sys.stdout.write('PROBE_READY\\r\\n'); sys.stdout.flush()\n\
                     [sys.stdout.write('GOT ' + repr(os.read(0, 256)) + '\\r\\n') or sys.stdout.flush() for _ in iter(int, 1)]";
        std::fs::write("/tmp/tmm_pasteprobe_test.py", probe).unwrap();

        // 1) bracketed-paste pane
        std::process::Command::new("tmux")
            .args(["new-session", "-d", "-s", TEST_SESSION, "BRACKET=1 python3 /tmp/tmm_pasteprobe_test.py"])
            .status()
            .unwrap();
        thread::sleep(Duration::from_millis(1500));
        tmux::paste_text(TEST_SESSION, "line1\rline2\rline3").unwrap();
        thread::sleep(Duration::from_millis(600));
        let output = tmux::capture_pane(TEST_SESSION, Some(50)).unwrap();
        println!("bracketed probe:\n{}", output);
        assert!(output.contains("\\x1b[200~"), "missing bracketed paste start marker");
        assert!(output.contains("\\x1b[201~"), "missing bracketed paste end marker");
        assert!(output.contains("line1\\rline2\\rline3") || (output.contains("line1") && output.contains("line3")),
            "pasted body lost");
        cleanup();

        // 2) legacy pane (no ?2004): raw text, no markers
        std::process::Command::new("tmux")
            .args(["new-session", "-d", "-s", TEST_SESSION, "python3 /tmp/tmm_pasteprobe_test.py"])
            .status()
            .unwrap();
        thread::sleep(Duration::from_millis(1500));
        tmux::paste_text(TEST_SESSION, "plain\rpaste").unwrap();
        thread::sleep(Duration::from_millis(600));
        let output = tmux::capture_pane(TEST_SESSION, Some(50)).unwrap();
        println!("legacy probe:\n{}", output);
        assert!(!output.contains("\\x1b[200~"), "legacy pane must not receive paste markers");
        assert!(output.contains("plain") && output.contains("paste"), "pasted body lost");
        println!("✅ paste_text brackets iff the pane requested it");
        cleanup();
        let _ = std::fs::remove_file("/tmp/tmm_pasteprobe_test.py");
    }
}
