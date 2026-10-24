//! Owns the dictation loop: hotkey events -> recorder -> pipeline -> paste.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use murmur_core::config::Settings;
use murmur_core::history::{History, NewEntry};
use murmur_core::hotkey::{HotkeyAction, HotkeyMachine};
use murmur_core::learn;
use murmur_core::pipeline::{Engine, Job, Outcome};
use murmur_sys::capture::Recorder;
use murmur_sys::frontmost::{self, Target};
use murmur_sys::hotkey::{HotkeyEvent, Hotkeys};
use murmur_sys::paste::{self, InsertOptions};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::overlay::{self, PillState};
use crate::secrets;

/// What we learned about the target while the user was talking.
#[derive(Default)]
struct Snapshot {
    target: Target,
    surrounding: Option<String>,
}

struct Session {
    started: Instant,
    snapshot: Arc<Mutex<Option<Snapshot>>>,
    probe: Option<JoinHandle<()>>,
}

pub struct Controller {
    pub app: AppHandle,
    pub settings: RwLock<Settings>,
    pub settings_path: PathBuf,
    pub history: Mutex<Option<History>>,
    recorder: Recorder,
    hotkeys: Mutex<Option<Hotkeys>>,
    machine: Mutex<HotkeyMachine>,
    session: Mutex<Option<Session>>,
    pub paused: AtomicBool,
    /// Increments per recording, so a stale level/timeout thread knows to stop.
    generation: AtomicU64,
    pub hotkey_error: Mutex<Option<String>>,
}

#[derive(Clone, Serialize)]
pub struct DictationEvent {
    pub text: String,
    pub app: Option<String>,
    pub note: Option<String>,
}

fn now_ms(start: Instant) -> u64 {
    start.elapsed().as_millis() as u64
}

impl Controller {
    pub fn new(app: AppHandle, settings_path: PathBuf, history_path: PathBuf) -> Arc<Self> {
        let settings = Settings::load(&settings_path).unwrap_or_else(|e| {
            log::error!("settings unreadable, using defaults: {e}");
            Settings::default()
        });
        let history = History::open(&history_path).map_err(|e| log::error!("history unavailable: {e}")).ok();
        let machine = HotkeyMachine::new(settings.hotkey.mode, settings.hotkey.hold_threshold_ms);
        Arc::new(Self {
            app,
            settings: RwLock::new(settings),
            settings_path,
            history: Mutex::new(history),
            recorder: Recorder::new(),
            hotkeys: Mutex::new(None),
            machine: Mutex::new(machine),
            session: Mutex::new(None),
            paused: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            hotkey_error: Mutex::new(None),
        })
    }

    /// Install the global hotkey and start the event loop thread.
    pub fn start(self: &Arc<Self>) {
        let (tx, rx) = mpsc::channel::<HotkeyEvent>();
        let (binding, cancel) = {
            let s = self.settings.read().unwrap();
            (s.hotkey.binding.clone(), s.hotkey.cancel.clone())
        };
        match Hotkeys::start(&binding, &cancel, move |e| {
            let _ = tx.send(e);
        }) {
            Ok(h) => *self.hotkeys.lock().unwrap() = Some(h),
            Err(e) => {
                log::error!("hotkey unavailable: {e}");
                *self.hotkey_error.lock().unwrap() = Some(e.to_string());
            }
        }
        let me = self.clone();
        std::thread::Builder::new().name("murmur-control".into()).spawn(move || me.event_loop(rx)).expect("control thread");
    }

    pub fn apply_settings(&self, new: Settings) -> Result<(), String> {
        murmur_sys::keys::Binding::parse(&new.hotkey.binding).map_err(|e| e.to_string())?;
        new.save(&self.settings_path).map_err(|e| e.to_string())?;
        if let Some(h) = self.hotkeys.lock().unwrap().as_ref() {
            h.set_binding(&new.hotkey.binding).map_err(|e| e.to_string())?;
        }
        *self.machine.lock().unwrap() = HotkeyMachine::new(new.hotkey.mode, new.hotkey.hold_threshold_ms);
        if let Some(h) = self.history.lock().unwrap().as_ref() {
            let _ = h.prune(new.history_limit);
        }
        let _ = self.app.emit("settings-changed", &new);
        *self.settings.write().unwrap() = new;
        Ok(())
    }

    fn event_loop(self: Arc<Self>, rx: Receiver<HotkeyEvent>) {
        let epoch = Instant::now();
        while let Ok(ev) = rx.recv() {
            if self.paused.load(Ordering::SeqCst) {
                continue;
            }
            let action = {
                let mut m = self.machine.lock().unwrap();
                match ev {
                    HotkeyEvent::Pressed => m.on_press(now_ms(epoch)),
                    HotkeyEvent::Released => m.on_release(now_ms(epoch)),
                    HotkeyEvent::Interrupted | HotkeyEvent::Cancel => {
                        if m.is_recording() {
                            m.reset();
                            drop(m);
                            self.cancel(ev == HotkeyEvent::Cancel);
                        }
                        continue;
                    }
                }
            };
            match action {
                HotkeyAction::Start => self.begin(),
                HotkeyAction::Stop => self.finish(),
                HotkeyAction::None => {}
            }
        }
    }

