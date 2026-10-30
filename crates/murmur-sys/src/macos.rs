//! macOS: event tap for the hotkey (incl. Fn/Globe), Accessibility reads of
//! the focused app and field, and synthetic ⌘V.

use std::ffi::c_void;
use std::process::Command;
use std::sync::atomic::{AtomicPtr, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
use core_foundation::string::{CFString, CFStringRef};
use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventType, CallbackResult, EventField,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use murmur_core::context::AppContext;
use objc2_app_kit::NSWorkspace;

use crate::hotkey::{Dispatch, HotkeyError, HotkeyStatus};
use crate::keys::Key;

// ---- Accessibility FFI -------------------------------------------------------

type AXUIElementRef = *const c_void;
type AXError = i32;
const AX_SUCCESS: AXError = 0;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(element: AXUIElementRef, attribute: CFStringRef, value: *mut CFTypeRef) -> AXError;
    fn AXUIElementSetMessagingTimeout(element: AXUIElementRef, seconds: f32) -> AXError;
    fn AXUIElementSetAttributeValue(element: AXUIElementRef, attribute: CFStringRef, value: CFTypeRef) -> AXError;
    fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
    fn CFGetTypeID(cf: CFTypeRef) -> usize;
    fn CFStringGetTypeID() -> usize;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightListenEventAccess() -> bool;
    fn CGRequestListenEventAccess() -> bool;
    fn CGEventTapEnable(tap: *mut c_void, enable: bool);
}

/// Owned AX/CF reference released on drop.
struct Owned(CFTypeRef);
impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) }
        }
    }
}

fn copy_attr(element: AXUIElementRef, name: &str) -> Option<Owned> {
    if element.is_null() {
        return None;
    }
    let attr = CFString::new(name);
    let mut value: CFTypeRef = std::ptr::null();
    let err = unsafe { AXUIElementCopyAttributeValue(element, attr.as_concrete_TypeRef(), &mut value) };
    (err == AX_SUCCESS && !value.is_null()).then_some(Owned(value))
}

fn as_string(v: &Owned) -> Option<String> {
    unsafe {
        if CFGetTypeID(v.0) != CFStringGetTypeID() {
            return None;
        }
        Some(CFString::wrap_under_get_rule(v.0 as CFStringRef).to_string())
    }
}

fn string_attr(element: AXUIElementRef, name: &str) -> Option<String> {
    copy_attr(element, name).as_ref().and_then(as_string)
}

/// Is this app allowed to use Accessibility (needed to paste and read fields)?
pub fn accessibility_trusted(prompt: bool) -> bool {
    let key = CFString::from_static_string("AXTrustedCheckOptionPrompt");
    let opts = CFDictionary::from_CFType_pairs(&[(key.as_CFType(), CFBoolean::from(prompt).as_CFType())]);
    unsafe { AXIsProcessTrustedWithOptions(opts.as_concrete_TypeRef() as *const c_void) }
}

/// Input Monitoring, needed for the hotkey event tap.
pub fn input_monitoring_granted(prompt: bool) -> bool {
    unsafe {
        if CGPreflightListenEventAccess() {
            return true;
        }
        if prompt {
            return CGRequestListenEventAccess();
        }
        false
    }
}

// ---- Frontmost app -----------------------------------------------------------

const BROWSER_SCRIPTS: &[(&str, &str)] = &[
    ("com.google.Chrome", "tell application id \"com.google.Chrome\" to get URL of active tab of front window"),
    ("com.brave.Browser", "tell application id \"com.brave.Browser\" to get URL of active tab of front window"),
    ("com.microsoft.edgemac", "tell application id \"com.microsoft.edgemac\" to get URL of active tab of front window"),
    ("company.thebrowser.Browser", "tell application id \"company.thebrowser.Browser\" to get URL of active tab of front window"),
    ("com.apple.Safari", "tell application id \"com.apple.Safari\" to get URL of front document"),
];

