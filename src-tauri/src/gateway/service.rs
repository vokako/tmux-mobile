//! The per-user service that keeps the gateway running (board #323): a
//! launchd LaunchAgent on macOS, a systemd user unit on Linux.
//!
//! IDENTITY, not name: an installed file is ours only if it runs THIS `tmm`
//! (absolute path) with THIS config root (the `XDG_CONFIG_HOME` base, passed
//! in the file's environment — never a token, never in argv). Every verb reads
//! the installed file back first; a file of our name that names another
//! executable or root is refused with what it names, never adopted or
//! overwritten (`--replace` backs it up first, for a migration). Install is
//! idempotent: same bytes and running → nothing happens; only `restart`
//! forces one.
//!
//! The service name is a parameter with ONE production default per platform
//! (`LABEL`, `UNIT`); a smoke run passes its own test name, so it can never
//! match — exact string compare — the real service.
//!
//! Rendering is pure (tested on bytes). The file runs `tmm gateway start`
//! directly: no shell, so no `sh -c` and no shell quoting; systemd's own
//! quoting, specifier (`%`) and expansion (`$`) rules are escaped, the plist
//! is XML-escaped, and a newline in any value is refused.

use std::path::{Path, PathBuf};
use std::process::Command;

/// launchd label (the one clawdbjs already runs, #313; deliberately not the
/// app bundle id `com.tmuxmobile.dev`).
pub const LABEL: &str = "cc.voka.tmux-mobile";
/// systemd user unit.
pub const UNIT: &str = "tmux-mobile-gateway.service";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub exe: String,
    /// The XDG base directory (config_dir's parent), not the app dir.
    pub config_home: String,
}

#[derive(Debug, Clone)]
pub struct Spec {
    pub name: String,
    pub id: Identity,
    pub path_env: String,
    pub home: String,
    /// launchd only: where stdout/stderr go.
    pub log: String,
}

fn no_newline(what: &str, v: &str) -> Result<(), String> {
    if v.contains('\n') || v.contains('\r') {
        Err(format!("{what} contains a newline: {v:?}"))
    } else {
        Ok(())
    }
}

