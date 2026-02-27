/// File system operations for remote file browsing
use serde::Serialize;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

const MAX_READ_SIZE: u64 = 10 * 1024 * 1024; // 10MB
const MAX_PREVIEW_SIZE: u64 = 512 * 1024; // 512KB for text preview

#[derive(Debug, Clone, Serialize)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    #[serde(rename = "type")]
    pub file_type: String, // "file", "dir", "symlink"
    pub size: u64,
    pub modified: u64, // unix timestamp
    pub permissions: String,
    pub hidden: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileStat {
    pub path: String,
    pub name: String,
    #[serde(rename = "type")]
    pub file_type: String,
    pub size: u64,
    pub modified: u64,
    pub permissions: String,
    pub readable: bool,
    pub writable: bool,
    pub is_text: bool,
    pub mime_hint: String,
}

fn resolve_path(p: &str) -> PathBuf {
    let expanded = if p.starts_with('~') {
        if let Some(home) = dirs::home_dir() {
            home.join(&p[1..].trim_start_matches('/'))
        } else {
            PathBuf::from(p)
        }
    } else {
        PathBuf::from(p)
    };
    // Canonicalize if exists, otherwise return as-is
    expanded.canonicalize().unwrap_or(expanded)
}

fn format_permissions(mode: u32) -> String {
    let mut s = String::with_capacity(10);
    let types = [(0o400, 'r'), (0o200, 'w'), (0o100, 'x'),
                 (0o040, 'r'), (0o020, 'w'), (0o010, 'x'),
                 (0o004, 'r'), (0o002, 'w'), (0o001, 'x')];
    for (bit, ch) in types {
        s.push(if mode & bit != 0 { ch } else { '-' });
    }
    s
}

fn mime_hint(name: &str) -> String {
    let ext = Path::new(name).extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "md" | "markdown" => "text/markdown",
        "csv" => "text/csv",
        "json" => "application/json",
        "toml" => "application/toml",
        "yaml" | "yml" => "application/yaml",
        "xml" | "html" | "htm" => "text/html",
        "js" | "mjs" | "cjs" => "text/javascript",
        "ts" | "tsx" => "text/typescript",
        "rs" => "text/rust",
        "py" => "text/python",
        "rb" => "text/ruby",
        "go" => "text/go",
        "java" => "text/java",
        "c" | "h" => "text/c",
        "cpp" | "cc" | "cxx" | "hpp" => "text/cpp",
        "css" | "scss" | "less" => "text/css",
        "sh" | "bash" | "zsh" | "fish" => "text/shell",
        "sql" => "text/sql",
        "svelte" => "text/svelte",
        "vue" => "text/vue",
        "txt" | "log" | "env" | "gitignore" | "dockerignore" => "text/plain",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        "zip" | "tar" | "gz" | "bz2" | "xz" | "7z" => "application/archive",
        _ => "application/octet-stream",
    }.to_string()
}

fn is_text_file(path: &Path, name: &str) -> bool {
    let mime = mime_hint(name);
    if mime.starts_with("text/") || mime == "application/json" || mime == "application/toml" || mime == "application/yaml" {
        return true;
    }
    // Check first 512 bytes for binary content
    if let Ok(bytes) = fs::read(path) {
        let check = &bytes[..bytes.len().min(512)];
        return !check.iter().any(|&b| b == 0);
    }
    false
}

pub fn get_cwd(session: &str) -> Result<String, String> {
    // Get the CWD of the active pane in the session
    let output = std::process::Command::new("tmux")
        .args(["display-message", "-t", session, "-p", "#{pane_current_path}"])
        .output()
        .map_err(|e| format!("tmux error: {}", e))?;
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        Ok(dirs::home_dir().map(|h| h.to_string_lossy().to_string()).unwrap_or_else(|| "/".to_string()))
    } else {
        Ok(path)
    }
}

