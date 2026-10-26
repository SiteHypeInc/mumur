//! Platform-neutral key model and the binding matcher shared by every OS
//! backend. Backends translate native events into `Key` and feed `Matcher`.

use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Fn,
    LeftCtrl,
    RightCtrl,
    LeftAlt,
    RightAlt,
    LeftShift,
    RightShift,
    LeftMeta,
    RightMeta,
    Space,
    Escape,
    Enter,
    Tab,
    Backspace,
    F(u8),
    Char(char),
    Other(u32),
}

impl Key {
    pub fn is_modifier(self) -> bool {
        matches!(
            self,
            Key::Fn
                | Key::LeftCtrl
                | Key::RightCtrl
                | Key::LeftAlt
                | Key::RightAlt
                | Key::LeftShift
                | Key::RightShift
                | Key::LeftMeta
                | Key::RightMeta
        )
    }
}

/// One element of a binding. Generic modifiers match either side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Part {
    Ctrl,
    Alt,
    Shift,
    Meta,
    Exact(Key),
}

impl Part {
    fn satisfied_by(self, down: &HashSet<Key>) -> bool {
        match self {
            Part::Ctrl => down.contains(&Key::LeftCtrl) || down.contains(&Key::RightCtrl),
            Part::Alt => down.contains(&Key::LeftAlt) || down.contains(&Key::RightAlt),
            Part::Shift => down.contains(&Key::LeftShift) || down.contains(&Key::RightShift),
            Part::Meta => down.contains(&Key::LeftMeta) || down.contains(&Key::RightMeta),
            Part::Exact(k) => down.contains(&k),
        }
    }

