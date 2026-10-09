//! First-run setup (board #323, owner 2026-10-09: "如果本地没有 TMM 的配置，启动时
//! 可以提供配置引导").
//!
//! `tmm setup` runs it alone; `tmm gateway` / `gateway install|start` run it
//! when config.toml does not exist yet. A terminal only: without one nothing
//! is asked, and only what `Config::load` already does on its own happens
//! (a token and a machine id), which the caller says.
//!
//! The answers land in config.toml through the ONE schema Config reads
//! (`config::validate_file`), edited in place with `toml_edit`: every key it
//! did not ask about, and every comment, stays. Nothing is written when the
//! existing file does not parse, when the reader cancels (EOF / an empty
//! line where a value is required is not a cancel — it takes the default),
//! or when an answer is invalid. The write is atomic (a 0600 temp file in
//! the same directory, then rename), because the file holds the token.

use std::io::{BufRead, Write};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answers {
    pub port: u16,
    pub host: String,
    /// None = tmux's default socket.
    pub tmux_socket: Option<String>,
    pub token: String,
    /// Both or neither.
    pub tls: Option<(String, String)>,
}

/// What the wizard starts from: the current file's values, else the defaults.
#[derive(Debug, Clone)]
pub struct Start {
    pub port: u16,
    pub host: String,
    pub tmux_socket: Option<String>,
    pub token: Option<String>,
    pub tls: Option<(String, String)>,
    /// The socket of a running tmux server, offered as a hint.
    pub detected_socket: Option<String>,
}

impl Start {
    pub fn from_file(text: &str, detected_socket: Option<String>) -> Result<Self, String> {
        crate::config::validate_file(text)?;
        let doc: toml_edit::DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| e.to_string())?;
        let s = |k: &str| doc.get(k).and_then(|v| v.as_str()).map(str::to_string).filter(|v| !v.is_empty());
        let port = doc.get("port").and_then(|v| v.as_integer()).and_then(|p| u16::try_from(p).ok()).unwrap_or(9899);
        let tls = match (s("tls_cert"), s("tls_key")) {
            (Some(c), Some(k)) => Some((c, k)),
            _ => None,
        };
        Ok(Start { port, host: s("host").unwrap_or_else(|| "0.0.0.0".into()), tmux_socket: s("tmux_socket"), token: s("token"), tls, detected_socket })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Done(Answers),
    Cancelled,
}

fn ask(input: &mut dyn BufRead, out: &mut dyn Write, prompt: &str, default: &str) -> Result<Option<String>, String> {
    if default.is_empty() {
        write!(out, "{prompt}: ").map_err(|e| e.to_string())?;
    } else {
        write!(out, "{prompt} [{default}]: ").map_err(|e| e.to_string())?;
    }
    out.flush().map_err(|e| e.to_string())?;
    let mut line = String::new();
    if input.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
        return Ok(None); // EOF: cancelled
    }
    let v = line.trim();
    Ok(Some(if v.is_empty() { default.to_string() } else { v.to_string() }))
}

