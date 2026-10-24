//! Everything Murmur needs from the operating system. Each module hides the
//! per-OS details behind one small API.

pub mod capture;
pub mod frontmost;
pub mod hotkey;
pub mod keys;
pub mod paste;
pub mod permissions;

#[cfg(target_os = "macos")]
mod macos;