pub fn frontmost() -> (AppContext, Option<i32>) {
    let ws = NSWorkspace::sharedWorkspace();
    let Some(app) = ws.frontmostApplication() else {
        return (AppContext::default(), None);
    };
    let name = app.localizedName().map(|s| s.to_string());
    let bundle = app.bundleIdentifier().map(|s| s.to_string());
    let pid = app.processIdentifier();

    let element = unsafe { AXUIElementCreateApplication(pid) };
    let _guard = Owned(element);
    unsafe { AXUIElementSetMessagingTimeout(element, 0.25) };
    let title = copy_attr(element, "AXFocusedWindow").and_then(|w| string_attr(w.0, "AXTitle"));

    let url_host = bundle
        .as_deref()
        .and_then(|b| BROWSER_SCRIPTS.iter().find(|(id, _)| *id == b))
        .and_then(|(_, script)| run_osascript(script, Duration::from_millis(400)))
        .and_then(|url| host_of(&url));

    (AppContext { app_name: name, app_id: bundle, window_title: title, url_host }, Some(pid))
}

fn host_of(url: &str) -> Option<String> {
    let rest = url.split("://").nth(1)?;
    let host = rest.split(['/', '?', '#']).next()?.split('@').next_back()?.split(':').next()?;
    (!host.is_empty()).then(|| host.to_lowercase())
}

fn run_osascript(script: &str, timeout: Duration) -> Option<String> {
    let mut child = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(script)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                let out = child.wait_with_output().ok()?;
                let s = String::from_utf8_lossy(&out.stdout).trim().to_owned();
                return (!s.is_empty()).then_some(s);
            }
            Ok(Some(_)) => return None,
            Ok(None) if start.elapsed() > timeout => {
                let _ = child.kill();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => return None,
        }
    }
}

/// Text of the focused field and the current selection, via Accessibility.
pub fn focused_text() -> (Option<String>, Option<String>) {
    let system = unsafe { AXUIElementCreateSystemWide() };
    let _guard = Owned(system);
    unsafe { AXUIElementSetMessagingTimeout(system, 0.25) };
    let Some(focused) = copy_attr(system, "AXFocusedUIElement") else {
        return (None, None);
    };
    // Never read password fields.
    if string_attr(focused.0, "AXSubrole").as_deref() == Some("AXSecureTextField") {
        return (None, None);
    }
    (string_attr(focused.0, "AXValue"), string_attr(focused.0, "AXSelectedText"))
}

/// Chrome and Electron apps (Grok, Slack, Cursor…) only build their
/// Accessibility tree once asked. Without it we can't see whether a paste
/// landed in their text fields.
pub fn enable_app_accessibility(pid: i32) {
    let element = unsafe { AXUIElementCreateApplication(pid) };
    let _guard = Owned(element);
    unsafe { AXUIElementSetMessagingTimeout(element, 0.25) };
    let attr = CFString::new("AXManualAccessibility");
    unsafe { AXUIElementSetAttributeValue(element, attr.as_concrete_TypeRef(), CFBoolean::true_value().as_CFTypeRef()) };
}

// ---- Synthetic keystrokes ----------------------------------------------------

const KC_V: u16 = 9;
const KC_RETURN: u16 = 36;
const KC_COMMAND: u16 = 55;

fn post_key(keycode: u16, flags: CGEventFlags) -> Result<(), String> {
    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState).map_err(|_| "event source")?;
    let down = CGEvent::new_keyboard_event(source.clone(), keycode, true).map_err(|_| "key down")?;
    let up = CGEvent::new_keyboard_event(source, keycode, false).map_err(|_| "key up")?;
    down.set_flags(flags);
    up.set_flags(flags);
    down.post(CGEventTapLocation::HID);
    std::thread::sleep(Duration::from_millis(8));
    up.post(CGEventTapLocation::HID);
    Ok(())
}

pub fn send_paste() -> Result<(), String> {
    let _ = KC_COMMAND;
    post_key(KC_V, CGEventFlags::CGEventFlagCommand)
}

