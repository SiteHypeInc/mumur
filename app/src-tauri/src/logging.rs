//! Logs to stderr and to a file users can send us (Copy diagnostics):
//! macOS ~/Library/Logs/Murmur/murmur.log, Windows %LOCALAPPDATA%\Murmur\logs,
//! Linux ~/.local/state/murmur.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

pub fn log_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let dir = if cfg!(target_os = "macos") {
        home?.join("Library/Logs/Murmur")
    } else if cfg!(target_os = "windows") {
        PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Murmur").join("logs")
    } else {
        home?.join(".local/state/murmur")
    };
    Some(dir.join("murmur.log"))
}

struct Tee(Mutex<Option<std::fs::File>>);

impl Write for Tee {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let _ = std::io::stderr().write_all(buf);
        if let Some(f) = self.0.lock().unwrap().as_mut() {
            let _ = f.write_all(buf);
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        if let Some(f) = self.0.lock().unwrap().as_mut() {
            let _ = f.flush();
        }
        Ok(())
    }
}

pub fn init() {
    let file = log_path().and_then(|p| {
        std::fs::create_dir_all(p.parent()?).ok()?;
        // Start fresh past 1 MB so the file never grows unbounded.
        let big = std::fs::metadata(&p).map(|m| m.len() > 1_000_000).unwrap_or(false);
        std::fs::OpenOptions::new().create(true).append(!big).write(true).truncate(big).open(&p).ok()
    });
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .target(env_logger::Target::Pipe(Box::new(Tee(Mutex::new(file)))))
        .init();
}

/// The last `n` lines of the log, for the diagnostics report.
pub fn tail(n: usize) -> String {
    let Some(text) = log_path().and_then(|p| std::fs::read_to_string(p).ok()) else { return String::new() };
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}
