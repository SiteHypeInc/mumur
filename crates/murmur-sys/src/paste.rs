//! Put text into whatever app has focus: swap the clipboard, send the paste
//! shortcut, then put the user's clipboard back.
//!
//! The restore is the dangerous part: put the old clipboard back before the
//! target app has read ours and it pastes the *old* contents. So:
//! - the clipboard write is read back before ⌘V is sent;
//! - on macOS we watch the focused field (Accessibility) and restore only once
//!   the text has visibly landed. If we can't confirm it, the transcript stays
//!   on the clipboard: losing an old clipboard beats losing a dictation;
//! - elsewhere we wait `restore_delay`, and restore only if the clipboard
//!   still holds our text (if the user copied something meanwhile, theirs wins).

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use arboard::{Clipboard, ImageData};
use murmur_core::modes::SendKey;

#[derive(Debug, thiserror::Error)]
pub enum PasteError {
    #[error("clipboard unavailable: {0}")]
    Clipboard(String),
    #[error("could not send the paste keystroke: {0}")]
    Keystroke(String),
    /// macOS drops synthetic keystrokes from apps without Accessibility, silently.
    #[error("Murmur isn't allowed to type yet (Accessibility)")]
    NotAllowed,
}

#[derive(Debug, Clone, Default)]
pub struct InsertOptions {
    pub restore_clipboard: bool,
    /// Minimum wait before restoring (and the whole wait where a paste can't be observed).
    pub restore_delay: Duration,
    pub send_key: Option<SendKey>,
    /// Target app, for the log.
    pub target_app: String,
    /// Target pid, so Chrome/Electron apps can be asked for their Accessibility tree.
    pub target_pid: Option<i32>,
}

/// How long to watch the field for the paste to land before giving up.
const CONFIRM_WINDOW: Duration = Duration::from_millis(2000);

enum Saved {
    Text(String),
    Image(ImageData<'static>),
    Empty,
}

impl Saved {
    /// What we'd restore, without the contents (they stay out of the log).
    fn describe(&self) -> String {
        match self {
            Saved::Text(t) => format!("text, {} chars", t.chars().count()),
            Saved::Image(i) => format!("image {}x{}", i.width, i.height),
            Saved::Empty => "empty clipboard".into(),
        }
    }
}

// One long-lived clipboard handle: on X11/Wayland the owner must stay alive
// for other apps to read what we set.
fn clipboard() -> &'static Mutex<Option<Clipboard>> {
    static CB: OnceLock<Mutex<Option<Clipboard>>> = OnceLock::new();
    CB.get_or_init(|| Mutex::new(None))
}

fn with_clipboard<T>(f: impl FnOnce(&mut Clipboard) -> Result<T, arboard::Error>) -> Result<T, PasteError> {
    let mut guard = clipboard().lock().unwrap();
    if guard.is_none() {
        *guard = Some(Clipboard::new().map_err(|e| PasteError::Clipboard(e.to_string()))?);
    }
    f(guard.as_mut().unwrap()).map_err(|e| PasteError::Clipboard(e.to_string()))
}

pub fn copy_to_clipboard(text: &str) -> Result<(), PasteError> {
    with_clipboard(|cb| cb.set_text(text.to_owned()))
}

fn clipboard_holds(text: &str) -> bool {
    with_clipboard(|cb| cb.get_text()).is_ok_and(|t| t == text)
}

/// Write `text` and read it back until the pasteboard really has it.
fn write_verified(text: &str) -> Result<u32, PasteError> {
    for attempt in 1..=4u32 {
        with_clipboard(|cb| cb.set_text(text.to_owned()))?;
        std::thread::sleep(Duration::from_millis(15 * attempt as u64));
        if clipboard_holds(text) {
            return Ok(attempt);
        }
        log::warn!("paste: clipboard read-back didn't match (attempt {attempt})");
    }
    Err(PasteError::Clipboard("the clipboard didn't take the text".into()))
}

/// A short, distinctive piece of the text to look for in the field: its last
/// line, last 40 chars. Fields rewrap and normalize newlines, so not the whole thing.
fn needle(text: &str) -> &str {
    let line = text.trim().lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("");
    let skip = line.chars().count().saturating_sub(40);
    let start = line.char_indices().nth(skip).map(|(i, _)| i).unwrap_or(0);
    &line[start..]
}

/// Did the paste land? The field now holds one more copy of the text than before.
fn landed(before: Option<&str>, after: Option<&str>, text: &str) -> bool {
    let n = needle(text);
    if n.is_empty() {
        return false;
    }
    let count = |v: Option<&str>| v.map_or(0, |v| v.matches(n).count());
    after.is_some() && count(after) > count(before)
}

/// The focused field's text, where the OS lets us read it.
fn field_value() -> Option<String> {
    #[cfg(target_os = "macos")]
    return crate::frontmost::focused_text().0;
    #[cfg(not(target_os = "macos"))]
    None
}

/// Whether this platform can watch a paste land (macOS Accessibility).
const CAN_CONFIRM: bool = cfg!(target_os = "macos");