pub fn send_enter(shift: bool, cmd: bool) -> Result<(), String> {
    let mut flags = CGEventFlags::CGEventFlagNull;
    if shift {
        flags |= CGEventFlags::CGEventFlagShift;
    }
    if cmd {
        flags |= CGEventFlags::CGEventFlagCommand;
    }
    post_key(KC_RETURN, flags)
}

// ---- Hotkey event tap --------------------------------------------------------

// Device-dependent modifier bits (IOKit NX_DEVICE*KEYMASK); they tell left from right.
const NX_LCTL: u64 = 0x0000_0001;
const NX_LSHIFT: u64 = 0x0000_0002;
const NX_RSHIFT: u64 = 0x0000_0004;
const NX_LCMD: u64 = 0x0000_0008;
const NX_RCMD: u64 = 0x0000_0010;
const NX_LALT: u64 = 0x0000_0020;
const NX_RALT: u64 = 0x0000_0040;
const NX_RCTL: u64 = 0x0000_2000;
const FN_FLAG: u64 = 0x0080_0000;

fn key_from_code(code: u16) -> Key {
    match code {
        63 | 179 => Key::Fn,
        59 => Key::LeftCtrl,
        62 => Key::RightCtrl,
        58 => Key::LeftAlt,
        61 => Key::RightAlt,
        56 => Key::LeftShift,
        60 => Key::RightShift,
        55 => Key::LeftMeta,
        54 => Key::RightMeta,
        49 => Key::Space,
        53 => Key::Escape,
        36 | 76 => Key::Enter,
        48 => Key::Tab,
        51 => Key::Backspace,
        122 => Key::F(1), 120 => Key::F(2), 99 => Key::F(3), 118 => Key::F(4),
        96 => Key::F(5), 97 => Key::F(6), 98 => Key::F(7), 100 => Key::F(8),
        101 => Key::F(9), 109 => Key::F(10), 103 => Key::F(11), 111 => Key::F(12),
        105 => Key::F(13), 107 => Key::F(14), 113 => Key::F(15), 106 => Key::F(16),
        64 => Key::F(17), 79 => Key::F(18), 80 => Key::F(19), 90 => Key::F(20),
        0 => Key::Char('a'), 11 => Key::Char('b'), 8 => Key::Char('c'), 2 => Key::Char('d'),
        14 => Key::Char('e'), 3 => Key::Char('f'), 5 => Key::Char('g'), 4 => Key::Char('h'),
        34 => Key::Char('i'), 38 => Key::Char('j'), 40 => Key::Char('k'), 37 => Key::Char('l'),
        46 => Key::Char('m'), 45 => Key::Char('n'), 31 => Key::Char('o'), 35 => Key::Char('p'),
        12 => Key::Char('q'), 15 => Key::Char('r'), 1 => Key::Char('s'), 17 => Key::Char('t'),
        32 => Key::Char('u'), 9 => Key::Char('v'), 13 => Key::Char('w'), 7 => Key::Char('x'),
        16 => Key::Char('y'), 6 => Key::Char('z'),
        29 => Key::Char('0'), 18 => Key::Char('1'), 19 => Key::Char('2'), 20 => Key::Char('3'),
        21 => Key::Char('4'), 23 => Key::Char('5'), 22 => Key::Char('6'), 26 => Key::Char('7'),
        28 => Key::Char('8'), 25 => Key::Char('9'),
        other => Key::Other(other as u32),
    }
}

/// For a FlagsChanged event, is the modifier that changed now down?
fn modifier_down(key: Key, flags: u64) -> Option<bool> {
    let bit = match key {
        Key::Fn => FN_FLAG,
        Key::LeftCtrl => NX_LCTL,
        Key::RightCtrl => NX_RCTL,
        Key::LeftAlt => NX_LALT,
        Key::RightAlt => NX_RALT,
        Key::LeftShift => NX_LSHIFT,
        Key::RightShift => NX_RSHIFT,
        Key::LeftMeta => NX_LCMD,
        Key::RightMeta => NX_RCMD,
        _ => return None,
    };
    Some(flags & bit != 0)
}

static TAP_PORT: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

