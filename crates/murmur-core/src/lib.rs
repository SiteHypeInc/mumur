//! Murmur's platform-free core. Nothing in here touches the microphone,
//! keyboard or clipboard; the `murmur-sys` crate does that and feeds this.

pub mod audio;
pub mod config;
pub mod context;
pub mod dictionary;
pub mod error;
#[cfg(feature = "history")]
pub mod history;
pub mod hotkey;
pub mod learn;
pub mod modes;
pub mod pipeline;
pub mod prompt;
pub mod providers;
pub mod text;

pub use error::{Error, Result};
