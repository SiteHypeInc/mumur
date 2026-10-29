//! macOS permission housekeeping.
//!
//! Unsigned (ad-hoc) builds get a new code identity every release. macOS keeps
//! the old Accessibility / Input Monitoring entries, so System Settings shows
//! "Murmur" switched ON while the new copy is not actually allowed, and
//! there's no way for the user to tell. On the first launch of a new build we
//! clear Murmur's own stale entries so macOS asks again for the real app.

use std::path::Path;

/// Something that changes whenever the app binary is replaced.
fn build_fingerprint() -> String {
    let Ok(exe) = std::env::current_exe() else { return String::new() };
    let Ok(meta) = std::fs::metadata(&exe) else { return String::new() };
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{}:{}:{}", env!("CARGO_PKG_VERSION"), meta.len(), mtime)
}

/// Returns true if stale grants were cleared.
pub fn clear_stale_grants(data_dir: &Path, bundle_id: &str) -> bool {
    let marker = data_dir.join("last-build");
    let now = build_fingerprint();
    let before = std::fs::read_to_string(&marker).unwrap_or_default();
    let _ = std::fs::create_dir_all(data_dir);
    let _ = std::fs::write(&marker, &now);
    if !cfg!(target_os = "macos") || before.trim() == now {
        return false;
    }
    let status = murmur_sys::permissions::status(false);
    if status.accessibility && status.input_monitoring {
        return false; // the grants carried over; nothing to fix
    }
    log::info!("new build without working permissions; clearing stale macOS grants for {bundle_id}");
    for service in ["Accessibility", "ListenEvent", "Microphone"] {
        let _ = std::process::Command::new("/usr/bin/tccutil").args(["reset", service, bundle_id]).status();
    }
    true
}