/// 0 = waiting for permission, 1 = active tap (can swallow keys), 2 = listen-only.
static TAP_STATUS: AtomicU8 = AtomicU8::new(0);

pub fn listener_status() -> HotkeyStatus {
    match TAP_STATUS.load(Ordering::SeqCst) {
        1 => HotkeyStatus::Active,
        2 => HotkeyStatus::ListenOnly,
        _ => HotkeyStatus::WaitingForPermission,
    }
}

fn make_tap(options: CGEventTapOptions, d: Arc<Mutex<Dispatch>>) -> Result<CGEventTap<'static>, ()> {
    CGEventTap::new(
        CGEventTapLocation::Session,
        CGEventTapPlacement::HeadInsertEventTap,
        options,
        vec![CGEventType::KeyDown, CGEventType::KeyUp, CGEventType::FlagsChanged],
        move |_proxy, etype, event| {
            match etype {
                CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput => {
                    let port = TAP_PORT.load(Ordering::SeqCst);
                    if !port.is_null() {
                        unsafe { CGEventTapEnable(port, true) };
                    }
                    d.lock().unwrap().reset();
                    return CallbackResult::Keep;
                }
                _ => {}
            }
            let code = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE) as u16;
            let key = key_from_code(code);
            let pressed = match etype {
                CGEventType::KeyDown => true,
                CGEventType::KeyUp => false,
                CGEventType::FlagsChanged => match modifier_down(key, event.get_flags().bits()) {
                    Some(down) => down,
                    None => return CallbackResult::Keep,
                },
                _ => return CallbackResult::Keep,
            };
            if d.lock().unwrap().on_key(key, pressed) {
                CallbackResult::Drop
            } else {
                CallbackResult::Keep
            }
        },
    )
}

/// Installs the keyboard tap on its own thread, retrying until macOS allows
/// it. Permission is often granted *after* launch (that's what onboarding
/// asks for), so a one-shot attempt would leave the hotkey dead until a
/// relaunch. An active tap needs Accessibility; with only Input Monitoring we
/// fall back to listen-only (works, but can't swallow combo keys).
pub fn start_listener(dispatch: Arc<Mutex<Dispatch>>) -> Result<(), HotkeyError> {
    // Make sure Murmur is listed in both Privacy panes.
    let _ = input_monitoring_granted(true);
    let _ = accessibility_trusted(true);
    std::thread::Builder::new()
        .name("murmur-hotkey".into())
        .spawn(move || {
            let mut logged = false;
            loop {
                for (options, status) in [(CGEventTapOptions::Default, 1u8), (CGEventTapOptions::ListenOnly, 2u8)] {
                    // A listen-only tap can be created without permission and then
                    // silently receive nothing, so only accept it when it's allowed.
                    if status == 2 && !input_monitoring_granted(false) {
                        continue;
                    }
                    let Ok(tap) = make_tap(options, dispatch.clone()) else { continue };
                    TAP_PORT.store(tap.mach_port().as_concrete_TypeRef() as *mut c_void, Ordering::SeqCst);
                    let Ok(source) = tap.mach_port().create_runloop_source(0) else { continue };
                    unsafe { CFRunLoop::get_current().add_source(&source, kCFRunLoopCommonModes) };
                    tap.enable();
                    TAP_STATUS.store(status, Ordering::SeqCst);
                    log::info!("keyboard tap installed ({})", if status == 1 { "active" } else { "listen-only" });
                    CFRunLoop::run_current();
                    // run_current only returns if the run loop is stopped; try again.
                    TAP_STATUS.store(0, Ordering::SeqCst);
                }
                if !logged {
                    log::warn!("keyboard tap not allowed yet; waiting for Accessibility / Input Monitoring");
                    logged = true;
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        })
        .map_err(|e| HotkeyError::Backend(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts() {
        assert_eq!(host_of("https://mail.google.com/mail/u/0/#inbox").as_deref(), Some("mail.google.com"));
        assert_eq!(host_of("http://user@localhost:3000/x").as_deref(), Some("localhost"));
        assert_eq!(host_of("about:blank"), None);
    }
}
