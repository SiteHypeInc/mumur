//! Put text into whatever app has focus: swap the clipboard, send the paste
//! shortcut, then put the user's clipboard back, but only if it still holds
//! our text (if the user copied something meanwhile, theirs wins).

use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use arboard::{Clipboard, ImageData};
use murmur_core::modes::SendKey;

#[derive(Debug, thiserror::Error)]
pub enum PasteError {
    #[error("clipboard unavailable: {0}")]
    Clipboard(String),
    #[error("could not send the paste keystroke: {0}")]
    Keystroke(String),
}

#[derive(Debug, Clone)]
pub struct InsertOptions {
    pub restore_clipboard: bool,
    pub restore_delay: Duration,
    pub send_key: Option<SendKey>,
}

enum Saved {
    Text(String),
    Image(ImageData<'static>),
    Empty,
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

pub fn insert_text(text: &str, opts: &InsertOptions) -> Result<(), PasteError> {
    if text.is_empty() {
        return Ok(());
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

    with_clipboard(|cb| cb.set_text(text.to_owned()))?;
    // Give the pasteboard server a beat before the target app reads it.
    std::thread::sleep(Duration::from_millis(25));
    send_paste().map_err(PasteError::Keystroke)?;

    if let Some(key) = opts.send_key {
        std::thread::sleep(Duration::from_millis(60));
        send_key(key).map_err(PasteError::Keystroke)?;
    }

    if opts.restore_clipboard {
        let ours = text.to_owned();
        let delay = opts.restore_delay;
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            let still_ours = with_clipboard(|cb| cb.get_text()).is_ok_and(|t| t == ours);
            if !still_ours {
                return;
            }
            let res = match saved {
                Saved::Text(t) => with_clipboard(|cb| cb.set_text(t)),
                Saved::Image(img) => with_clipboard(|cb| cb.set_image(img)),
                Saved::Empty => with_clipboard(|cb| cb.clear()),
            };
            if let Err(e) = res {
                log::warn!("clipboard restore failed: {e}");
            }
        });
    }
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