/// One argument in a systemd ExecStart= line: double-quoted, `\` and `"`
/// escaped, `%` doubled (specifiers) and `$` doubled (variable expansion).
pub(crate) fn systemd_arg(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '%' => out.push_str("%%"),
            '$' => out.push_str("$$"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// One `Environment=` assignment: quoted, `\` and `"` escaped, `%` doubled
/// (systemd does not expand `$` there, so it stays literal).
pub(crate) fn systemd_env(key: &str, value: &str) -> String {
    let mut v = String::new();
    for c in value.chars() {
        match c {
            '\\' => v.push_str("\\\\"),
            '"' => v.push_str("\\\""),
            '%' => v.push_str("%%"),
            c => v.push(c),
        }
    }
    format!("Environment=\"{key}={v}\"")
}

pub fn render_systemd(spec: &Spec) -> Result<String, String> {
    for (w, v) in [("executable", &spec.id.exe), ("config root", &spec.id.config_home), ("PATH", &spec.path_env), ("HOME", &spec.home)] {
        no_newline(w, v)?;
    }
    Ok(format!(
        "# Managed by `tmm gateway install` (board #323). Edit with tmm, not by hand.\n\
         [Unit]\n\
         Description=tmux-mobile gateway\n\
         After=network.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={} gateway start\n\
         {}\n\
         {}\n\
         {}\n\
         Restart=on-failure\n\
         RestartSec=3s\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        systemd_arg(&spec.id.exe),
        systemd_env("XDG_CONFIG_HOME", &spec.id.config_home),
        systemd_env("HOME", &spec.home),
        systemd_env("PATH", &spec.path_env),
    ))
}

pub(crate) fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

pub fn render_plist(spec: &Spec) -> Result<String, String> {
    for (w, v) in [("label", &spec.name), ("executable", &spec.id.exe), ("config root", &spec.id.config_home), ("PATH", &spec.path_env), ("HOME", &spec.home), ("log", &spec.log)] {
        no_newline(w, v)?;
    }
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <!-- Managed by `tmm gateway install` (board #323). Edit with tmm, not by hand. -->\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
         \t<key>Label</key><string>{label}</string>\n\
         \t<key>ProgramArguments</key>\n\
         \t<array>\n\
         \t\t<string>{exe}</string>\n\
         \t\t<string>gateway</string>\n\
         \t\t<string>start</string>\n\
         \t</array>\n\
         \t<key>EnvironmentVariables</key>\n\
         \t<dict>\n\
         \t\t<key>XDG_CONFIG_HOME</key><string>{root}</string>\n\
         \t\t<key>HOME</key><string>{home}</string>\n\
         \t\t<key>PATH</key><string>{path}</string>\n\
         \t</dict>\n\
         \t<key>RunAtLoad</key><true/>\n\
         \t<key>KeepAlive</key><true/>\n\
         \t<key>StandardOutPath</key><string>{log}</string>\n\
         \t<key>StandardErrorPath</key><string>{log}</string>\n\
         </dict>\n\
         </plist>\n",
        label = xml(&spec.name),
        exe = xml(&spec.id.exe),
        root = xml(&spec.id.config_home),
        home = xml(&spec.home),
        path = xml(&spec.path_env),
        log = xml(&spec.log),
    ))
}

fn unescape_systemd(s: &str) -> String {
    let inner = s.trim().trim_start_matches('"').trim_end_matches('"');
    let mut out = String::new();
    let mut it = inner.chars().peekable();
    while let Some(c) = it.next() {
        match (c, it.peek()) {
            ('\\', Some(&n)) => { out.push(n); it.next(); }
            ('%', Some('%')) | ('$', Some('$')) => { out.push(c); it.next(); }
            _ => out.push(c),
        }
    }
    out
}

fn unxml(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// The identity an installed file names, or None when it is not a file we
/// could have written.
pub fn identity_of(text: &str, mac: bool) -> Option<Identity> {
    if mac {
        let args = text.split("<key>ProgramArguments</key>").nth(1)?;
        let exe = args.split("<string>").nth(1)?.split("</string>").next()?;
        let env = text.split("<key>XDG_CONFIG_HOME</key>").nth(1)?;
        let root = env.split("<string>").nth(1)?.split("</string>").next()?;
        Some(Identity { exe: unxml(exe), config_home: unxml(root) })
    } else {
        let exec = text.lines().find_map(|l| l.strip_prefix("ExecStart="))?;
        let quoted = exec.strip_suffix(" gateway start")?;
        let root = text.lines().find_map(|l| l.strip_prefix("Environment=\"XDG_CONFIG_HOME="))?;
        Some(Identity { exe: unescape_systemd(quoted), config_home: unescape_systemd(&format!("\"{root}")) })
    }
}

/// Where the file for `name` lives.
pub fn file_for(name: &str, mac: bool, home: &Path) -> PathBuf {
    if mac {
        home.join("Library/LaunchAgents").join(format!("{name}.plist"))
    } else {
        home.join(".config/systemd/user").join(name)
    }
}

/// What `install` will do, decided from the file on disk (pure).
#[derive(Debug, PartialEq, Eq)]
pub enum Plan {
    Create,
    /// Same identity, same bytes: only (re)start when it is not running.
    Keep,
    /// Same identity, different bytes (a new PATH, a moved log): rewrite + reload.
    Update,
    /// A file of our name that is not ours.
    Refuse(String),
}

pub fn plan(existing: Option<&str>, rendered: &str, ours: &Identity, mac: bool) -> Plan {
    let Some(text) = existing else { return Plan::Create };
    match identity_of(text, mac) {
        Some(id) if id == *ours => {
            if text == rendered { Plan::Keep } else { Plan::Update }
        }
        Some(id) => Plan::Refuse(format!(
            "an installed service of this name runs {} with config root {} — not this tmm ({}, {}); use --replace to back it up and take over",
            id.exe, id.config_home, ours.exe, ours.config_home
        )),
        None => Plan::Refuse("an installed service of this name was not written by tmm gateway; use --replace to back it up and take over".into()),
    }
}

// ─── Effects ────────────────────────────────────────────────────────────────

pub fn mac() -> bool {
    cfg!(target_os = "macos")
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

/// This tmm and this config root.
pub fn current_identity() -> Result<Identity, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = exe.canonicalize().unwrap_or(exe);
    let dir = crate::config::config_dir();
    let dir = if dir.is_absolute() { dir } else { std::env::current_dir().map_err(|e| e.to_string())?.join(dir) };
    let base = dir.parent().ok_or("config dir has no parent")?.to_path_buf();
    Ok(Identity { exe: exe.to_string_lossy().into(), config_home: base.to_string_lossy().into() })
}

pub fn spec(name: &str) -> Result<Spec, String> {
    let id = current_identity()?;
    let log = Path::new(&id.config_home).join("tmux-mobile").join("gateway.log");
    Ok(Spec {
        name: name.to_string(),
        id,
        path_env: std::env::var("PATH").unwrap_or_default(),
        home: home().to_string_lossy().into(),
        log: log.to_string_lossy().into(),
    })
}

fn run(cmd: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(cmd).args(args).output().map_err(|e| format!("{cmd}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into())
    } else {
        Err(format!("{cmd} {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()))
    }
}

fn uid() -> String {
    // SAFETY: getuid has no preconditions.
    unsafe { libc::getuid() }.to_string()
}

/// Is the service running? (pid when it is)
pub fn running(name: &str) -> Option<u32> {
    if mac() {
        let out = run("launchctl", &["print", &format!("gui/{}/{name}", uid())]).ok()?;
        out.lines().find_map(|l| l.trim().strip_prefix("pid = ")).and_then(|p| p.trim().parse().ok())
    } else {
        let out = run("systemctl", &["--user", "show", name, "--property=ActiveState,MainPID"]).ok()?;
        let active = out.lines().any(|l| l == "ActiveState=active");
        let pid = out.lines().find_map(|l| l.strip_prefix("MainPID=")).and_then(|p| p.parse::<u32>().ok()).unwrap_or(0);
        (active && pid > 0).then_some(pid)
    }
}

fn load(name: &str, file: &Path) -> Result<(), String> {
    if mac() {
        run("launchctl", &["bootstrap", &format!("gui/{}", uid()), &file.to_string_lossy()]).map(|_| ())
    } else {
        run("systemctl", &["--user", "daemon-reload"])?;
        run("systemctl", &["--user", "enable", "--now", name]).map(|_| ())
    }
}

fn unload(name: &str) -> Result<(), String> {
    if mac() {
        run("launchctl", &["bootout", &format!("gui/{}/{name}", uid())]).map(|_| ())
    } else {
        run("systemctl", &["--user", "disable", "--now", name]).map(|_| ())
    }
}

#[derive(Debug)]
pub enum Done {
    Installed,
    AlreadyRunning,
    Started,
    Updated,
    Replaced(PathBuf),
}

/// Install (or keep) the service `name` for this tmm and config root.
pub fn install(name: &str, replace: bool) -> Result<Done, String> {
    let spec = spec(name)?;
    let mac = mac();
    let rendered = if mac { render_plist(&spec)? } else { render_systemd(&spec)? };
    let file = file_for(name, mac, Path::new(&spec.home));
    let existing = std::fs::read_to_string(&file).ok();
    let decided = plan(existing.as_deref(), &rendered, &spec.id, mac);
    let write = |f: &Path| -> Result<(), String> {
        if let Some(dir) = f.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        if mac {
            std::fs::create_dir_all(Path::new(&spec.log).parent().unwrap_or(Path::new("."))).map_err(|e| e.to_string())?;
        }
        let tmp = f.with_extension("tmm-new");
        std::fs::write(&tmp, &rendered).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, f).map_err(|e| e.to_string())
    };
    match decided {
        Plan::Keep if running(name).is_some() => Ok(Done::AlreadyRunning),
        Plan::Keep => {
            if mac {
                let _ = unload(name);
            }
            load(name, &file)?;
            Ok(Done::Started)
        }
        Plan::Create => {
            write(&file)?;
            load(name, &file)?;
            Ok(Done::Installed)
        }
        Plan::Update => {
            let _ = unload(name);
            write(&file)?;
            load(name, &file)?;
            Ok(Done::Updated)
        }
        Plan::Refuse(why) if !replace => Err(why),
        Plan::Refuse(_) => {
            // A migration (the #313 hand-written plist): back up, take over,
            // and put the old one back if ours does not come up.
            let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let backup = file.with_file_name(format!("{}.bak-{stamp}", file.file_name().unwrap_or_default().to_string_lossy()));
            std::fs::copy(&file, &backup).map_err(|e| format!("back up {}: {e}", file.display()))?;
            let _ = unload(name);
            let up = write(&file).and_then(|_| load(name, &file)).and_then(|_| {
                std::thread::sleep(std::time::Duration::from_secs(2));
                running(name).map(|_| ()).ok_or_else(|| "the new service did not start".to_string())
            });
            if let Err(e) = up {
                let _ = unload(name);
                let _ = std::fs::copy(&backup, &file);
                let _ = load(name, &file);
                return Err(format!("{e}; the previous service file was restored from {}", backup.display()));
            }
            Ok(Done::Replaced(backup))
        }
    }
}

/// The installed file, checked to be ours.
fn ours_installed(name: &str) -> Result<Option<PathBuf>, String> {
    let file = file_for(name, mac(), &home());
    let Ok(text) = std::fs::read_to_string(&file) else { return Ok(None) };
    let me = current_identity()?;
    match identity_of(&text, mac()) {
        Some(id) if id == me => Ok(Some(file)),
        Some(id) => Err(format!("the installed {name} runs {} with config root {} — not this tmm; refusing", id.exe, id.config_home)),
        None => Err(format!("the installed {name} was not written by tmm gateway; refusing")),
    }
}

pub fn uninstall(name: &str) -> Result<bool, String> {
    let Some(file) = ours_installed(name)? else { return Ok(false) };
    let _ = unload(name);
    std::fs::remove_file(&file).map_err(|e| e.to_string())?;
    if !mac() {
        let _ = run("systemctl", &["--user", "daemon-reload"]);
    }
    Ok(true)
}

pub fn restart(name: &str) -> Result<(), String> {
    let file = ours_installed(name)?.ok_or_else(|| format!("{name} is not installed: run tmm gateway install"))?;
    if mac() {
        run("launchctl", &["kickstart", "-k", &format!("gui/{}/{name}", uid())]).map(|_| ()).or_else(|_| load(name, &file))
    } else {
        run("systemctl", &["--user", "restart", name]).map(|_| ())
    }
}

/// Installed (and ours?), running pid.
pub fn state(name: &str) -> (Result<bool, String>, Option<u32>) {
    (ours_installed(name).map(|f| f.is_some()), running(name))
}

pub fn logs_command(name: &str, follow: bool) -> (String, Vec<String>) {
    if mac() {
        let log = spec(name).map(|s| s.log).unwrap_or_default();
        let mut a = vec!["-n".to_string(), "100".into()];
        if follow {
            a.push("-f".into());
        }
        a.push(log);
        ("tail".into(), a)
    } else {
        let mut a = vec!["--user".to_string(), "-u".into(), name.into(), "-n".into(), "100".into(), "--no-pager".into()];
        if follow {
            a.push("-f".into());
        }
        ("journalctl".into(), a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec_with(exe: &str, root: &str) -> Spec {
        Spec {
            name: "tmux-mobile-gateway-test.service".into(),
            id: Identity { exe: exe.into(), config_home: root.into() },
            path_env: "/usr/bin:/bin".into(),
            home: "/home/u".into(),
            log: "/home/u/.config/tmux-mobile/gateway.log".into(),
        }
    }

    #[test]
    fn systemd_quoting_escapes_specifiers_expansion_quotes_and_backslashes() {
        assert_eq!(systemd_arg("/opt/tmm"), "\"/opt/tmm\"");
        assert_eq!(systemd_arg("/a b/%h/$X/\"q\"\\z"), "\"/a b/%%h/$$X/\\\"q\\\"\\\\z\"");
        assert_eq!(systemd_env("PATH", "/a b:%p:$HOME"), "Environment=\"PATH=/a b:%%p:$HOME\"");
        assert!(render_systemd(&spec_with("/bin/tmm\nExecStartPre=/bin/rm", "/r")).is_err(), "a newline cannot smuggle a directive");
    }

    #[test]
    fn a_rendered_unit_runs_tmm_gateway_start_with_its_config_root_and_reads_back_as_the_same_identity() {
        let s = spec_with("/home/u/My Tools/tmm", "/home/u/.config scratch/%x");
        let unit = render_systemd(&s).unwrap();
        assert!(unit.contains("ExecStart=\"/home/u/My Tools/tmm\" gateway start\n"));
        assert!(unit.contains("Environment=\"XDG_CONFIG_HOME=/home/u/.config scratch/%%x\"\n"));
        assert!(unit.contains("Restart=on-failure") && unit.contains("WantedBy=default.target"));
        assert!(!unit.contains("sh -c") && !unit.to_lowercase().contains("token"), "no shell, no secret");
        assert_eq!(identity_of(&unit, false), Some(s.id.clone()));
        let plist = render_plist(&s).unwrap();
        assert!(plist.contains("<string>/home/u/My Tools/tmm</string>\n\t\t<string>gateway</string>\n\t\t<string>start</string>"));
        assert!(plist.contains("<key>KeepAlive</key><true/>"));
        assert_eq!(identity_of(&plist, true), Some(s.id.clone()));
        let odd = spec_with("/a&b/<tmm>", "/r'\"");
        assert_eq!(identity_of(&render_plist(&odd).unwrap(), true), Some(odd.id), "XML escaping round-trips");
    }

    #[test]
    fn install_is_idempotent_and_never_takes_a_file_that_is_not_ours() {
        let s = spec_with("/opt/tmm", "/home/u/.config");
        let unit = render_systemd(&s).unwrap();
        assert_eq!(plan(None, &unit, &s.id, false), Plan::Create);
        assert_eq!(plan(Some(&unit), &unit, &s.id, false), Plan::Keep, "same bytes: nothing to rewrite");
        let other_path = render_systemd(&Spec { path_env: "/new".into(), ..s.clone() }).unwrap();
        assert_eq!(plan(Some(&other_path), &unit, &s.id, false), Plan::Update, "same identity, new environment");
        let elsewhere = render_systemd(&spec_with("/usr/local/bin/tmm", "/home/u/.config")).unwrap();
        assert!(matches!(plan(Some(&elsewhere), &unit, &s.id, false), Plan::Refuse(_)), "another executable");
        let other_root = render_systemd(&spec_with("/opt/tmm", "/tmp/scratch")).unwrap();
        assert!(matches!(plan(Some(&other_root), &unit, &s.id, false), Plan::Refuse(_)), "another config root");
        let hand = "[Service]\nExecStart=/usr/bin/node server.js\n";
        assert!(matches!(plan(Some(hand), &unit, &s.id, false), Plan::Refuse(_)), "a hand-written unit is not ours");
        // The #313 hand-written plist on clawdbjs.
        let old_plist = "<plist><dict><key>Label</key><string>cc.voka.tmux-mobile</string><key>ProgramArguments</key><array><string>/Users/clawd/workplace/tmux-mobile/src-tauri/target/release/server</string></array></dict></plist>";
        assert!(matches!(plan(Some(old_plist), &render_plist(&s).unwrap(), &s.id, true), Plan::Refuse(_)));
    }

    #[test]
    fn a_test_service_name_can_never_address_the_real_one() {
        let home = Path::new("/home/u");
        for mac in [false, true] {
            let real = file_for(if mac { LABEL } else { UNIT }, mac, home);
            let test = file_for(if mac { "cc.voka.tmux-mobile.test" } else { "tmux-mobile-gateway-test.service" }, mac, home);
            assert_ne!(real, test);
            assert_ne!(real.file_name(), test.file_name(), "exact names, no prefix match");
        }
        assert_eq!(file_for(LABEL, true, home), Path::new("/home/u/Library/LaunchAgents/cc.voka.tmux-mobile.plist"));
        assert_eq!(file_for(UNIT, false, home), Path::new("/home/u/.config/systemd/user/tmux-mobile-gateway.service"));
    }
}
