// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod controller;
mod local;
mod logging;
mod overlay;
mod perms;
mod secrets;
mod tray;


use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use controller::Controller;

/// Dock icon: always when the "Show in Dock" setting is on (the default),
/// otherwise only while the Murmur window is open.
fn show_in_dock(app: &AppHandle, window_open: bool) {
    let always = app.try_state::<std::sync::Arc<Controller>>().is_none_or(|c| c.settings.read().unwrap().show_in_dock);
    let visible = always || window_open;
    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(if visible { tauri::ActivationPolicy::Regular } else { tauri::ActivationPolicy::Accessory });
    let _ = (app, visible);
}

pub fn open_settings(app: &AppHandle, section: &str) {
    show_in_dock(app, true);
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.eval(format!("window.showSection && window.showSection({section:?})"));
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        return;
    }
    let url = WebviewUrl::App(format!("settings.html#{section}").into());
    match WebviewWindowBuilder::new(app, "settings", url)
        .title("Murmur")
        .inner_size(900.0, 640.0)
        .min_inner_size(720.0, 520.0)
        .center()
        .build()
    {
        Ok(w) => {
            let _ = w.set_focus();
        }
        Err(e) => log::error!("could not open settings: {e}"),
    }
}

fn main() {
    logging::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| open_settings(app, "general")))
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .invoke_handler(tauri::generate_handler![
            commands::overview,
            commands::save_settings,
            commands::set_api_key,
            commands::test_connection,
            commands::history,
            commands::delete_history,
            commands::ollama_models,
            commands::ollama_pull,
            commands::open_url,
            commands::copy_text,
            commands::request_permissions,
            commands::set_paused,
            commands::toggle_recording,
            commands::models,
            commands::test_microphone,
            commands::hotkey_status,
            commands::open_privacy,
            commands::reset_permissions,
            commands::relaunch,
            commands::diagnostics,
            commands::download_model,
            commands::delete_model,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let dir = app.path().app_config_dir()?;
            let data = app.path().app_data_dir()?;
            perms::clear_stale_grants(&data, &app.config().identifier);
            let ctl = Controller::new(handle.clone(), dir.join("settings.toml"), data.join("history.sqlite3"), data.join("models"));
            app.manage(ctl.clone());
            overlay::create(&handle)?;
            ctl.start();
            ctl.prewarm();

            tray::install(&handle)?;

            // First run: nothing configured yet, so show onboarding.
            let configured = {
                let s = ctl.settings.read().unwrap();
                if s.stt.provider == murmur_core::config::SttProvider::Local {
                    ctl.local.is_ready(local::LocalEngine::model_id(&s.stt.model))
                } else {
                    secrets::PROVIDERS.iter().any(|p| secrets::get(p).is_some())
                }
            };
            if !configured || ctl.hotkey_error.lock().unwrap().is_some() {
                open_settings(&handle, "setup");
            } else {
                show_in_dock(&handle, false);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing settings hides it; the app keeps running in the menu bar.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "settings" {
                    api.prevent_close();
                    let _ = window.hide();
                    show_in_dock(window.app_handle(), false);
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building Murmur")
        .run(|app, event| {
            // Clicking the Dock icon opens the Murmur window.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                open_settings(app, "general");
            }
            let _ = (app, event);
        });
}
