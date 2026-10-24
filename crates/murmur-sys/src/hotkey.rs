//! Global hotkey: native key events in, `HotkeyEvent`s out.

use std::sync::{Arc, Mutex};

use crate::keys::{Binding, Edge, Key, Matcher, Verdict};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEvent {
    Pressed,
    Released,
    /// A modifier-only hold turned into a keyboard shortcut; discard the recording.
    Interrupted,
    /// The cancel key (Escape) while armed.
    Cancel,
}

#[derive(Debug, thiserror::Error)]
pub enum HotkeyError {
    #[error("{0}")]
    Parse(#[from] crate::keys::ParseError),
    #[error("{0}")]
    Permission(String),
    #[error("hotkey backend failed: {0}")]
    Backend(String),
}

type Sink = Box<dyn Fn(HotkeyEvent) + Send>;

/// State shared with the OS callback thread.
pub struct Dispatch {
    matcher: Matcher,
    cancel: Option<Matcher>,
    cancel_armed: bool,
    sink: Sink,
}

impl Dispatch {
    /// Returns true when the event should be swallowed.
    pub(crate) fn on_key(&mut self, key: Key, pressed: bool) -> bool {
        let (edge, verdict) = self.matcher.on_key(key, pressed);
        if let Some(edge) = edge {
            (self.sink)(match edge {
                Edge::Pressed => HotkeyEvent::Pressed,
                Edge::Released => HotkeyEvent::Released,
                Edge::Interrupted => HotkeyEvent::Interrupted,
            });
        }
        let mut swallow = verdict == Verdict::Consume;
        if let Some(c) = self.cancel.as_mut() {
            let (edge, _) = c.on_key(key, pressed);
            if self.cancel_armed && edge == Some(Edge::Pressed) {
                (self.sink)(HotkeyEvent::Cancel);
                swallow = true;
            }
        }
        swallow
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(crate) fn reset(&mut self) {
        self.matcher.reset();
        if let Some(c) = self.cancel.as_mut() {
            c.reset();
        }
    }
}

pub struct Hotkeys {
    dispatch: Arc<Mutex<Dispatch>>,
}

impl Hotkeys {
    /// Starts listening. `sink` is called on the OS event thread; keep it cheap
    /// (send to a channel).
    pub fn start(binding: &str, cancel: &str, sink: impl Fn(HotkeyEvent) + Send + 'static) -> Result<Self, HotkeyError> {
        let dispatch = Arc::new(Mutex::new(Dispatch {
            matcher: Matcher::new(Binding::parse(binding)?),
            cancel: Binding::parse(cancel).ok().map(Matcher::new),
            cancel_armed: false,
            sink: Box::new(sink),
        }));
        backend::start_listener(dispatch.clone())?;
        Ok(Self { dispatch })
    }

    pub fn set_binding(&self, binding: &str) -> Result<(), HotkeyError> {
        let b = Binding::parse(binding)?;
        let mut d = self.dispatch.lock().unwrap();
        d.matcher = Matcher::new(b);
        Ok(())
    }

    /// Only swallow Escape while a recording is live.
    pub fn arm_cancel(&self, armed: bool) {
        self.dispatch.lock().unwrap().cancel_armed = armed;
    }
}

#[cfg(target_os = "macos")]
mod backend {
    pub use crate::macos::start_listener;
}

#[cfg(not(target_os = "macos"))]
mod backend {
    use super::*;

