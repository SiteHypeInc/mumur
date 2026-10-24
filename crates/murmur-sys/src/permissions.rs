//! OS permission checks. Only macOS gates these behind user consent.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status {
    /// Needed to paste and to read the focused field.
    pub accessibility: bool,
    /// Needed for the global hotkey.
    pub input_monitoring: bool,
}

pub fn status(prompt: bool) -> Status {
    #[cfg(target_os = "macos")]
    {
        Status {
            accessibility: crate::macos::accessibility_trusted(prompt),
            input_monitoring: crate::macos::input_monitoring_granted(prompt),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = prompt;
        Status { accessibility: true, input_monitoring: true }
    }
}
