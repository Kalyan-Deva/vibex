//! Vibex Terminal setup: a single-file, per-user installer (no admin).
//!
//!   vibex-setup.exe               install (asks first)
//!   vibex-setup.exe --quiet       install without dialogs
//!   vibex-setup.exe --uninstall   remove everything it added
//!
//! Installs to %LOCALAPPDATA%\Programs\Vibex and registers: Start menu
//! shortcut, "Open in Vibex" on folders/drives/folder backgrounds, "Run in
//! Vibex" on .ps1/.bat/.cmd, `vibex` on PATH and in Win+R (App Paths), and an
//! Apps & features entry.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};

const PAYLOAD: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/payload.bin"));
/// OpenConsole.exe from microsoft/terminal (MIT), when packaged with it.
const OPENCONSOLE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/openconsole.bin"));
/// vibex.com: typing `vibex` in a shell runs it, so CLI output waits.
const LAUNCHER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/launcher.bin"));
/// nano and Neovim (a zip of vendor\tools, see tools\fetch-bundle.ps1).
const TOOLS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tools.bin"));
const OPENCONSOLE_LICENSE: &str = "OpenConsole.exe is part of Windows Terminal (https://github.com/microsoft/terminal).

MIT License

Copyright (c) Microsoft Corporation. All rights reserved.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the \"Software\"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
";
const HAS_PAYLOAD: &str = env!("VIBEX_HAS_PAYLOAD");
const SHELL_PS1: &[u8] = include_bytes!("../../../shell/vibex.ps1");
const SHELL_BASH: &[u8] = include_bytes!("../../../shell/vibex.bash");
const SHELL_COMMANDS: &[u8] = include_bytes!("../../../shell/vibex-commands.ps1");
const VERSION: &str = env!("CARGO_PKG_VERSION");