    /// Start/stop from the tray or the settings window.
    pub fn toggle(self: &Arc<Self>) {
        let recording = {
            let mut m = self.machine.lock().unwrap();
            let was = m.is_recording();
            if was {
                m.reset();
            } else {
                // Put the machine in a latched state: a press, then a quick release.
                m.on_press(0);
                m.on_release(1);
            }
            was
        };
        if recording { self.finish() } else { self.begin() }
    }

    fn begin(self: &Arc<Self>) {
        let device = self.settings.read().unwrap().audio.device.clone();
        if let Err(e) = self.recorder.start(Some(device)) {
            self.machine.lock().unwrap().reset();
            overlay::set(&self.app, PillState::Message { text: e.to_string(), tone: "error" });
            overlay::sound(&self.app, "error");
            self.hide_later(2500);
            return;
        }
        if let Some(h) = self.hotkeys.lock().unwrap().as_ref() {
            h.arm_cancel(true);
        }
        let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;

        // Read the target app while the user talks; it can take a few hundred ms.
        let snapshot = Arc::new(Mutex::new(None));
        let slot = snapshot.clone();
        let read_field = self.settings.read().unwrap().polish.enabled;
        let probe = std::thread::spawn(move || {
            let target = frontmost::target();
            let surrounding = if read_field {
                let (value, selected) = frontmost::focused_text();
                selected.filter(|s| !s.trim().is_empty()).or(value).map(|v| tail(&v, 1500))
            } else {
                None
            };
            *slot.lock().unwrap() = Some(Snapshot { target, surrounding });
        });
        *self.session.lock().unwrap() = Some(Session { started: Instant::now(), snapshot, probe: Some(probe) });

        overlay::set(&self.app, PillState::Recording { mode: None });
        overlay::sound(&self.app, "start");

        // Level meter + max-duration guard.
        let me = self.clone();
        let max = Duration::from_secs(self.settings.read().unwrap().audio.max_recording_secs.max(10));
        std::thread::spawn(move || {
            let started = Instant::now();
            while me.generation.load(Ordering::SeqCst) == gen && me.recorder.is_recording() {
                overlay::level(&me.app, me.recorder.level());
                if started.elapsed() > max {
                    me.machine.lock().unwrap().reset();
                    me.finish();
                    break;
                }
                std::thread::sleep(Duration::from_millis(33));
            }
        });
    }

