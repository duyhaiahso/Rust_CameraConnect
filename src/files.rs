use crate::clock::Timestamp;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const APP_DIRECTORY: &str = "BaslerCameraTest";

fn app_data_dir() -> io::Result<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?);
    Ok(base.join(APP_DIRECTORY))
}

pub fn capture_dir() -> io::Result<PathBuf> {
    let base = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?);
    let path = base.join("Pictures").join(APP_DIRECTORY);
    fs::create_dir_all(&path)?;
    Ok(path)
}

pub fn next_capture_path(dir: &Path, timestamp: &Timestamp) -> io::Result<PathBuf> {
    for suffix in 0..1000 {
        let name = if suffix == 0 {
            format!("Basler_{}.png", timestamp.filename_key())
        } else {
            format!("Basler_{}_{suffix}.png", timestamp.filename_key())
        };
        let path = dir.join(name);
        if !path.exists() {
            return Ok(path);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "No free capture filename is available",
    ))
}

pub fn read_vietnamese_setting() -> bool {
    let Ok(path) = app_data_dir().map(|dir| dir.join("language.txt")) else {
        return false;
    };
    matches!(fs::read_to_string(path), Ok(value) if value.trim() == "vi")
}

pub fn save_vietnamese_setting(enabled: bool) -> io::Result<()> {
    let dir = app_data_dir()?;
    fs::create_dir_all(&dir)?;
    fs::write(dir.join("language.txt"), if enabled { "vi" } else { "en" })
}

pub struct Logger {
    directory: PathBuf,
}

impl Logger {
    pub fn new() -> io::Result<Self> {
        let directory = app_data_dir()?.join("logs");
        fs::create_dir_all(&directory)?;
        prune_old_logs(&directory);
        Ok(Self { directory })
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn write(&self, timestamp: &Timestamp, level: &str, message: &str) -> io::Result<()> {
        let path = self
            .directory
            .join(format!("{}.jsonl", timestamp.date_key()));
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        writeln!(
            file,
            "{{\"timestamp\":\"{}\",\"level\":\"{}\",\"source\":\"camera\",\"message\":\"{}\"}}",
            timestamp.iso8601(),
            escape_json(level),
            escape_json(message)
        )
    }
}

fn escape_json(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            ch if ch < ' ' => escaped.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => escaped.push(ch),
        }
    }
    escaped
}

fn prune_old_logs(directory: &Path) {
    let now = SystemTime::now();
    let retention = Duration::from_secs(183 * 86_400);
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !matches!(
            path.extension().and_then(|part| part.to_str()),
            Some("jsonl" | "log")
        ) {
            continue;
        }
        let expired = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > retention);
        if expired {
            let _ = fs::remove_file(path);
        }
    }
}