    fn map(k: rdev::Key) -> Key {
        use rdev::Key as R;
        match k {
            R::ControlLeft => Key::LeftCtrl,
            R::ControlRight => Key::RightCtrl,
            R::Alt => Key::LeftAlt,
            R::AltGr => Key::RightAlt,
            R::ShiftLeft => Key::LeftShift,
            R::ShiftRight => Key::RightShift,
            R::MetaLeft => Key::LeftMeta,
            R::MetaRight => Key::RightMeta,
            R::Function => Key::Fn,
            R::Space => Key::Space,
            R::Escape => Key::Escape,
            R::Return | R::KpReturn => Key::Enter,
            R::Tab => Key::Tab,
            R::Backspace => Key::Backspace,
            R::F1 => Key::F(1), R::F2 => Key::F(2), R::F3 => Key::F(3), R::F4 => Key::F(4),
            R::F5 => Key::F(5), R::F6 => Key::F(6), R::F7 => Key::F(7), R::F8 => Key::F(8),
            R::F9 => Key::F(9), R::F10 => Key::F(10), R::F11 => Key::F(11), R::F12 => Key::F(12),
            R::KeyA => Key::Char('a'), R::KeyB => Key::Char('b'), R::KeyC => Key::Char('c'),
            R::KeyD => Key::Char('d'), R::KeyE => Key::Char('e'), R::KeyF => Key::Char('f'),
            R::KeyG => Key::Char('g'), R::KeyH => Key::Char('h'), R::KeyI => Key::Char('i'),
            R::KeyJ => Key::Char('j'), R::KeyK => Key::Char('k'), R::KeyL => Key::Char('l'),
            R::KeyM => Key::Char('m'), R::KeyN => Key::Char('n'), R::KeyO => Key::Char('o'),
            R::KeyP => Key::Char('p'), R::KeyQ => Key::Char('q'), R::KeyR => Key::Char('r'),
            R::KeyS => Key::Char('s'), R::KeyT => Key::Char('t'), R::KeyU => Key::Char('u'),
            R::KeyV => Key::Char('v'), R::KeyW => Key::Char('w'), R::KeyX => Key::Char('x'),
            R::KeyY => Key::Char('y'), R::KeyZ => Key::Char('z'),
            R::Num0 => Key::Char('0'), R::Num1 => Key::Char('1'), R::Num2 => Key::Char('2'),
            R::Num3 => Key::Char('3'), R::Num4 => Key::Char('4'), R::Num5 => Key::Char('5'),
            R::Num6 => Key::Char('6'), R::Num7 => Key::Char('7'), R::Num8 => Key::Char('8'),
            R::Num9 => Key::Char('9'),
            R::Unknown(code) => Key::Other(code),
            _ => Key::Other(u32::MAX),
        }
    }

    /// rdev's passive listener: a low-level hook on Windows, XRecord on X11.
    /// It can't swallow events, so non-mac defaults are modifier-only (Ctrl+Win).
    pub fn start_listener(dispatch: Arc<Mutex<Dispatch>>) -> Result<(), HotkeyError> {
        std::thread::Builder::new()
            .name("murmur-hotkey".into())
            .spawn(move || {
                let res = rdev::listen(move |ev| {
                    let (key, pressed) = match ev.event_type {
                        rdev::EventType::KeyPress(k) => (map(k), true),
                        rdev::EventType::KeyRelease(k) => (map(k), false),
                        _ => return,
                    };
                    dispatch.lock().unwrap().on_key(key, pressed);
                });
                if let Err(e) = res {
                    log::error!("hotkey listener stopped: {e:?}");
                }
            })
            .map_err(|e| HotkeyError::Backend(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn dispatch_emits_events_and_cancel_only_when_armed() {
        let (tx, rx) = mpsc::channel();
        let mut d = Dispatch {
            matcher: Matcher::new(Binding::parse("RightOption").unwrap()),
            cancel: Binding::parse("Escape").ok().map(Matcher::new),
            cancel_armed: false,
            sink: Box::new(move |e| tx.send(e).unwrap()),
        };
        assert!(!d.on_key(Key::Escape, true));
        d.on_key(Key::Escape, false);
        d.on_key(Key::RightAlt, true);
        d.on_key(Key::RightAlt, false);
        d.cancel_armed = true;
        assert!(d.on_key(Key::Escape, true));
        let got: Vec<_> = rx.try_iter().collect();
        assert_eq!(got, vec![HotkeyEvent::Pressed, HotkeyEvent::Released, HotkeyEvent::Cancel]);
    }
}