/// The wizard over any input/output (pure enough to test with scripted
/// answers). Re-asks an invalid answer up to three times, then cancels.
pub fn wizard(input: &mut dyn BufRead, out: &mut dyn Write, start: &Start, new_token: &dyn Fn() -> String) -> Result<Outcome, String> {
    macro_rules! get {
        ($prompt:expr, $default:expr, $check:expr) => {{
            let mut got = None;
            for _ in 0..3 {
                let Some(v) = ask(input, out, $prompt, $default)? else { return Ok(Outcome::Cancelled) };
                match $check(&v) {
                    Ok(x) => { got = Some(x); break; }
                    Err(e) => writeln!(out, "  {e}").map_err(|e| e.to_string())?,
                }
            }
            match got { Some(x) => x, None => return Ok(Outcome::Cancelled) }
        }};
    }
    writeln!(out, "tmux-mobile setup — {}", crate::config::config_file().display()).map_err(|e| e.to_string())?;
    let port = get!("Port", &start.port.to_string(), |v: &str| v.parse::<u16>().ok().filter(|p| *p > 0).ok_or("a port is 1–65535".to_string()));
    writeln!(out, "  0.0.0.0 = reachable from your phone on this network (the token is the guard, not encryption — use TLS or Tailscale off your LAN);").map_err(|e| e.to_string())?;
    writeln!(out, "  127.0.0.1 = this machine only.").map_err(|e| e.to_string())?;
    let host = get!("Listen on", &start.host, |v: &str| v.parse::<std::net::IpAddr>().map(|_| v.to_string()).map_err(|_| "an IP address, e.g. 0.0.0.0 or 127.0.0.1".to_string()));
    if let Some(d) = &start.detected_socket {
        writeln!(out, "  a running tmux server uses {d}; leave empty for tmux's default socket.").map_err(|e| e.to_string())?;
    }
    let sock_default = start.tmux_socket.clone().unwrap_or_default();
    let tmux_socket = get!("tmux socket (-S path, empty = default)", &sock_default, |v: &str| {
        Ok::<_, String>(if v.is_empty() || v == "-" { None } else { Some(v.to_string()) })
    });
    let token = match &start.token {
        Some(t) => {
            let keep = get!("Keep the existing token? (y/n)", "y", |v: &str| match v.to_ascii_lowercase().as_str() {
                "y" | "yes" => Ok(true),
                "n" | "no" => Ok(false),
                _ => Err("y or n".to_string()),
            });
            if keep { t.clone() } else { new_token() }
        }
        None => new_token(),
    };
    let tls_default = start.tls.as_ref().map(|(c, _)| c.clone()).unwrap_or_default();
    let cert = get!("TLS certificate (PEM path, empty = no TLS)", &tls_default, |v: &str| {
        if v.is_empty() || v == "-" { Ok(None) } else if Path::new(v).is_file() { Ok(Some(v.to_string())) } else { Err(format!("no file at {v}")) }
    });
    let tls = match cert {
        None => None,
        Some(c) => {
            let key_default = start.tls.as_ref().map(|(_, k)| k.clone()).unwrap_or_default();
            let key = get!("TLS private key (PEM path)", &key_default, |v: &str| {
                if Path::new(v).is_file() { Ok(v.to_string()) } else { Err(format!("a key file is required with a certificate (no file at {v:?})")) }
            });
            Some((c, key))
        }
    };
    Ok(Outcome::Done(Answers { port, host, tmux_socket, token, tls }))
}

/// `existing` with the answers set — every other key and comment kept —
/// checked against the one schema.
pub fn apply(existing: &str, a: &Answers) -> Result<String, String> {
    crate::config::validate_file(existing).map_err(|e| format!("config.toml does not parse ({e}); fix or move it — nothing was written"))?;
    let mut doc: toml_edit::DocumentMut = existing.parse().map_err(|e: toml_edit::TomlError| e.to_string())?;
    doc["port"] = toml_edit::value(i64::from(a.port));
    doc["host"] = toml_edit::value(a.host.as_str());
    doc["token"] = toml_edit::value(a.token.as_str());
    match &a.tmux_socket {
        Some(s) => doc["tmux_socket"] = toml_edit::value(s.as_str()),
        None => { doc.remove("tmux_socket"); }
    }
    match &a.tls {
        Some((c, k)) => {
            doc["tls_cert"] = toml_edit::value(c.as_str());
            doc["tls_key"] = toml_edit::value(k.as_str());
        }
        None => {
            doc.remove("tls_cert");
            doc.remove("tls_key");
        }
    }
    let text = doc.to_string();
    crate::config::validate_file(&text)?;
    Ok(text)
}

/// Replace `path` with `text` atomically, mode 0600 from the first byte.
pub fn write_0600(path: &Path, text: &str) -> Result<(), String> {
    let dir = path.parent().ok_or("config path has no directory")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let tmp = dir.join(format!(".config.toml.{}.tmp", std::process::id()));
    {
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        let mut f = o.open(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
        f.write_all(text.as_bytes()).and_then(|_| f.sync_all()).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            e.to_string()
        })?;
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

fn detect_socket() -> Option<String> {
    let out = std::process::Command::new("tmux").args(["display-message", "-p", "#{socket_path}"]).output().ok()?;
    let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !p.is_empty()).then_some(p)
}

/// `tmm setup` (and the first-run step of `tmm gateway`). Returns whether
/// config.toml was written.
pub fn run_interactive() -> Result<bool, String> {
    use std::io::IsTerminal;
    let path = crate::config::config_file();
    if !std::io::stdin().is_terminal() {
        let _ = crate::config::Config::load(); // a token and a machine id, as on any first start
        eprintln!("setup: not a terminal — nothing asked; defaults apply ({} has a token and nothing else). Run tmm setup in a terminal to choose.", path.display());
        return Ok(false);
    }
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let start = Start::from_file(&existing, detect_socket())
        .map_err(|e| format!("config.toml does not parse ({e}); fix or move it — nothing was written"))?;
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut out = std::io::stdout();
    match wizard(&mut input, &mut out, &start, &|| uuid::Uuid::new_v4().to_string())? {
        Outcome::Cancelled => {
            println!("\nsetup cancelled — nothing was written");
            Ok(false)
        }
        Outcome::Done(a) => {
            let text = apply(&existing, &a)?;
            write_0600(&path, &text)?;
            println!("✓ wrote {}", path.display());
            println!("  next: tmm gateway   (installs and starts the background service)");
            Ok(true)
        }
    }
}

