// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod controller;
mod local;
mod overlay;
mod secrets;
mod tray;


use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use controller::Controller;

pub fn open_settings(app: &AppHandle, section: &str) {
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
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

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
            commands::download_model,
            commands::delete_model,
        ])
        .setup(|app| {
            // Menu-bar app: no Dock icon, no app switcher entry.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let handle = app.handle().clone();
            let dir = app.path().app_config_dir()?;
            let data = app.path().app_data_dir()?;
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
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing settings hides it; the app keeps running in the menu bar.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "settings" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Murmur");
}
