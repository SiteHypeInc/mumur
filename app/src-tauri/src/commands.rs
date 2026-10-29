//! Commands the settings window calls.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use murmur_core::config::{PolishProvider, Settings, SttProvider};
use murmur_core::history::Entry;
use murmur_core::providers::check;
use serde::Serialize;
use tauri::State;

use crate::controller::Controller;
use crate::secrets;

type Ctl<'a> = State<'a, Arc<Controller>>;

#[derive(Serialize)]
pub struct Overview {
    settings: Settings,
    keys: std::collections::HashMap<&'static str, bool>,
    accessibility: bool,
    input_monitoring: bool,
    hotkey_error: Option<String>,
    hotkey_status: murmur_sys::hotkey::HotkeyStatus,
    default_input: Option<String>,
    devices: Vec<String>,
    platform: &'static str,
    version: &'static str,
    total_words: i64,
    paused: bool,
}

#[tauri::command]
pub fn overview(ctl: Ctl) -> Overview {
    let perms = murmur_sys::permissions::status(false);
    let total_words = ctl.history.lock().unwrap().as_ref().and_then(|h| h.total_words().ok()).unwrap_or(0);
    Overview {
        settings: ctl.settings.read().unwrap().clone(),
        keys: secrets::PROVIDERS.iter().map(|p| (*p, secrets::get(p).is_some())).collect(),
        accessibility: perms.accessibility,
        input_monitoring: perms.input_monitoring,
        hotkey_error: ctl.hotkey_error.lock().unwrap().clone(),
        hotkey_status: ctl.hotkey_status(),
        default_input: murmur_sys::capture::default_input_name(),
        devices: murmur_sys::capture::input_devices(),
        platform: std::env::consts::OS,
        version: env!("CARGO_PKG_VERSION"),
        total_words,
        paused: ctl.paused.load(Ordering::SeqCst),
    }
}

#[tauri::command]
pub fn save_settings(ctl: Ctl, settings: Settings) -> Result<(), String> {
    ctl.apply_settings(settings)
}

#[tauri::command]
pub fn set_api_key(provider: String, key: String) -> Result<(), String> {
    secrets::set(&provider, &key)
}

/// Verify the keys the current settings will actually use.
#[tauri::command]
pub async fn test_connection(ctl: Ctl<'_>) -> Result<String, String> {
    let s = ctl.settings.read().unwrap().clone();
    let mut report = Vec::new();

    let sp = s.stt.provider;
    if sp == SttProvider::Local {
        let id = crate::local::LocalEngine::model_id(&s.stt.model);
        if !ctl.local.is_ready(id) {
            return Err("Download a speech model first (Settings → Engines)".into());
        }
        report.push("Transcription: on-device ✓".to_owned());
    } else {
        let stt_key = secrets::get(secrets::stt_key_name(sp));
        if sp != SttProvider::Custom && stt_key.is_none() {
            return Err(format!("Add your {} API key for transcription", sp.label()));
        }
        let base = if s.stt.base_url.trim().is_empty() { sp.default_base_url().to_owned() } else { s.stt.base_url.clone() };
        check::openai_compatible(sp.label(), &base, stt_key.as_deref()).await.map_err(|e| e.user_message())?;
        report.push(format!("Transcription: {} ✓", sp.label()));
    }

    if s.polish.enabled {
        let pp = s.polish.provider;
        let key = secrets::get(secrets::polish_key_name(pp));
        let base = if s.polish.base_url.trim().is_empty() { pp.default_base_url().to_owned() } else { s.polish.base_url.clone() };
        match (pp, key) {
            (PolishProvider::Anthropic, Some(k)) => check::anthropic(&base, &k).await,
            (PolishProvider::Custom | PolishProvider::Ollama, k) => check::openai_compatible(pp.label(), &base, k.as_deref()).await,
            (_, Some(k)) => check::openai_compatible(pp.label(), &base, Some(&k)).await,
            (_, None) => {
                report.push(format!("Cleanup: built-in tidy-up (add a {} key or use Ollama for AI cleanup)", pp.label()));
                return Ok(report.join(" · "));
            }
        }
        .map_err(|e| match pp {
            PolishProvider::Ollama => "Ollama isn't running. Install it from ollama.com, then run: ollama pull llama3.2".to_owned(),
            _ => e.user_message(),
        })?;
        report.push(format!("Cleanup: {} ✓", pp.label()));
    }
    Ok(report.join(" · "))
}

#[tauri::command]
pub fn history(ctl: Ctl, query: Option<String>, limit: Option<usize>) -> Vec<Entry> {
    let q = query.filter(|q| !q.trim().is_empty());
    ctl.history
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|h| h.list(q.as_deref(), limit.unwrap_or(200)).ok())
        .unwrap_or_default()
}