/// The first-run step: setup only when there is no config.toml yet.
pub fn first_run() -> Result<(), String> {
    if crate::config::config_file().exists() {
        return Ok(());
    }
    run_interactive().map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start(text: &str) -> Start {
        Start::from_file(text, Some("/tmp/tmux-1000/default".into())).unwrap()
    }
    fn run(text: &str, script: &str) -> (Outcome, String) {
        let mut out = Vec::new();
        let o = wizard(&mut script.as_bytes(), &mut out, &start(text), &|| "fresh-token".into()).unwrap();
        (o, String::from_utf8(out).unwrap())
    }

    #[test]
    fn defaults_come_from_the_file_and_an_empty_answer_keeps_them() {
        let (o, out) = run("port = 9900\nhost = \"127.0.0.1\"\ntoken = \"old\"\n", "\n\n\n\n\n");
        assert_eq!(o, Outcome::Done(Answers { port: 9900, host: "127.0.0.1".into(), tmux_socket: None, token: "old".into(), tls: None }));
        assert!(out.contains("0.0.0.0 = reachable from your phone") && out.contains("127.0.0.1 = this machine only"));
        assert!(out.contains("/tmp/tmux-1000/default"), "the detected socket is offered");
        let (o, _) = run("", "\n\n\n\n");
        assert_eq!(o, Outcome::Done(Answers { port: 9899, host: "0.0.0.0".into(), tmux_socket: None, token: "fresh-token".into(), tls: None }), "fresh: 0.0.0.0 and a new token");
    }

    #[test]
    fn invalid_answers_are_asked_again_and_eof_cancels() {
        let (o, out) = run("", "99999\n0\n8080\nlocalhost\n127.0.0.1\n/tmp/s\n\n");
        assert!(out.contains("a port is 1–65535") && out.contains("an IP address"));
        assert_eq!(o, Outcome::Done(Answers { port: 8080, host: "127.0.0.1".into(), tmux_socket: Some("/tmp/s".into()), token: "fresh-token".into(), tls: None }));
        assert_eq!(run("", "8080\n").0, Outcome::Cancelled, "EOF mid-way writes nothing");
        assert_eq!(run("", "x\nx\nx\n").0, Outcome::Cancelled, "three bad answers cancel");
        let (o, _) = run("token = \"old\"\n", "\n\n\nn\n\n");
        assert!(matches!(o, Outcome::Done(Answers { ref token, .. }) if token == "fresh-token"), "declining to keep the token makes a new one");
    }

    #[test]
    fn tls_needs_both_existing_files() {
        let dir = std::env::temp_dir().join(format!("setup-tls-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (c, k) = (dir.join("c.pem"), dir.join("k.pem"));
        std::fs::write(&c, "x").unwrap();
        std::fs::write(&k, "x").unwrap();
        let script = format!("\n\n\n/nope.pem\n{}\n/nope.key\n{}\n", c.display(), k.display());
        let (o, out) = run("", &script);
        assert!(out.contains("no file at /nope.pem") && out.contains("a key file is required"));
        assert!(matches!(o, Outcome::Done(Answers { tls: Some(_), .. })));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn apply_sets_the_answers_and_keeps_every_other_key_and_comment() {
        let existing = "# my notes\ntoken = \"old\"\nteam_rules = \"keep me\"\n# about tls\ntls_cert = \"/c\"\ntls_key = \"/k\"\n\n[extra]\nx = 1\n";
        let a = Answers { port: 9900, host: "127.0.0.1".into(), tmux_socket: Some("/s".into()), token: "old".into(), tls: None };
        let text = apply(existing, &a).unwrap();
        assert!(text.contains("# my notes") && text.contains("team_rules = \"keep me\"") && text.contains("[extra]\nx = 1"));
        assert!(text.contains("port = 9900") && text.contains("host = \"127.0.0.1\"") && text.contains("tmux_socket = \"/s\""));
        assert!(!text.contains("tls_cert") && !text.contains("tls_key"), "no TLS answered: both removed");
        crate::config::validate_file(&text).unwrap();
        assert!(apply("port = \"not a number\"\n", &a).is_err(), "a file that does not parse is never rewritten");
        assert!(apply("port = 99999\n", &a).is_err(), "out of range for the schema");
    }

    #[cfg(unix)]
    #[test]
    fn the_write_is_atomic_and_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("setup-write-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("config.toml");
        std::fs::write(&p, "token = \"a\"\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_0600(&p, "token = \"b\"\n").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "token = \"b\"\n");
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "no temp file left behind");
        std::fs::remove_dir_all(&dir).ok();
    }
}
