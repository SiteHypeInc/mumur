//! The floating pill. It must never take focus: the text goes to whatever
//! app the user was in, so showing the pill can't activate us.

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

pub const LABEL: &str = "overlay";
const WIDTH: f64 = 236.0;
const HEIGHT: f64 = 64.0;
const BOTTOM_MARGIN: f64 = 72.0;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PillState {
    Recording { mode: Option<String> },
    Processing,
    Done { words: usize, ms: u64 },
    Message { text: String, tone: &'static str },
    Hidden,
}

pub fn create(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let w = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("overlay.html".into()))
        .title("Murmur")
        .inner_size(WIDTH, HEIGHT)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .skip_taskbar(true)
        .focused(false)
        .focusable(false)
        .visible(false)
        .build()?;
    Ok(w)
}

fn window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(LABEL)
}

/// Bottom-center of the monitor the cursor is on.
fn place(app: &AppHandle, w: &WebviewWindow) {
    let cursor = app.cursor_position().ok();
    let monitors = w.available_monitors().unwrap_or_default();
    let monitor = cursor
        .and_then(|c| {
            monitors.iter().find(|m| {
                let (p, s) = (m.position(), m.size());
                c.x >= p.x as f64 && c.x < (p.x + s.width as i32) as f64 && c.y >= p.y as f64 && c.y < (p.y + s.height as i32) as f64
            })
        })
        .cloned()
        .or_else(|| w.primary_monitor().ok().flatten());
    if let Some(m) = monitor {
        let scale = m.scale_factor();
        let (p, s) = (m.position(), m.size());
        let x = p.x as f64 + (s.width as f64 - WIDTH * scale) / 2.0;
        let y = p.y as f64 + s.height as f64 - (HEIGHT + BOTTOM_MARGIN) * scale;
        let _ = w.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
    }
}

#[cfg(target_os = "macos")]
fn show_without_focus(w: &WebviewWindow) {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    let w2 = w.clone();
    // AppKit calls must happen on the main thread; the pointer is fetched there too.
    let _ = w.run_on_main_thread(move || unsafe {
        let Ok(ptr) = w2.ns_window() else { return };
        let obj = &*(ptr as *mut AnyObject);
        // Float above full-screen apps too (NSStatusWindowLevel).
        let _: () = msg_send![obj, setLevel: 25isize];
        // Bring to front without activating Murmur, so focus stays in the user's app.
        let _: () = msg_send![obj, orderFrontRegardless];
    });
}

#[cfg(not(target_os = "macos"))]
fn show_without_focus(w: &WebviewWindow) {
    // focusable(false) gives the window WS_EX_NOACTIVATE on Windows.
    let _ = w.show();
}

pub fn set(app: &AppHandle, state: PillState) {
    let Some(w) = window(app) else { return };
    let hidden = matches!(state, PillState::Hidden);
    let _ = w.emit_to(LABEL, "pill", &state);
    if hidden {
        let _ = w.hide();
    } else if !w.is_visible().unwrap_or(false) {
        place(app, &w);
        show_without_focus(&w);
        // Click-through. Applied after showing: GTK can't shape an unrealized window.
        let _ = w.set_ignore_cursor_events(true);
    }
}

pub fn level(app: &AppHandle, level: f32) {
    let _ = app.emit_to(LABEL, "level", level);
}

pub fn sound(app: &AppHandle, kind: &'static str) {
    let _ = app.emit_to(LABEL, "sound", kind);
}
