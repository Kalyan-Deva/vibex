//! A system-wide hotkey (the Quake-style dropdown), registered on a small
//! thread of its own with a Win32 message loop.

/// Parse "WIN|CTRL" + "`" into Win32 modifier flags and a virtual-key code.
pub fn parse(key: &str, mods: &str) -> Result<(u32, u32), String> {
    const MOD_ALT: u32 = 0x1;
    const MOD_CONTROL: u32 = 0x2;
    const MOD_SHIFT: u32 = 0x4;
    const MOD_WIN: u32 = 0x8;
    let mut m = 0;
    for part in mods.split(['|', '+', ' ']).map(str::trim).filter(|p| !p.is_empty()) {
        m |= match part.to_ascii_uppercase().as_str() {
            "WIN" | "SUPER" => MOD_WIN,
            "CTRL" | "CONTROL" => MOD_CONTROL,
            "ALT" => MOD_ALT,
            "SHIFT" => MOD_SHIFT,
            other => return Err(format!("unknown modifier '{other}'")),
        };
    }
    if m == 0 {
        return Err("a global hotkey needs a modifier (WIN, CTRL, ALT)".into());
    }
    let k = key.to_ascii_lowercase();
    let vk = match k.as_str() {
        "`" | "~" | "backquote" | "grave" => 0xC0, // VK_OEM_3
        "space" => 0x20,
        "enter" => 0x0D,
        "tab" => 0x09,
        "escape" | "esc" => 0x1B,
        f if f.starts_with('f') && f.len() > 1 && f[1..].parse::<u32>().is_ok_and(|n| (1..=24).contains(&n)) => 0x70 + f[1..].parse::<u32>().unwrap() - 1,
        c if c.len() == 1 && c.chars().all(|c| c.is_ascii_alphanumeric()) => c.to_ascii_uppercase().as_bytes()[0] as u32,
        other => return Err(format!("unsupported hotkey '{other}'")),
    };
    Ok((m, vk))
}

/// Register the hotkey; `on_press` runs (on the hotkey thread) each time
/// it's pressed. Err if another program already owns that combination.
#[cfg(windows)]
pub fn register(mods: u32, vk: u32, on_press: impl Fn() + Send + 'static) -> Result<(), String> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, MOD_NOREPEAT};
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG, WM_HOTKEY};
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("vibex-hotkey".into())
        .spawn(move || unsafe {
            // registered on this thread: WM_HOTKEY arrives in its queue
            if RegisterHotKey(std::ptr::null_mut(), 1, mods | MOD_NOREPEAT, vk) == 0 {
                let _ = tx.send(Err(std::io::Error::last_os_error().to_string()));
                return;
            }
            let _ = tx.send(Ok(()));
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                if msg.message == WM_HOTKEY {
                    on_press();
                }
            }
        })
        .map_err(|e| e.to_string())?;
    rx.recv().map_err(|e| e.to_string())?
}

#[cfg(not(windows))]
pub fn register(_mods: u32, _vk: u32, _on_press: impl Fn() + Send + 'static) -> Result<(), String> {
    Err("global hotkeys are Windows-only for now".into())
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[test]
    fn registers_an_unused_combination() {
        // Ctrl+Alt+Shift+F23: nothing uses it
        let (m, vk) = super::parse("F23", "CTRL|ALT|SHIFT").unwrap();
        assert!(super::register(m, vk, || {}).is_ok());
    }

    #[test]
    fn parse_keys() {
        assert_eq!(super::parse("`", "WIN").unwrap(), (0x8, 0xC0));
        assert_eq!(super::parse("t", "CTRL|ALT").unwrap(), (0x3, b'T' as u32));
        assert_eq!(super::parse("F12", "WIN|SHIFT").unwrap(), (0xC, 0x7B));
        assert!(super::parse("`", "").is_err());
        assert!(super::parse("??", "WIN").is_err());
    }
}