pub fn insert_text(text: &str, opts: &InsertOptions) -> Result<(), PasteError> {
    if text.is_empty() {
        return Ok(());
    }
    let app = if opts.target_app.is_empty() { "the focused app" } else { opts.target_app.as_str() };
    #[cfg(target_os = "macos")]
    if !crate::macos::accessibility_trusted(false) {
        // Leave the text on the clipboard; the caller tells the user to press ⌘V.
        copy_to_clipboard(text)?;
        log::warn!("paste: no Accessibility permission; transcript left on the clipboard for {app}");
        return Err(PasteError::NotAllowed);
    }
    #[cfg(target_os = "macos")]
    if let Some(pid) = opts.target_pid {
        crate::macos::enable_app_accessibility(pid);
    }
    let saved = if opts.restore_clipboard {
        with_clipboard(|cb| {
            Ok(match cb.get_text() {
                Ok(t) => Saved::Text(t),
                Err(_) => match cb.get_image() {
                    Ok(img) => Saved::Image(img.to_owned_img()),
                    Err(_) => Saved::Empty,
                },
            })
        })?
    } else {
        Saved::Empty
    };
    let before = if opts.restore_clipboard && CAN_CONFIRM { field_value() } else { None };

    let tries = write_verified(text)?;
    log::info!("paste: clipboard write OK ({} chars, verified on try {tries})", text.chars().count());
    send_paste().map_err(PasteError::Keystroke)?;
    let sent = Instant::now();
    log::info!("paste: {} sent to {app}", if cfg!(target_os = "macos") { "Cmd+V" } else { "Ctrl+V" });

    if let Some(key) = opts.send_key {
        std::thread::sleep(Duration::from_millis(60));
        send_key(key).map_err(PasteError::Keystroke)?;
    }

    if !opts.restore_clipboard {
        log::info!("paste: clipboard restore is off; transcript stays on the clipboard");
        return Ok(());
    }
    let ours = text.to_owned();
    let delay = opts.restore_delay;
    let app = app.to_owned();
    std::thread::spawn(move || {
        if CAN_CONFIRM {
            let mut confirmed = false;
            while sent.elapsed() < CONFIRM_WINDOW.max(delay) {
                std::thread::sleep(Duration::from_millis(50));
                if landed(before.as_deref(), field_value().as_deref(), &ours) {
                    confirmed = true;
                    break;
                }
            }
            if !confirmed {
                log::warn!(
                    "paste: couldn't confirm the paste landed in {app} after {} ms; leaving the transcript on the clipboard",
                    sent.elapsed().as_millis()
                );
                return;
            }
            log::info!("paste: confirmed in {app} after {} ms", sent.elapsed().as_millis());
            // The app has read the clipboard; give it a beat to finish, then honor the minimum.
            std::thread::sleep(delay.saturating_sub(sent.elapsed()).max(Duration::from_millis(100)));
        } else {
            std::thread::sleep(delay);
        }
        if !clipboard_holds(&ours) {
            log::info!("paste: clipboard changed since the paste (you copied something); not restoring");
            return;
        }
        let what = saved.describe();
        let res = match saved {
            Saved::Text(t) => with_clipboard(|cb| cb.set_text(t)),
            Saved::Image(img) => with_clipboard(|cb| cb.set_image(img)),
            Saved::Empty => with_clipboard(|cb| cb.clear()),
        };
        match res {
            Ok(()) => log::info!("paste: clipboard restored {} ms after the paste ({what})", sent.elapsed().as_millis()),
            Err(e) => log::warn!("paste: clipboard restore failed: {e}"),
        }
    });
    Ok(())
}

#[cfg(target_os = "macos")]
fn send_paste() -> Result<(), String> {
    crate::macos::send_paste()
}

#[cfg(target_os = "macos")]
fn send_key(key: SendKey) -> Result<(), String> {
    match key {
        SendKey::Enter => crate::macos::send_enter(false, false),
        SendKey::ShiftEnter => crate::macos::send_enter(true, false),
        SendKey::CmdEnter => crate::macos::send_enter(false, true),
    }
}

#[cfg(not(target_os = "macos"))]
fn chord(mods: &[enigo::Key], key: enigo::Key) -> Result<(), String> {
    use enigo::{Direction, Enigo, Keyboard, Settings};
    let mut e = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
    for m in mods {
        e.key(*m, Direction::Press).map_err(|e| e.to_string())?;
    }
    let r = e.key(key, Direction::Click).map_err(|e| e.to_string());
    for m in mods.iter().rev() {
        let _ = e.key(*m, Direction::Release);
    }
    r
}

#[cfg(not(target_os = "macos"))]
fn send_paste() -> Result<(), String> {
    chord(&[enigo::Key::Control], enigo::Key::Unicode('v'))
}

#[cfg(not(target_os = "macos"))]
fn send_key(key: SendKey) -> Result<(), String> {
    use enigo::Key as K;
    match key {
        SendKey::Enter => chord(&[], K::Return),
        SendKey::ShiftEnter => chord(&[K::Shift], K::Return),
        SendKey::CmdEnter => chord(&[K::Control], K::Return),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needle_is_the_tail_of_the_last_line() {
        assert_eq!(needle("  hello there  "), "hello there");
        assert_eq!(needle("first line\nsecond line\n"), "second line");
        let long = "a".repeat(30) + &"b".repeat(40);
        assert_eq!(needle(&long), "b".repeat(40));
        assert_eq!(needle("héllo wörld ünïcode"), "héllo wörld ünïcode");
    }

    #[test]
    fn landed_needs_one_more_copy() {
        let t = "Ship it on Friday. ";
        assert!(landed(Some("Notes: "), Some("Notes: Ship it on Friday. "), t));
        assert!(landed(None, Some("Ship it on Friday."), t));
        // Same text already in the field: only an extra copy counts.
        assert!(!landed(Some("Ship it on Friday."), Some("Ship it on Friday."), t));
        assert!(landed(Some("Ship it on Friday."), Some("Ship it on Friday. Ship it on Friday."), t));
        // Field unreadable, or the old clipboard got pasted instead.
        assert!(!landed(None, None, t));
        assert!(!landed(Some(""), Some("old clipboard"), t));
        assert!(!landed(None, Some("anything"), "   "));
    }

    #[test]
    fn saved_description_omits_contents() {
        assert_eq!(Saved::Text("secret".into()).describe(), "text, 6 chars");
        assert_eq!(Saved::Empty.describe(), "empty clipboard");
    }
}