const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Vibex";
const APP_PATHS_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\App Paths\vibex.exe";
/// Explorer verbs we own: (registry key, menu text, command template).
/// `{exe}` is replaced with the installed vibex.exe.
const VERBS: &[(&str, &str, &str)] = &[
    (r"Software\Classes\Directory\shell\Vibex", "Open in Vibex", r#""{exe}" --cwd "%V""#),
    (r"Software\Classes\Directory\Background\shell\Vibex", "Open in Vibex", r#""{exe}" --cwd "%V""#),
    (r"Software\Classes\Drive\shell\Vibex", "Open in Vibex", r#""{exe}" --cwd "%V""#),
    (r"Software\Classes\SystemFileAssociations\.ps1\shell\RunInVibex", "Run in Vibex", r#""{exe}" --cwd "%W" -- powershell.exe -NoLogo -File "%1""#),
    (r"Software\Classes\SystemFileAssociations\.bat\shell\RunInVibex", "Run in Vibex", r#""{exe}" --cwd "%W" -- cmd.exe /c "%1""#),
    (r"Software\Classes\SystemFileAssociations\.cmd\shell\RunInVibex", "Run in Vibex", r#""{exe}" --cwd "%W" -- cmd.exe /c "%1""#),
];

fn install_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Users\Public"));
    base.join("Programs").join("Vibex")
}

fn start_menu_lnk() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(Path::new(&appdata).join(r"Microsoft\Windows\Start Menu\Programs\Vibex.lnk"))
}

// ------------------------------------------------------------------ UI

#[cfg(windows)]
mod ui {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, IDYES, MB_ICONERROR, MB_ICONINFORMATION, MB_ICONQUESTION, MB_YESNO};

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    pub fn ask(text: &str) -> bool {
        let (t, c) = (w(text), w("Vibex Setup"));
        unsafe { MessageBoxW(std::ptr::null_mut(), t.as_ptr(), c.as_ptr(), MB_YESNO | MB_ICONQUESTION) == IDYES }
    }

    pub fn info(text: &str) {
        let (t, c) = (w(text), w("Vibex Setup"));
        unsafe { MessageBoxW(std::ptr::null_mut(), t.as_ptr(), c.as_ptr(), MB_ICONINFORMATION) };
    }

    pub fn error(text: &str) {
        let (t, c) = (w(text), w("Vibex Setup"));
        unsafe { MessageBoxW(std::ptr::null_mut(), t.as_ptr(), c.as_ptr(), MB_ICONERROR) };
    }
}

// ------------------------------------------------------------ registry

#[cfg(windows)]
mod reg {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegGetValueW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_DWORD, REG_EXPAND_SZ,
        REG_OPTION_NON_VOLATILE, REG_SZ, RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ,
    };

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    fn open(path: &str) -> Result<HKEY, String> {
        let mut key: HKEY = null_mut();
        let p = w(path);
        let r = unsafe { RegCreateKeyExW(HKEY_CURRENT_USER, p.as_ptr(), 0, null(), REG_OPTION_NON_VOLATILE, KEY_WRITE, null(), &mut key, null_mut()) };
        if r != ERROR_SUCCESS {
            return Err(format!("cannot open HKCU\\{path} (error {r})"));
        }
        Ok(key)
    }

    /// Set a string value (`name = None` is the key's default value).
    pub fn set_str(path: &str, name: Option<&str>, value: &str) -> Result<(), String> {
        set_string(path, name, value, REG_SZ)
    }

    pub fn set_expand(path: &str, name: &str, value: &str) -> Result<(), String> {
        set_string(path, Some(name), value, REG_EXPAND_SZ)
    }

    fn set_string(path: &str, name: Option<&str>, value: &str, kind: u32) -> Result<(), String> {
        let key = open(path)?;
        let n = name.map(w);
        let v = w(value);
        let r = unsafe { RegSetValueExW(key, n.as_ref().map_or(null(), |n| n.as_ptr()), 0, kind, v.as_ptr() as *const u8, (v.len() * 2) as u32) };
        unsafe { RegCloseKey(key) };
        if r != ERROR_SUCCESS {
            return Err(format!("cannot write HKCU\\{path} (error {r})"));
        }
        Ok(())
    }

    pub fn set_dword(path: &str, name: &str, value: u32) -> Result<(), String> {
        let key = open(path)?;
        let n = w(name);
        let r = unsafe { RegSetValueExW(key, n.as_ptr(), 0, REG_DWORD, &value as *const u32 as *const u8, 4) };
        unsafe { RegCloseKey(key) };
        if r != ERROR_SUCCESS {
            return Err(format!("cannot write HKCU\\{path}\\{name} (error {r})"));
        }
        Ok(())
    }

    /// Read a string value without expanding %VARS%.
    pub fn get_str(path: &str, name: &str) -> Option<String> {
        let (p, n) = (w(path), w(name));
        let mut size: u32 = 0;
        let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND;
        let r = unsafe { RegGetValueW(HKEY_CURRENT_USER, p.as_ptr(), n.as_ptr(), flags, null_mut(), null_mut(), &mut size) };
        if r != ERROR_SUCCESS || size == 0 {
            return None;
        }
        let mut buf = vec![0u16; (size as usize).div_ceil(2)];
        let r = unsafe { RegGetValueW(HKEY_CURRENT_USER, p.as_ptr(), n.as_ptr(), flags, null_mut(), buf.as_mut_ptr() as *mut _, &mut size) };
        if r != ERROR_SUCCESS {
            return None;
        }
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..end]))
    }

    pub fn delete_tree(path: &str) {
        let p = w(path);
        unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, p.as_ptr()) };
        // RegDeleteTreeW removes the subtree's contents; remove the key too
        let parent_end = path.rfind('\\').unwrap_or(0);
        let (parent, leaf) = (&path[..parent_end], &path[parent_end + 1..]);
        let (pp, lp) = (w(parent), w(leaf));
        unsafe {
            let mut key: HKEY = null_mut();
            if windows_sys::Win32::System::Registry::RegOpenKeyExW(
                HKEY_CURRENT_USER,
                pp.as_ptr(),
                0,
                windows_sys::Win32::System::Registry::KEY_ALL_ACCESS,
                &mut key,
            ) == ERROR_SUCCESS
            {
                RegDeleteTreeW(key, lp.as_ptr());
                windows_sys::Win32::System::Registry::RegDeleteKeyW(key, lp.as_ptr());
                RegCloseKey(key);
            }
        }
    }
}