#[tauri::command]
pub fn delete_history(ctl: Ctl, id: Option<i64>) -> Result<(), String> {
    let guard = ctl.history.lock().unwrap();
    let Some(h) = guard.as_ref() else { return Ok(()) };
    match id {
        Some(id) => h.delete(id),
        None => h.clear(),
    }
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn copy_text(text: String) -> Result<(), String> {
    murmur_sys::paste::copy_to_clipboard(&text).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn request_permissions() -> (bool, bool) {
    let s = murmur_sys::permissions::status(true);
    (s.accessibility, s.input_monitoring)
}

#[tauri::command]
pub fn set_paused(ctl: Ctl, paused: bool) {
    ctl.paused.store(paused, Ordering::SeqCst);
}

#[tauri::command]
pub fn models(ctl: Ctl) -> Vec<crate::local::ModelStatus> {
    ctl.local.list()
}

#[tauri::command]
pub async fn download_model(app: tauri::AppHandle, ctl: Ctl<'_>, id: String) -> Result<(), String> {
    ctl.local.download(&app, &id).await?;
    ctl.prewarm();
    Ok(())
}

#[tauri::command]
pub fn delete_model(ctl: Ctl, id: String) -> Result<(), String> {
    ctl.local.delete(&id)
}

#[tauri::command]
pub fn toggle_recording(ctl: Ctl) {
    ctl.inner().toggle();
}

#[derive(Serialize)]
pub struct MicTest {
    /// 0..1 meter peak.
    level: f32,
    /// Largest raw sample; 0 means the OS is handing us silence (blocked mic).
    max_sample: f32,
    device: Option<String>,
}

#[tauri::command]
pub async fn test_microphone(ctl: Ctl<'_>) -> Result<MicTest, String> {
    let c = ctl.inner().clone();
    let device = {
        let d = c.settings.read().unwrap().audio.device.clone();
        if d.is_empty() { murmur_sys::capture::default_input_name() } else { Some(d) }
    };
    let (level, max_sample) = tauri::async_runtime::spawn_blocking(move || c.test_microphone())
        .await
        .map_err(|e| e.to_string())??;
    Ok(MicTest { level, max_sample, device })
}

#[tauri::command]
pub fn hotkey_status(ctl: Ctl) -> murmur_sys::hotkey::HotkeyStatus {
    ctl.hotkey_status()
}

/// Open a System Settings privacy pane: "accessibility", "input_monitoring", "microphone".
#[tauri::command]
pub fn open_privacy(pane: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let anchor = match pane.as_str() {
            "accessibility" => "Privacy_Accessibility",
            "input_monitoring" => "Privacy_ListenEvent",
            "microphone" => "Privacy_Microphone",
            _ => return Err("unknown pane".into()),
        };
        std::process::Command::new("open")
            .arg(format!("x-apple.systempreferences:com.apple.preference.security?{anchor}"))
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "windows")]
    if pane == "microphone" {
        std::process::Command::new("cmd").args(["/C", "start", "ms-settings:privacy-microphone"]).spawn().map_err(|e| e.to_string())?;
    }
    let _ = pane;
    Ok(())
}

/// macOS: forget Murmur's permission grants (they go stale when an unsigned
/// build is replaced), then relaunch so it asks fresh.
#[tauri::command]
pub fn reset_permissions(app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let id = app.config().identifier.clone();
        for service in ["Accessibility", "ListenEvent", "Microphone"] {
            let _ = std::process::Command::new("/usr/bin/tccutil").args(["reset", service, &id]).status();
        }
    }
    app.restart();
}

#[tauri::command]
pub fn relaunch(app: tauri::AppHandle) {
    app.restart();
}

/// Plain-text status report for support ("Copy diagnostics").
#[tauri::command]
pub fn diagnostics(ctl: Ctl) -> String {
    let p = murmur_sys::permissions::status(false);
    let s = ctl.settings.read().unwrap().clone();
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
    [
        format!("Murmur {} on {} {}", env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH),
        format!("app: {exe}"),
        format!("hotkey: {} ({:?}), status {:?}, paused {}", s.hotkey.binding, s.hotkey.mode, ctl.hotkey_status(), ctl.paused.load(Ordering::SeqCst)),
        format!("accessibility: {}, input monitoring: {}", p.accessibility, p.input_monitoring),
        format!(
            "mic: {} (default: {})",
            if s.audio.device.is_empty() { "system default" } else { &s.audio.device },
            murmur_sys::capture::default_input_name().unwrap_or_else(|| "none".into())
        ),
        format!("engine: {:?} {} · cleanup: {} {:?}", s.stt.provider, crate::local::LocalEngine::model_id(&s.stt.model), s.polish.enabled, s.polish.provider),
        format!("model ready: {}", ctl.local.is_ready(crate::local::LocalEngine::model_id(&s.stt.model))),
    ]
    .join("\n")
}
