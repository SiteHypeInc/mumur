//! Menu-bar / system-tray menu, including the "Recent" list: one click
//! copies any of your last dictations, so an app that swallowed a paste
//! never costs you what you said.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Wry};

use crate::controller::Controller;
use crate::overlay::{self, PillState};

const TRAY_ID: &str = "murmur";
const RECENT: usize = 15;

fn label_for(text: &str) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() > 48 {
        format!("{}…", one_line.chars().take(47).collect::<String>())
    } else {
        one_line
    }
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let ctl = app.state::<Arc<Controller>>();
    let (paste_key, paused) = {
        let s = ctl.settings.read().unwrap();
        (s.hotkey.paste_last.clone(), ctl.paused.load(Ordering::SeqCst))
    };
    let recent = ctl.recent(RECENT);

    let toggle = MenuItem::with_id(app, "toggle", "Start / stop dictation", true, None::<&str>)?;
    let paste_label = if paste_key.is_empty() { "Paste last dictation".to_owned() } else { format!("Paste last dictation  ({paste_key})") };
    let paste_last = MenuItem::with_id(app, "paste_last", paste_label, !recent.is_empty(), None::<&str>)?;

    let recent_items: Vec<MenuItem<Wry>> = recent
        .iter()
        .map(|(id, text)| MenuItem::with_id(app, format!("recent:{id}"), label_for(text), true, None::<&str>))
        .collect::<tauri::Result<_>>()?;
    let empty = MenuItem::with_id(app, "recent:none", "Nothing yet", false, None::<&str>)?;
    let hint = MenuItem::with_id(app, "recent:hint", "Click one to copy it", false, None::<&str>)?;
    let mut refs: Vec<&dyn IsMenuItem<Wry>> = Vec::new();
    if recent_items.is_empty() {
        refs.push(&empty);
    } else {
        refs.push(&hint);
        refs.extend(recent_items.iter().map(|i| i as &dyn IsMenuItem<Wry>));
    }
    let recent_menu = Submenu::with_items(app, "Recent", true, &refs)?;

    let history = MenuItem::with_id(app, "history", "History…", true, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", "Settings…", true, None::<&str>)?;
    let pause = CheckMenuItem::with_id(app, "pause", "Pause hotkey", true, paused, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Murmur", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    Menu::with_items(app, &[&toggle, &paste_last, &recent_menu, &sep1, &history, &open, &sep2, &pause, &quit])
}

fn on_menu(app: &AppHandle, id: &str) {
    let ctl = app.state::<Arc<Controller>>().inner().clone();
    match id {
        "open" => crate::open_settings(app, "general"),
        "history" => crate::open_settings(app, "history"),
        "toggle" => ctl.toggle(),
        "paste_last" => ctl.paste_last(),
        "pause" => {
            let now = !ctl.paused.load(Ordering::SeqCst);
            ctl.paused.store(now, Ordering::SeqCst);
            refresh(app);
        }
        "quit" => app.exit(0),
        other => {
            if let Some(entry) = other.strip_prefix("recent:").and_then(|n| n.parse::<i64>().ok()) {
                if let Some(text) = ctl.recent(RECENT).into_iter().find(|(i, _)| *i == entry).map(|(_, t)| t) {
                    match murmur_sys::paste::copy_to_clipboard(&text) {
                        Ok(()) => overlay::set(app, PillState::Message { text: "Copied. Paste with ⌘V / Ctrl+V".into(), tone: "ok" }),
                        Err(e) => overlay::set(app, PillState::Message { text: e.to_string(), tone: "error" }),
                    }
                    ctl.hide_overlay_later(1600);
                }
            }
        }
    }
}

pub fn install(app: &AppHandle) -> tauri::Result<()> {
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))?;
    TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Murmur")
        .menu(&build_menu(app)?)
        .on_menu_event(|app, ev| on_menu(app, ev.id().as_ref()))
        // Monochrome template icon so macOS tints it for light/dark menu bars.
        .icon(icon)
        .icon_as_template(true)
        .build(app)?;
    Ok(())
}

/// Rebuild after each dictation so "Recent" is current.
pub fn refresh(app: &AppHandle) {
    let app2 = app.clone();
    // Menus must be built on the main thread on macOS.
    let _ = app.run_on_main_thread(move || {
        if let (Some(tray), Ok(menu)) = (app2.tray_by_id(TRAY_ID), build_menu(&app2)) {
            let _ = tray.set_menu(Some(menu));
        }
    });
}