    fn cancel(&self, by_user: bool) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.recorder.cancel();
        if let Some(h) = self.hotkeys.lock().unwrap().as_ref() {
            h.arm_cancel(false);
        }
        self.session.lock().unwrap().take();
        if by_user {
            overlay::sound(&self.app, "cancel");
        }
        overlay::set(&self.app, PillState::Hidden);
    }

    fn finish(self: &Arc<Self>) {
        let Some(mut session) = self.session.lock().unwrap().take() else { return };
        self.generation.fetch_add(1, Ordering::SeqCst);
        let samples = self.recorder.stop();
        if let Some(h) = self.hotkeys.lock().unwrap().as_ref() {
            h.arm_cancel(false);
        }
        overlay::sound(&self.app, "stop");
        overlay::set(&self.app, PillState::Processing);
        let me = self.clone();
        tauri::async_runtime::spawn(async move {
            if let Some(p) = session.probe.take() {
                let _ = tauri::async_runtime::spawn_blocking(move || p.join()).await;
            }
            let snap = session.snapshot.lock().unwrap().take().unwrap_or_default();
            log::info!("recorded {} ms", session.started.elapsed().as_millis());
            me.process(samples, snap).await;
        });
    }

    async fn process(self: Arc<Self>, samples: Vec<f32>, snap: Snapshot) {
        let settings = self.settings.read().unwrap().clone();
        let stt_key = secrets::get(secrets::stt_key_name(settings.stt.provider));
        let polish_key = secrets::get(secrets::polish_key_name(settings.polish.provider));
        let transcriber = match settings.build_transcriber(stt_key) {
            Ok(t) => t,
            Err(e) => return self.fail(&e.user_message()),
        };
        let engine = Engine { transcriber, polisher: settings.build_polisher(polish_key) };
        log::info!("captured {} samples, {:.1} dBFS", samples.len(), murmur_core::audio::rms_dbfs(&samples));
        let job = Job { samples, context: snap.target.context.clone(), surrounding_text: snap.surrounding };
        let outcome = match engine.run(job, &settings).await {
            Ok(o) => o,
            Err(e) => return self.fail(&e.user_message()),
        };
        log::info!(
            "dictation: {} ms audio, stt {} ms, cleanup {} ms, {:?} in {:?}{}",
            outcome.audio_ms,
            outcome.stt_ms,
            outcome.polish_ms,
            outcome.resolved.category,
            snap.target.context.app_name,
            outcome.note.as_deref().map(|n| format!(" ({n})")).unwrap_or_default()
        );
        if outcome.text.is_empty() {
            let text = outcome.note.clone().unwrap_or_else(|| "Nothing to insert".into());
            overlay::set(&self.app, PillState::Message { text, tone: "muted" });
            return self.hide_later(1400);
        }

        let text = if settings.paste.trailing_space {
            murmur_core::text::with_trailing_space(&outcome.text)
        } else {
            outcome.text.clone()
        };
        let opts = InsertOptions {
            restore_clipboard: settings.paste.restore_clipboard,
            restore_delay: Duration::from_millis(settings.paste.restore_delay_ms),
            send_key: outcome.resolved.send_key,
        };
        let t = text.clone();
        let pasted = tauri::async_runtime::spawn_blocking(move || paste::insert_text(&t, &opts)).await;
        if !matches!(pasted, Ok(Ok(()))) {
            let _ = paste::copy_to_clipboard(&text);
            overlay::set(&self.app, PillState::Message { text: "Copied. Paste with ⌘V / Ctrl+V".into(), tone: "warn" });
            self.hide_later(3000);
        } else {
            match &outcome.note {
                Some(note) => {
                    overlay::set(&self.app, PillState::Message { text: note.clone(), tone: "warn" });
                    self.hide_later(2200);
                }
                None => {
                    let words = outcome.text.split_whitespace().count();
                    overlay::set(&self.app, PillState::Done { words, ms: outcome.stt_ms + outcome.polish_ms });
                    self.hide_later(700);
                }
            }
        }

        self.record(&outcome, &snap.target, &settings);
        if settings.learn_from_edits && cfg!(target_os = "macos") {
            self.watch_for_edits(outcome.text.clone(), snap.target.pid);
        }
    }

    fn record(&self, o: &Outcome, target: &Target, settings: &Settings) {
        let category = format!("{:?}", o.resolved.category).to_lowercase();
        if settings.history_limit > 0 {
            if let Some(h) = self.history.lock().unwrap().as_ref() {
                let _ = h.add(
                    &NewEntry {
                        app: target.context.app_name.as_deref(),
                        category: &category,
                        raw: &o.raw,
                        text: &o.text,
                        audio_ms: o.audio_ms,
                        stt_ms: o.stt_ms,
                        polish_ms: o.polish_ms,
                    },
                    settings.history_limit,
                );
            }
        }
        let _ = self.app.emit(
            "dictation",
            DictationEvent { text: o.text.clone(), app: target.context.app_name.clone(), note: o.note.clone() },
        );
    }

    /// Look at the field a little later; if the user fixed a misheard word,
    /// remember the spelling.
    fn watch_for_edits(self: &Arc<Self>, pasted: String, pid: Option<i32>) {
        let me = self.clone();
        std::thread::spawn(move || {
            for wait in [8u64, 25] {
                std::thread::sleep(Duration::from_secs(wait));
                if frontmost::target().pid != pid {
                    return;
                }
                let (Some(field), _) = frontmost::focused_text() else { return };
                let known = me.settings.read().unwrap().dictionary.terms.clone();
                let learned = learn::learn_from_edit(&pasted, &field, &known);
                if learned.is_empty() {
                    continue;
                }
                let mut s = me.settings.read().unwrap().clone();
                let added = s.dictionary.add_terms(learned);
                if added.is_empty() || me.apply_settings(s).is_err() {
                    return;
                }
                overlay::set(&me.app, PillState::Message { text: format!("Learned: {}", added.join(", ")), tone: "ok" });
                me.hide_later(2200);
                let _ = me.app.emit("learned", added);
                return;
            }
        });
    }

    fn fail(self: &Arc<Self>, message: &str) {
        log::warn!("dictation failed: {message}");
        overlay::set(&self.app, PillState::Message { text: message.to_owned(), tone: "error" });
        overlay::sound(&self.app, "error");
        self.hide_later(3000);
    }

    /// Hide the pill after `ms`, unless a new recording has started since.
    fn hide_later(self: &Arc<Self>, ms: u64) {
        let me = self.clone();
        let gen = self.generation.load(Ordering::SeqCst);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(ms));
            if me.generation.load(Ordering::SeqCst) == gen && !me.recorder.is_recording() {
                overlay::set(&me.app, PillState::Hidden);
            }
        });
    }
}

fn tail(s: &str, max_chars: usize) -> String {
    let n = s.chars().count();
    s.chars().skip(n.saturating_sub(max_chars)).collect()
}