#[cfg(windows)]
fn broadcast_changes() {
    use windows_sys::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};
    use windows_sys::Win32::UI::WindowsAndMessaging::{SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE};
    let env: Vec<u16> = "Environment".encode_utf16().chain(Some(0)).collect();
    unsafe {
        let mut result = 0usize;
        SendMessageTimeoutW(HWND_BROADCAST, WM_SETTINGCHANGE, 0, env.as_ptr() as isize, SMTO_ABORTIFHUNG, 2000, &mut result);
        SHChangeNotify(SHCNE_ASSOCCHANGED as i32, SHCNF_IDLIST, std::ptr::null(), std::ptr::null());
    }
}

// --------------------------------------------------------------- PATH

/// Add/remove `dir` from the user PATH (HKCU\Environment), keeping the
/// value's other entries and type.
#[cfg(windows)]
fn edit_user_path(dir: &Path, add: bool) -> Result<(), String> {
    let dir_s = dir.to_string_lossy().to_string();
    let current = reg::get_str("Environment", "Path").unwrap_or_default();
    let mut parts: Vec<String> = current.split(';').filter(|p| !p.trim().is_empty()).map(str::to_string).collect();
    let had = parts.iter().any(|p| p.trim_end_matches('\\').eq_ignore_ascii_case(dir_s.trim_end_matches('\\')));
    if add && !had {
        parts.push(dir_s);
    } else if !add && had {
        parts.retain(|p| !p.trim_end_matches('\\').eq_ignore_ascii_case(dir_s.trim_end_matches('\\')));
    } else {
        return Ok(());
    }
    reg::set_expand("Environment", "Path", &parts.join(";"))
}

// ------------------------------------------------------------ shortcut

/// Create the Start menu shortcut with the shell's own COM object via
/// PowerShell (WScript.Shell), which ships with every Windows.
fn create_shortcut(lnk: &Path, target: &Path) -> Result<(), String> {
    let q = |p: &Path| p.to_string_lossy().replace('\'', "''");
    let script = format!(
        "$s = (New-Object -ComObject WScript.Shell).CreateShortcut('{lnk}'); \
         $s.TargetPath = '{exe}'; $s.IconLocation = '{exe},0'; \
         $s.WorkingDirectory = $env:USERPROFILE; $s.Description = 'Vibex Terminal'; $s.Save()",
        lnk = q(lnk),
        exe = q(target),
    );
    let status = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &script])
        .creation_flags_no_window()
        .status()
        .map_err(|e| format!("cannot run PowerShell for the shortcut: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("creating the Start menu shortcut failed".into())
    }
}

/// Unpack the editors into <dir>\tools (Windows' own tar reads the zip).
#[cfg(windows)]
fn install_tools(dir: &Path) -> Result<(), String> {
    let tools = dir.join("tools");
    let zip = std::env::temp_dir().join(format!("vibex-tools-{}.zip", std::process::id()));
    std::fs::write(&zip, TOOLS).map_err(|e| format!("cannot write {}: {e}", zip.display()))?;
    if tools.exists() {
        std::fs::remove_dir_all(&tools).map_err(|_| "nano or Neovim is still open from Vibex: close it, then run setup again.".to_string())?;
    }
    std::fs::create_dir_all(&tools).map_err(|e| format!("cannot create {}: {e}", tools.display()))?;
    let tar = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows")).join(r"System32\tar.exe");
    let status = std::process::Command::new(tar)
        .arg("-xf")
        .arg(&zip)
        .arg("-C")
        .arg(&tools)
        .creation_flags_no_window()
        .status()
        .map_err(|e| format!("cannot unpack the editors: {e}"));
    let _ = std::fs::remove_file(&zip);
    match status? {
        s if s.success() => Ok(()),
        _ => Err("unpacking nano and Neovim failed".into()),
    }
}

#[cfg(not(windows))]
fn install_tools(_dir: &Path) -> Result<(), String> {
    Ok(())
}

trait NoWindow {
    fn creation_flags_no_window(&mut self) -> &mut Self;
}

impl NoWindow for std::process::Command {
    fn creation_flags_no_window(&mut self) -> &mut Self {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            self.creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        }
        #[cfg(not(windows))]
        self
    }
}

// ------------------------------------------------------- install/remove

fn vibex_running(exe: &Path) -> bool {
    // a running exe can't be opened for writing
    exe.is_file() && std::fs::OpenOptions::new().write(true).open(exe).is_err()
}