pub fn list_dir(path: &str, show_hidden: bool) -> Result<Vec<FileEntry>, String> {
    let dir = resolve_path(path);
    let entries = fs::read_dir(&dir).map_err(|e| format!("Cannot read {}: {}", dir.display(), e))?;

    let mut result: Vec<FileEntry> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let hidden = name.starts_with('.');
        if !show_hidden && hidden { continue; }

        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let file_type = if meta.is_dir() { "dir" } else if meta.file_type().is_symlink() { "symlink" } else { "file" };
        let modified = meta.modified().ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs()).unwrap_or(0);
        let mode = meta.permissions().mode();

        result.push(FileEntry {
            path: entry.path().to_string_lossy().to_string(),
            name,
            file_type: file_type.to_string(),
            size: meta.len(),
            modified,
            permissions: format_permissions(mode),
            hidden,
        });
    }

    // Sort: dirs first, then alphabetical (case-insensitive)
    result.sort_by(|a, b| {
        let dir_ord = (a.file_type != "dir").cmp(&(b.file_type != "dir"));
        dir_ord.then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Ok(result)
}

pub fn stat_file(path: &str) -> Result<FileStat, String> {
    let p = resolve_path(path);
    let meta = fs::metadata(&p).map_err(|e| format!("stat error: {}", e))?;
    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let file_type = if meta.is_dir() { "dir" } else { "file" };
    let mode = meta.permissions().mode();
    let modified = meta.modified().ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs()).unwrap_or(0);
    let is_text = if meta.is_file() { is_text_file(&p, &name) } else { false };

    Ok(FileStat {
        path: p.to_string_lossy().to_string(),
        name,
        file_type: file_type.to_string(),
        size: meta.len(),
        modified,
        permissions: format_permissions(mode),
        readable: mode & 0o400 != 0,
        writable: mode & 0o200 != 0,
        is_text,
        mime_hint: mime_hint(&p.to_string_lossy()),
    })
}

pub fn read_file(path: &str) -> Result<String, String> {
    let p = resolve_path(path);
    let meta = fs::metadata(&p).map_err(|e| format!("read error: {}", e))?;
    if meta.len() > MAX_PREVIEW_SIZE {
        return Err(format!("File too large for preview: {} bytes (max {})", meta.len(), MAX_PREVIEW_SIZE));
    }
    fs::read_to_string(&p).map_err(|e| format!("read error: {}", e))
}

pub fn write_file(path: &str, content: &str) -> Result<(), String> {
    let p = resolve_path(path);
    fs::write(&p, content).map_err(|e| format!("write error: {}", e))
}

pub fn create_dir(path: &str) -> Result<(), String> {
    let p = resolve_path(path);
    fs::create_dir_all(&p).map_err(|e| format!("mkdir error: {}", e))
}

pub fn delete_path(path: &str) -> Result<(), String> {
    let p = resolve_path(path);
    if p.is_dir() {
        fs::remove_dir_all(&p).map_err(|e| format!("delete error: {}", e))
    } else {
        fs::remove_file(&p).map_err(|e| format!("delete error: {}", e))
    }
}

pub fn rename_path(from: &str, to: &str) -> Result<(), String> {
    let f = resolve_path(from);
    let t = resolve_path(to);
    fs::rename(&f, &t).map_err(|e| format!("rename error: {}", e))
}

pub fn download_file(path: &str) -> Result<(String, String), String> {
    let p = resolve_path(path);
    let meta = fs::metadata(&p).map_err(|e| format!("download error: {}", e))?;
    if meta.len() > MAX_READ_SIZE {
        return Err(format!("File too large: {} bytes (max {})", meta.len(), MAX_READ_SIZE));
    }
    let bytes = fs::read(&p).map_err(|e| format!("download error: {}", e))?;
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    Ok((name, b64))
}

pub fn upload_file(path: &str, data_b64: &str) -> Result<(), String> {
    let p = resolve_path(path);
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(data_b64)
        .map_err(|e| format!("invalid base64: {}", e))?;
    fs::write(&p, &bytes).map_err(|e| format!("upload error: {}", e))
}
