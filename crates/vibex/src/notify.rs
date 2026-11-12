//! Desktop notifications for OSC 9 / OSC 777 while Vibex is in the
//! background: a notification-area balloon (shown by Windows as a toast)
//! plus a taskbar flash. The tray icon exists only while a window has shown
//! a notification and is removed when that window closes.

use winit::window::Window;

#[cfg(windows)]
fn hwnd(window: &Window) -> Option<*mut std::ffi::c_void> {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(w) => Some(w.hwnd.get() as *mut std::ffi::c_void),
        _ => None,
    }
}

#[cfg(windows)]
mod imp {
    use std::collections::HashSet;
    use std::sync::Mutex;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Shell::{Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW};
    use windows_sys::Win32::UI::WindowsAndMessaging::{FlashWindowEx, LoadIconW, FLASHWINFO, FLASHW_TIMERNOFG, FLASHW_TRAY, IDI_APPLICATION};

    /// Windows that currently own a tray icon (by HWND value).
    static ADDED: Mutex<Option<HashSet<isize>>> = Mutex::new(None);

    fn copy(dst: &mut [u16], s: &str) {
        let n = dst.len() - 1;
        for (i, u) in s.encode_utf16().take(n).enumerate() {
            dst[i] = u;
        }
    }

    fn data(hwnd: *mut std::ffi::c_void) -> NOTIFYICONDATAW {
        let mut d: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        d.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        d.hWnd = hwnd;
        d.uID = 1;
        d
    }

    pub fn show(hwnd: *mut std::ffi::c_void, title: &str, body: &str) {
        let mut d = data(hwnd);
        d.uFlags = NIF_ICON | NIF_TIP | NIF_INFO;
        unsafe {
            // resource 1 is the app icon embedded by build.rs
            let mut icon = LoadIconW(GetModuleHandleW(std::ptr::null()), 1 as _);
            if icon.is_null() {
                icon = LoadIconW(std::ptr::null_mut(), IDI_APPLICATION);
            }
            d.hIcon = icon;
        }
        copy(&mut d.szTip, "Vibex");
        copy(&mut d.szInfoTitle, title);
        copy(&mut d.szInfo, if body.is_empty() { " " } else { body });
        d.dwInfoFlags = NIIF_INFO;
        let mut set = ADDED.lock().unwrap();
        let set = set.get_or_insert_with(HashSet::new);
        let key = hwnd as isize;
        unsafe {
            if set.contains(&key) {
                Shell_NotifyIconW(NIM_MODIFY, &d);
            } else if Shell_NotifyIconW(NIM_ADD, &d) != 0 {
                set.insert(key);
            }
            let fi = FLASHWINFO { cbSize: std::mem::size_of::<FLASHWINFO>() as u32, hwnd, dwFlags: FLASHW_TRAY | FLASHW_TIMERNOFG, uCount: 0, dwTimeout: 0 };
            FlashWindowEx(&fi);
        }
    }

    pub fn remove(hwnd: *mut std::ffi::c_void) {
        let mut set = ADDED.lock().unwrap();
        if let Some(set) = set.as_mut() {
            if set.remove(&(hwnd as isize)) {
                let d = data(hwnd);
                unsafe {
                    Shell_NotifyIconW(NIM_DELETE, &d);
                }
            }
        }
    }
}

pub fn show(window: &Window, title: &str, body: &str) {
    #[cfg(windows)]
    if let Some(h) = hwnd(window) {
        imp::show(h, title, body);
    }
    #[cfg(not(windows))]
    {
        let _ = (window, title, body);
        // freedesktop: hand off to notify-send when present
        let _ = std::process::Command::new("notify-send").arg(title).arg(body).spawn();
    }
}

pub fn remove(window: &Window) {
    #[cfg(windows)]
    if let Some(h) = hwnd(window) {
        imp::remove(h);
    }
    #[cfg(not(windows))]
    let _ = window;
}