    fn covers(self, key: Key) -> bool {
        match self {
            Part::Ctrl => matches!(key, Key::LeftCtrl | Key::RightCtrl),
            Part::Alt => matches!(key, Key::LeftAlt | Key::RightAlt),
            Part::Shift => matches!(key, Key::LeftShift | Key::RightShift),
            Part::Meta => matches!(key, Key::LeftMeta | Key::RightMeta),
            Part::Exact(k) => k == key,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub parts: Vec<Part>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("unknown key \"{0}\" in hotkey")]
pub struct ParseError(pub String);

impl Binding {
    /// Parse "Fn", "RightOption", "Ctrl+Shift+Space", "Cmd+Alt+D", "F13".
    pub fn parse(s: &str) -> Result<Self, ParseError> {
        let mut parts = Vec::new();
        for raw in s.split('+').map(str::trim).filter(|p| !p.is_empty()) {
            let lower = raw.to_lowercase();
            let part = match lower.as_str() {
                "fn" | "globe" => Part::Exact(Key::Fn),
                "ctrl" | "control" => Part::Ctrl,
                "alt" | "option" | "opt" => Part::Alt,
                "shift" => Part::Shift,
                "cmd" | "command" | "meta" | "super" | "win" => Part::Meta,
                "leftctrl" | "leftcontrol" => Part::Exact(Key::LeftCtrl),
                "rightctrl" | "rightcontrol" => Part::Exact(Key::RightCtrl),
                "leftalt" | "leftoption" => Part::Exact(Key::LeftAlt),
                "rightalt" | "rightoption" | "altgr" => Part::Exact(Key::RightAlt),
                "leftshift" => Part::Exact(Key::LeftShift),
                "rightshift" => Part::Exact(Key::RightShift),
                "leftcmd" | "leftcommand" | "leftmeta" => Part::Exact(Key::LeftMeta),
                "rightcmd" | "rightcommand" | "rightmeta" => Part::Exact(Key::RightMeta),
                "space" => Part::Exact(Key::Space),
                "esc" | "escape" => Part::Exact(Key::Escape),
                "enter" | "return" => Part::Exact(Key::Enter),
                "tab" => Part::Exact(Key::Tab),
                f if f.starts_with('f') && f.len() > 1 && f[1..].parse::<u8>().is_ok_and(|n| (1..=24).contains(&n)) => {
                    Part::Exact(Key::F(f[1..].parse().unwrap()))
                }
                c if c.chars().count() == 1 && c.chars().all(|c| c.is_ascii_alphanumeric()) => {
                    Part::Exact(Key::Char(c.chars().next().unwrap()))
                }
                _ => return Err(ParseError(raw.to_owned())),
            };
            if !parts.contains(&part) {
                parts.push(part);
            }
        }
        if parts.is_empty() {
            return Err(ParseError(s.to_owned()));
        }
        Ok(Self { parts })
    }

    /// Modifier-only bindings (Fn, RightOption, Ctrl+Alt) can't be swallowed;
    /// bindings with a regular key should be, so "Ctrl+Space" doesn't type a space.
    pub fn trigger_key(&self) -> Option<Key> {
        self.parts.iter().find_map(|p| match p {
            Part::Exact(k) if !k.is_modifier() => Some(*k),
            _ => None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Pressed,
    Released,
    /// Another key was pressed while a modifier-only binding was held: the
    /// user is typing a shortcut (Fn+Left, ⌥+letter), not dictating.
    Interrupted,
}

/// Tracks held keys and reports when the binding becomes fully held / stops
/// being held. For modifier-only bindings, pressing any other key while held
/// cancels the activation (so Fn+Arrow or ⌥+letter typing never triggers).
#[derive(Debug)]
pub struct Matcher {
    binding: Binding,
    down: HashSet<Key>,
    active: bool,
    /// Set when another key was pressed during a modifier-only hold.
    spoiled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Let the event through.
    Pass,
    /// Event belongs to the binding; backends may swallow it.
    Consume,
}

impl Matcher {
    pub fn new(binding: Binding) -> Self {
        Self { binding, down: HashSet::new(), active: false, spoiled: false }
    }

    pub fn binding(&self) -> &Binding {
        &self.binding
    }

    fn held(&self) -> bool {
        self.binding.parts.iter().all(|p| p.satisfied_by(&self.down))
    }

    fn is_part(&self, key: Key) -> bool {
        self.binding.parts.iter().any(|p| p.covers(key))
    }

    /// Returns the edge (if the binding changed state) and whether to swallow the event.
    pub fn on_key(&mut self, key: Key, pressed: bool) -> (Option<Edge>, Verdict) {
        let modifier_only = self.binding.trigger_key().is_none();
        if pressed {
            let repeat = !self.down.insert(key);
            if !self.is_part(key) {
                if self.active && modifier_only && !self.spoiled {
                    self.spoiled = true;
                    return (Some(Edge::Interrupted), Verdict::Pass);
                }
                return (None, Verdict::Pass);
            }
            if !self.active && self.held() {
                self.active = true;
                self.spoiled = false;
                let v = if Some(key) == self.binding.trigger_key() { Verdict::Consume } else { Verdict::Pass };
                return (Some(Edge::Pressed), v);
            }
            let v = if self.active && Some(key) == self.binding.trigger_key() && repeat { Verdict::Consume } else { Verdict::Pass };
            (None, v)
        } else {
            self.down.remove(&key);
            if self.active && self.is_part(key) && !self.held() {
                self.active = false;
                let v = if Some(key) == self.binding.trigger_key() { Verdict::Consume } else { Verdict::Pass };
                return (Some(Edge::Released), v);
            }
            (None, Verdict::Pass)
        }
    }

    /// Any modifier physically held right now (as far as we've seen).
    pub fn modifiers_down(&self) -> bool {
        self.down.iter().any(|k| k.is_modifier())
    }

    /// True if the current/last hold had other keys pressed during it.
    pub fn was_spoiled(&self) -> bool {
        self.spoiled
    }

    /// Forget held keys, e.g. after the tap was disabled and events were missed.
    pub fn reset(&mut self) {
        self.down.clear();
        self.active = false;
        self.spoiled = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bindings() {
        assert_eq!(Binding::parse("Fn").unwrap().parts, vec![Part::Exact(Key::Fn)]);
        assert_eq!(
            Binding::parse("Ctrl+Shift+Space").unwrap().parts,
            vec![Part::Ctrl, Part::Shift, Part::Exact(Key::Space)]
        );
        assert_eq!(Binding::parse("RightOption").unwrap().parts, vec![Part::Exact(Key::RightAlt)]);
        assert_eq!(Binding::parse("f13").unwrap().trigger_key(), Some(Key::F(13)));
        assert!(Binding::parse("Hyper+Q").is_err());
        assert!(Binding::parse("").is_err());
    }

    #[test]
    fn modifier_only_press_release() {
        let mut m = Matcher::new(Binding::parse("Fn").unwrap());
        assert_eq!(m.on_key(Key::Fn, true), (Some(Edge::Pressed), Verdict::Pass));
        assert_eq!(m.on_key(Key::Fn, false), (Some(Edge::Released), Verdict::Pass));
    }

    #[test]
    fn combo_swallows_trigger_key() {
        let mut m = Matcher::new(Binding::parse("Ctrl+Space").unwrap());
        assert_eq!(m.on_key(Key::LeftCtrl, true), (None, Verdict::Pass));
        assert_eq!(m.on_key(Key::Space, true), (Some(Edge::Pressed), Verdict::Consume));
        assert_eq!(m.on_key(Key::Space, true), (None, Verdict::Consume)); // auto-repeat
        assert_eq!(m.on_key(Key::Space, false), (Some(Edge::Released), Verdict::Consume));
        assert_eq!(m.on_key(Key::LeftCtrl, false), (None, Verdict::Pass));
    }

    #[test]
    fn releasing_the_modifier_first_also_releases() {
        let mut m = Matcher::new(Binding::parse("Ctrl+Space").unwrap());
        m.on_key(Key::RightCtrl, true);
        m.on_key(Key::Space, true);
        assert_eq!(m.on_key(Key::RightCtrl, false).0, Some(Edge::Released));
        assert_eq!(m.on_key(Key::Space, false).0, None);
    }

    #[test]
    fn side_specific_binding_ignores_other_side() {
        let mut m = Matcher::new(Binding::parse("RightOption").unwrap());
        assert_eq!(m.on_key(Key::LeftAlt, true).0, None);
        assert_eq!(m.on_key(Key::RightAlt, true).0, Some(Edge::Pressed));
    }

    #[test]
    fn chord_during_modifier_hold_is_spoiled() {
        let mut m = Matcher::new(Binding::parse("Fn").unwrap());
        m.on_key(Key::Fn, true);
        assert_eq!(m.on_key(Key::Other(123), true).0, Some(Edge::Interrupted));
        assert_eq!(m.on_key(Key::Other(124), true).0, None);
        assert!(m.was_spoiled());
        assert_eq!(m.on_key(Key::Fn, false).0, Some(Edge::Released));
        assert_eq!(m.on_key(Key::Fn, true).0, Some(Edge::Pressed));
        assert!(!m.was_spoiled());
    }
}