#[cfg(windows)]
fn install() -> Result<PathBuf, String> {
    if HAS_PAYLOAD != "1" || PAYLOAD.is_empty() {
        return Err("This setup was built without Vibex inside it. Build it with tools\\package.ps1.".into());
    }
    let dir = install_dir();
    let exe = dir.join("vibex.exe");
    if vibex_running(&exe) {
        return Err("Vibex is running. Close every Vibex window, then run setup again.".into());
    }
    std::fs::create_dir_all(dir.join("shell")).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    std::fs::write(&exe, PAYLOAD).map_err(|e| format!("cannot write {}: {e}", exe.display()))?;
    std::fs::write(dir.join("shell").join("vibex.ps1"), SHELL_PS1).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("shell").join("vibex.bash"), SHELL_BASH).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("shell").join("vibex-commands.ps1"), SHELL_COMMANDS).map_err(|e| e.to_string())?;
    if !LAUNCHER.is_empty() {
        let com = dir.join("vibex.com");
        std::fs::write(&com, LAUNCHER).map_err(|e| format!("cannot write {}: {e}", com.display()))?;
    }
    if !TOOLS.is_empty() {
        install_tools(&dir)?;
    }
    if !OPENCONSOLE.is_empty() {
        let oc = dir.join("OpenConsole.exe");
        std::fs::write(&oc, OPENCONSOLE).map_err(|e| format!("cannot write {}: {e}", oc.display()))?;
        std::fs::write(dir.join("OpenConsole-LICENSE.txt"), OPENCONSOLE_LICENSE).map_err(|e| e.to_string())?;
    }
    // keep a copy of this setup as the uninstaller
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let uninstaller = dir.join("vibex-setup.exe");
    if !same_file(&me, &uninstaller) {
        std::fs::copy(&me, &uninstaller).map_err(|e| format!("cannot copy the uninstaller: {e}"))?;
    }

    let exe_s = exe.to_string_lossy().to_string();
    let un_s = uninstaller.to_string_lossy().to_string();
    let dir_s = dir.to_string_lossy().to_string();

    // Apps & features
    reg::set_str(UNINSTALL_KEY, Some("DisplayName"), "Vibex Terminal")?;
    reg::set_str(UNINSTALL_KEY, Some("DisplayVersion"), VERSION)?;
    reg::set_str(UNINSTALL_KEY, Some("Publisher"), "Kalyan Gopalam")?;
    reg::set_str(UNINSTALL_KEY, Some("DisplayIcon"), &format!("{exe_s},0"))?;
    reg::set_str(UNINSTALL_KEY, Some("InstallLocation"), &dir_s)?;
    reg::set_str(UNINSTALL_KEY, Some("UninstallString"), &format!("\"{un_s}\" --uninstall"))?;
    reg::set_str(UNINSTALL_KEY, Some("QuietUninstallString"), &format!("\"{un_s}\" --uninstall --quiet"))?;
    reg::set_dword(UNINSTALL_KEY, "NoModify", 1)?;
    reg::set_dword(UNINSTALL_KEY, "NoRepair", 1)?;
    reg::set_dword(UNINSTALL_KEY, "EstimatedSize", ((PAYLOAD.len() + OPENCONSOLE.len() + TOOLS.len() * 3 + 512 * 1024) / 1024) as u32)?;

    // Win+R "vibex"
    reg::set_str(APP_PATHS_KEY, None, &exe_s)?;
    reg::set_str(APP_PATHS_KEY, Some("Path"), &dir_s)?;

    // Explorer verbs
    for (key, text, cmd) in VERBS {
        reg::set_str(key, None, text)?;
        reg::set_str(key, Some("Icon"), &format!("\"{exe_s}\",0"))?;
        reg::set_str(&format!(r"{key}\command"), None, &cmd.replace("{exe}", &exe_s))?;
    }

    edit_user_path(&dir, true)?;
    if let Some(lnk) = start_menu_lnk() {
        create_shortcut(&lnk, &exe)?;
    }
    broadcast_changes();
    Ok(dir)
}

