//! Which app is the dictation going into, and what's in its focused field.

use murmur_core::context::AppContext;

#[derive(Debug, Clone, Default)]
pub struct Target {
    pub context: AppContext,
    pub pid: Option<i32>,
}

pub fn target() -> Target {
    imp::target()
}

/// (field value, selected text) of the focused text field, where the OS lets
/// us read it. Password fields are never read.
pub fn focused_text() -> (Option<String>, Option<String>) {
    imp::focused_text()
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;

    pub fn target() -> Target {
        let (context, pid) = crate::macos::frontmost();
        Target { context, pid }
    }

    pub fn focused_text() -> (Option<String>, Option<String>) {
        crate::macos::focused_text()
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use super::*;
    use windows_sys::Win32::Foundation::{CloseHandle, HWND};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId};

    pub fn target() -> Target {
        unsafe {
            let hwnd: HWND = GetForegroundWindow();
            if hwnd.is_null() {
                return Target::default();
            }
            let mut title = [0u16; 512];
            let n = GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32);
            let window_title = (n > 0).then(|| String::from_utf16_lossy(&title[..n as usize]));

            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            let mut exe = None;
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if !h.is_null() {
                let mut buf = [0u16; 1024];
                let mut len = buf.len() as u32;
                if QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len) != 0 {
                    let path = String::from_utf16_lossy(&buf[..len as usize]);
                    exe = path.rsplit(['\\', '/']).next().map(str::to_owned);
                }
                CloseHandle(h);
            }
            let app_name = exe.as_deref().map(|e| e.trim_end_matches(".exe").trim_end_matches(".EXE").to_owned());
            Target {
                context: AppContext { app_name, app_id: exe, window_title, url_host: None },
                pid: Some(pid as i32),
            }
        }
    }

    pub fn focused_text() -> (Option<String>, Option<String>) {
        // UI Automation support is planned; without it we don't read fields on Windows.
        (None, None)
    }
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
mod imp {
    use super::*;
    use std::process::Command;

    fn xdotool(args: &[&str]) -> Option<String> {
        let out = Command::new("xdotool").args(args).output().ok()?;
        let s = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        (out.status.success() && !s.is_empty()).then_some(s)
    }

    pub fn target() -> Target {
        let window_title = xdotool(&["getactivewindow", "getwindowname"]);
        let class = xdotool(&["getactivewindow", "getwindowclassname"]);
        let pid = xdotool(&["getactivewindow", "getwindowpid"]).and_then(|p| p.parse().ok());
        Target { context: AppContext { app_name: class.clone(), app_id: class, window_title, url_host: None }, pid }
    }

    pub fn focused_text() -> (Option<String>, Option<String>) {
        (None, None)
    }
}
