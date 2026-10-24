//! Turns raw key press/release events into start/stop decisions.
//!
//! The OS layer only reports "the dictation key went down/up at time t".
//! Everything about push-to-talk vs. toggle vs. hybrid lives here so it can be
//! tested without a keyboard.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ActivationMode {
    /// Press to start, press again to stop.
    Toggle,
    /// Record only while the key is held.
    PushToTalk,
    /// Hold past the threshold = push-to-talk; quick tap = latch on until the next press.
    #[default]
    Hybrid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyAction {
    Start,
    Stop,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Idle,
    /// `latched` is true once a hybrid tap has locked recording on.
    Recording { started_at: u64, latched: bool },
}

#[derive(Debug, Clone)]
pub struct HotkeyMachine {
    mode: ActivationMode,
    hold_threshold_ms: u64,
    debounce_ms: u64,
    state: State,
    key_down: bool,
    last_press_at: Option<u64>,
}

impl HotkeyMachine {
    pub fn new(mode: ActivationMode, hold_threshold_ms: u64) -> Self {
        Self {
            mode,
            hold_threshold_ms,
            debounce_ms: 30,
            state: State::Idle,
            key_down: false,
            last_press_at: None,
        }
    }

    pub fn is_recording(&self) -> bool {
        matches!(self.state, State::Recording { .. })
    }

    /// Call when recording ended for a reason other than the hotkey
    /// (cancel key, error, max duration).
    pub fn reset(&mut self) {
        self.state = State::Idle;
    }

    pub fn on_press(&mut self, now_ms: u64) -> HotkeyAction {
        // OS key-repeat sends presses while the key is held; only the first counts.
        if self.key_down {
            return HotkeyAction::None;
        }
        self.key_down = true;
        if let Some(prev) = self.last_press_at {
            if now_ms.saturating_sub(prev) < self.debounce_ms {
                return HotkeyAction::None;
            }
        }
        self.last_press_at = Some(now_ms);

        match self.state {
            State::Idle => {
                self.state = State::Recording { started_at: now_ms, latched: false };
                HotkeyAction::Start
            }
            State::Recording { latched, .. } => match self.mode {
                ActivationMode::Toggle => self.stop(),
                ActivationMode::Hybrid if latched => self.stop(),
                // A press while a push-to-talk recording is live can only happen if a
                // release was lost; treat it as the stop the user meant.
                ActivationMode::PushToTalk | ActivationMode::Hybrid => self.stop(),
            },
        }
    }

    pub fn on_release(&mut self, now_ms: u64) -> HotkeyAction {
        if !self.key_down {
            return HotkeyAction::None;
        }
        self.key_down = false;

        let State::Recording { started_at, latched } = self.state else {
            return HotkeyAction::None;
        };
        match self.mode {
            ActivationMode::Toggle => HotkeyAction::None,
            ActivationMode::PushToTalk => self.stop(),
            ActivationMode::Hybrid => {
                if latched {
                    HotkeyAction::None
                } else if now_ms.saturating_sub(started_at) >= self.hold_threshold_ms {
                    self.stop()
                } else {
                    self.state = State::Recording { started_at, latched: true };
                    HotkeyAction::None
                }
            }
        }
    }

    fn stop(&mut self) -> HotkeyAction {
        self.state = State::Idle;
        HotkeyAction::Stop
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use HotkeyAction::*;

    #[test]
    fn toggle_ignores_release() {
        let mut m = HotkeyMachine::new(ActivationMode::Toggle, 300);
        assert_eq!(m.on_press(0), Start);
        assert_eq!(m.on_release(50), None);
        assert_eq!(m.on_press(1000), Stop);
        assert_eq!(m.on_release(1050), None);
        assert!(!m.is_recording());
    }

    #[test]
    fn push_to_talk_stops_on_release() {
        let mut m = HotkeyMachine::new(ActivationMode::PushToTalk, 300);
        assert_eq!(m.on_press(0), Start);
        assert_eq!(m.on_release(40), Stop);
    }

    #[test]
    fn hybrid_hold_acts_as_push_to_talk() {
        let mut m = HotkeyMachine::new(ActivationMode::Hybrid, 300);
        assert_eq!(m.on_press(0), Start);
        assert_eq!(m.on_release(2000), Stop);
    }

    #[test]
    fn hybrid_tap_latches_until_next_press() {
        let mut m = HotkeyMachine::new(ActivationMode::Hybrid, 300);
        assert_eq!(m.on_press(0), Start);
        assert_eq!(m.on_release(120), None);
        assert!(m.is_recording());
        assert_eq!(m.on_press(5000), Stop);
        assert_eq!(m.on_release(5100), None);
        assert_eq!(m.on_press(6000), Start);
    }

    #[test]
    fn key_repeat_is_ignored() {
        let mut m = HotkeyMachine::new(ActivationMode::Hybrid, 300);
        assert_eq!(m.on_press(0), Start);
        for t in (500..1500).step_by(33) {
            assert_eq!(m.on_press(t), None);
        }
        assert_eq!(m.on_release(1600), Stop);
    }

    #[test]
    fn bounce_is_debounced() {
        let mut m = HotkeyMachine::new(ActivationMode::Toggle, 300);
        assert_eq!(m.on_press(0), Start);
        assert_eq!(m.on_release(5), None);
        assert_eq!(m.on_press(12), None); // contact bounce
        assert!(m.is_recording());
    }

    #[test]
    fn reset_returns_to_idle() {
        let mut m = HotkeyMachine::new(ActivationMode::Toggle, 300);
        m.on_press(0);
        m.on_release(10);
        m.reset();
        assert_eq!(m.on_press(1000), Start);
    }
}