/// Wait (up to a minute) for process `pid` to end.
#[cfg(windows)]
fn wait_for_exit(pid: u32) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
    unsafe {
        let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if !h.is_null() {
            WaitForSingleObject(h, 60_000);
            CloseHandle(h);
        }
    }
    // the exe's file lock goes a moment after the process
    std::thread::sleep(std::time::Duration::from_millis(300));
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

#[cfg(windows)]
fn uninstall() -> Result<(), String> {
    let dir = install_dir();
    let exe = dir.join("vibex.exe");
    if vibex_running(&exe) {
        return Err("Vibex is running. Close every Vibex window, then uninstall again.".into());
    }
    for (key, _, _) in VERBS {
        reg::delete_tree(key);
    }
    reg::delete_tree(APP_PATHS_KEY);
    reg::delete_tree(UNINSTALL_KEY);
    let _ = edit_user_path(&dir, false);
    if let Some(lnk) = start_menu_lnk() {
        let _ = std::fs::remove_file(lnk);
    }
    broadcast_changes();

    // Files. If we're running from the install dir, finish from a temp copy
    // (a running exe can't delete itself).
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    if me.starts_with(&dir) {
        let tmp = std::env::temp_dir().join("vibex-uninstall.exe");
        std::fs::copy(&me, &tmp).map_err(|e| e.to_string())?;
        std::process::Command::new(&tmp).args(["--remove-files", &dir.to_string_lossy()]).creation_flags_no_window().spawn().map_err(|e| e.to_string())?;
        return Ok(());
    }
    remove_files(&dir);
    Ok(())
}

fn remove_files(dir: &Path) {
    // wait for the original uninstaller to exit, then delete
    for _ in 0..50 {
        if std::fs::remove_dir_all(dir).is_ok() || !dir.exists() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

#[cfg(windows)]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let quiet = args.iter().any(|a| a == "--quiet" || a == "/S");
    if let Some(i) = args.iter().position(|a| a == "--remove-files") {
        if let Some(dir) = args.get(i + 1) {
            remove_files(Path::new(dir));
        }
        return;
    }
    if args.iter().any(|a| a == "--uninstall") {
        if !quiet && !ui::ask("Remove Vibex Terminal?\n\nYour config in %USERPROFILE%\\.config\\vibex is kept.") {
            return;
        }
        match uninstall() {
            Ok(()) => {
                if !quiet {
                    ui::info("Vibex has been removed.");
                }
            }
            Err(e) => {
                if quiet {
                    eprintln!("{e}");
                } else {
                    ui::error(&e);
                }
                std::process::exit(1);
            }
        }
        return;
    }

    // an update started by Vibex: wait for it to exit first
    if let Some(pid) = args.iter().position(|a| a == "--wait-pid").and_then(|i| args.get(i + 1)).and_then(|p| p.parse().ok()) {
        wait_for_exit(pid);
    }
    let relaunch = args.iter().any(|a| a == "--relaunch");

    let dir = install_dir();
    if !quiet
        && !ui::ask(&format!(
            "Install Vibex Terminal {VERSION}?\n\n\
             • Installs to {}\n\
             • Start menu shortcut\n\
             • \"Open in Vibex\" on folders, \"Run in Vibex\" on .ps1/.bat/.cmd\n\
             • Adds vibex to your PATH\n\nNo admin rights needed.",
            dir.display()
        ))
    {
        return;
    }
    match install() {
        Ok(dir) if relaunch => {
            let _ = std::process::Command::new(dir.join("vibex.exe")).spawn();
        }
        Ok(dir) => {
            if !quiet {
                ui::info(&format!(
                    "Vibex is installed.\n\nStart it from the Start menu, type vibex in Win+R, or right-click a folder → Open in Vibex.\n\n(Installed in {})",
                    dir.display()
                ));
            }
        }
        Err(e) => {
            if quiet {
                eprintln!("{e}");
            } else {
                ui::error(&e);
            }
            std::process::exit(1);
        }
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("vibex-setup is Windows-only");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbs_reference_the_exe() {
        for (key, text, cmd) in VERBS {
            assert!(key.starts_with(r"Software\Classes\"), "{key}");
            assert!(!text.is_empty());
            assert!(cmd.contains("{exe}"));
        }
    }

    #[test]
    fn install_dir_is_per_user() {
        let d = install_dir();
        assert!(d.ends_with(r"Programs\Vibex"));
    }
}
