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
    let stt_key = secrets::get(secrets::stt_key_name(sp));
    if sp != SttProvider::Custom && stt_key.is_none() {
        return Err(format!("Add your {} API key for transcription", sp.label()));
    }
    let base = if s.stt.base_url.trim().is_empty() { sp.default_base_url().to_owned() } else { s.stt.base_url.clone() };
    check::openai_compatible(sp.label(), &base, stt_key.as_deref()).await.map_err(|e| e.user_message())?;
    report.push(format!("Transcription: {} ✓", sp.label()));

    if s.polish.enabled {
        let pp = s.polish.provider;
        let key = secrets::get(secrets::polish_key_name(pp));
        let base = if s.polish.base_url.trim().is_empty() { pp.default_base_url().to_owned() } else { s.polish.base_url.clone() };
        match (pp, key) {
            (PolishProvider::Anthropic, Some(k)) => check::anthropic(&base, &k).await,
            (PolishProvider::Custom, k) => check::openai_compatible(pp.label(), &base, k.as_deref()).await,
            (_, Some(k)) => check::openai_compatible(pp.label(), &base, Some(&k)).await,
            (_, None) => return Err(format!("Add your {} API key for cleanup, or turn cleanup off", pp.label())),
        }
        .map_err(|e| e.user_message())?;
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
pub fn toggle_recording(ctl: Ctl) {
    ctl.inner().toggle();
}
