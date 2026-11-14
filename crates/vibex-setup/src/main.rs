//! Vibex Terminal setup: a single-file, per-user installer (no admin).
//!
//!   vibex-setup.exe               install (asks first)
//!   vibex-setup.exe --quiet       install without dialogs
//!   vibex-setup.exe --uninstall   remove everything it added
//!   (`/S` = `--quiet`; Vibex's updater adds `--wait-pid <pid> --relaunch`)
//!
//! Installs to %LOCALAPPDATA%\Programs\Vibex and registers: Start menu
//! shortcut, "Open in Vibex" on folders/drives/folder backgrounds, "Run in
//! Vibex" on .ps1/.bat/.cmd, `vibex` on PATH and in Win+R (App Paths), and an
//! Apps & features entry.
//!
//! Files are written next to their targets first (`*.vibex-new`), then
//! swapped in by renames; if any swap fails, the ones already done are
//! undone, so a failed install leaves the previous one as it was.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

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
///
/// `%V` / `%W` is `C:\` for a drive root, and `"C:\"` reads as `C:"` plus
/// the rest of the line (a backslash escapes the quote). The folder verbs
/// end with it and Vibex strips the stray quote; the script verbs have more
/// after it, so they pass `"%W\."` (`C:\\.`, `C:\dir\.`: the same folder).
const VERBS: &[(&str, &str, &str)] = &[
    (r"Software\Classes\Directory\shell\Vibex", "Open in Vibex", r#""{exe}" --cwd "%V""#),
    (r"Software\Classes\Directory\Background\shell\Vibex", "Open in Vibex", r#""{exe}" --cwd "%V""#),
    (r"Software\Classes\Drive\shell\Vibex", "Open in Vibex", r#""{exe}" --cwd "%V""#),
    (r"Software\Classes\SystemFileAssociations\.ps1\shell\RunInVibex", "Run in Vibex", r#""{exe}" --cwd "%W\." -- powershell.exe -NoLogo -File "%1""#),
    (r"Software\Classes\SystemFileAssociations\.bat\shell\RunInVibex", "Run in Vibex", r#""{exe}" --cwd "%W\." -- cmd.exe /c "%1""#),
    (r"Software\Classes\SystemFileAssociations\.cmd\shell\RunInVibex", "Run in Vibex", r#""{exe}" --cwd "%W\." -- cmd.exe /c "%1""#),
];

/// What setup puts in the install folder (the uninstaller removes these
/// and nothing else). `tools.new` / `tools.old` are Vibex's own editor
/// update folders.
const OWNED_FILES: &[&str] = &[
    "vibex.exe",
    "vibex.com",
    "vibex-setup.exe",
    "OpenConsole.exe",
    "OpenConsole-LICENSE.txt",
    r"shell\vibex.ps1",
    r"shell\vibex.bash",
    r"shell\vibex-commands.ps1",
];
const OWNED_DIRS: &[&str] = &["tools", "tools.new", "tools.old"];
/// Suffixes of the files setup stages and swaps (see [`Txn`]).
const NEW: &str = ".vibex-new";
const OLD: &str = ".vibex-old";

/// %LOCALAPPDATA%\Programs\Vibex. No fallback: a shared folder such as
/// C:\Users\Public would let other accounts swap the exe.
fn install_dir() -> Result<PathBuf, String> {
    match std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        Some(base) if base.is_absolute() => Ok(base.join("Programs").join("Vibex")),
        _ => Err("%LOCALAPPDATA% is not set, so there is no per-user Programs folder to install into.".into()),
    }
}

fn system32(rel: &str) -> PathBuf {
    std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows")).join("System32").join(rel)
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
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_SUCCESS};
    pub use windows_sys::Win32::System::Registry::REG_EXPAND_SZ;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegGetValueW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_DWORD, REG_OPTION_NON_VOLATILE,
        REG_SZ, RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ,
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

    /// Write a REG_SZ or REG_EXPAND_SZ (`kind`) value.
    pub fn set_string(path: &str, name: Option<&str>, value: &str, kind: u32) -> Result<(), String> {
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

    /// Read a string value and its type (REG_SZ / REG_EXPAND_SZ) without
    /// expanding %VARS%. `Ok(None)`: no such value. Any other failure is an
    /// error, so a caller never mistakes "couldn't read" for "empty".
    pub fn get_string(path: &str, name: &str) -> Result<Option<(String, u32)>, String> {
        let (p, n) = (w(path), w(name));
        let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND;
        let failed = |r: u32| format!("cannot read HKCU\\{path}\\{name} (error {r})");
        // the value can grow between asking its size and reading it
        for _ in 0..4 {
            let (mut kind, mut size) = (0u32, 0u32);
            let r = unsafe { RegGetValueW(HKEY_CURRENT_USER, p.as_ptr(), n.as_ptr(), flags, &mut kind, null_mut(), &mut size) };
            if r == ERROR_FILE_NOT_FOUND {
                return Ok(None);
            }
            if r != ERROR_SUCCESS {
                return Err(failed(r));
            }
            let mut buf = vec![0u16; (size as usize).div_ceil(2) + 1];
            let mut size = (buf.len() * 2) as u32;
            let r = unsafe { RegGetValueW(HKEY_CURRENT_USER, p.as_ptr(), n.as_ptr(), flags, &mut kind, buf.as_mut_ptr() as *mut _, &mut size) };
            if r == ERROR_MORE_DATA {
                continue;
            }
            if r != ERROR_SUCCESS {
                return Err(failed(r));
            }
            let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            let text = String::from_utf16(&buf[..end]).map_err(|_| format!("HKCU\\{path}\\{name} isn't valid text; leaving it alone"))?;
            return Ok(Some((text, kind)));
        }
        Err(format!("HKCU\\{path}\\{name} keeps changing; run setup again"))
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

/// Expand `%NAME%` with `lookup`; unknown names stay as written.
fn expand_vars(s: &str, lookup: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find('%').filter(|&j| j > 0).and_then(|j| lookup(&after[..j]).map(|v| (j, v))) {
            Some((j, v)) => {
                out.push_str(&v);
                rest = &after[j + 1..];
            }
            None => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The user PATH with `dir` added (at the end) or removed, or `None` when
/// nothing changes. Entries match case-insensitively, ignoring quotes, a
/// trailing `\` and `%VARS%` (via `expand`). Every other entry, empty ones
/// included, is kept exactly as it was; nothing is ever cut short.
fn merge_path(current: &str, dir: &str, add: bool, expand: &dyn Fn(&str) -> String) -> Option<String> {
    let norm = |s: &str| expand(s.trim().trim_matches('"')).trim_end_matches(['\\', '/']).to_lowercase();
    let want = norm(dir);
    let ours = |e: &str| !e.trim().is_empty() && norm(e) == want;
    let had = current.split(';').any(ours);
    match (add, had) {
        (true, false) if current.is_empty() => Some(dir.to_string()),
        // keep the trailing `;`, so removing `dir` again gives back `current`
        (true, false) if current.ends_with(';') => Some(format!("{current}{dir};")),
        (true, false) => Some(format!("{current};{dir}")),
        (false, true) => Some(current.split(';').filter(|e| !ours(e)).collect::<Vec<_>>().join(";")),
        _ => None,
    }
}

/// Add/remove `dir` from the user PATH (HKCU\Environment), keeping the
/// value's other entries and its type (REG_EXPAND_SZ or REG_SZ). If the
/// value can't be read it is left alone.
#[cfg(windows)]
fn edit_user_path(dir: &Path, add: bool) -> Result<(), String> {
    let (current, kind) = reg::get_string("Environment", "Path")?.unwrap_or((String::new(), reg::REG_EXPAND_SZ));
    let expand = |s: &str| expand_vars(s, &|name| std::env::var(name).ok());
    match merge_path(&current, &dir.to_string_lossy(), add, &expand) {
        Some(path) => reg::set_string("Environment", Some("Path"), &path, kind),
        None => Ok(()),
    }
}

// ------------------------------------------------------------ shortcut

/// The paths reach PowerShell as environment variables, never inside the
/// script text, so quotes (', ’), `$` or `;` in a path can't run anything.
const SHORTCUT_PS: &str = "$s = (New-Object -ComObject WScript.Shell).CreateShortcut($env:VIBEX_SETUP_LNK); \
     $s.TargetPath = $env:VIBEX_SETUP_TARGET; $s.IconLocation = $env:VIBEX_SETUP_TARGET + ',0'; \
     $s.WorkingDirectory = $env:USERPROFILE; $s.Description = 'Vibex Terminal'; $s.Save()";

/// Create the Start menu shortcut with the shell's own COM object via
/// PowerShell (WScript.Shell), which ships with every Windows.
fn create_shortcut(lnk: &Path, target: &Path) -> Result<(), String> {
    // System32's PowerShell by full path: a plain `powershell.exe` would be
    // looked up next to this setup first (often the Downloads folder)
    let status = std::process::Command::new(system32(r"WindowsPowerShell\v1.0\powershell.exe"))
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", SHORTCUT_PS])
        .env("VIBEX_SETUP_LNK", lnk)
        .env("VIBEX_SETUP_TARGET", target)
        .creation_flags_no_window()
        .status()
        .map_err(|e| format!("cannot run PowerShell for the shortcut: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("creating the Start menu shortcut failed".into())
    }
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

// ------------------------------------------------------ staged files

/// `<p><suffix>`, next to `p`.
fn sibling(p: &Path, suffix: &str) -> PathBuf {
    let mut name = p.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    p.with_file_name(name)
}

fn exists(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok()
}

/// Remove a file or a folder; a link is removed, never what it points at.
fn remove_any(p: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.file_type().is_symlink() => std::fs::remove_file(p).or_else(|_| std::fs::remove_dir(p)),
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(p),
        Ok(_) => std::fs::remove_file(p),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

fn write_synced(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::File::create(path)?;
    f.write_all(data)?;
    f.sync_all()
}

/// A `*.vibex-new` / `*.vibex-old*` left by an interrupted setup.
fn is_leftover(name: &str) -> bool {
    let owned =
        |stem: &str| stem == "tools.zip" || OWNED_DIRS.contains(&stem) || OWNED_FILES.iter().any(|f| Path::new(f).file_name().is_some_and(|n| n == stem));
    [NEW, OLD].iter().any(|m| name.find(m).is_some_and(|i| owned(&name[..i])))
}

struct Item {
    dest: PathBuf,
    new: PathBuf,
    /// where the previous `dest` was moved to
    old: Option<PathBuf>,
    placed: bool,
}

/// Files and folders written beside their destinations, then swapped in
/// together: `dest -> dest.vibex-old`, `dest.vibex-new -> dest`. A swap
/// that fails undoes the ones before it; dropping an uncommitted `Txn`
/// deletes what it staged. (Renaming works on a running exe, so a Vibex
/// started half-way through still never sees a half-written file.)
#[derive(Default)]
struct Txn {
    items: Vec<Item>,
    committed: bool,
}

fn in_use(dest: &Path, e: std::io::Error) -> String {
    format!("cannot replace {}: {e}\n\nClose Vibex (and any nano or Neovim it opened), then run setup again.", dest.display())
}

impl Txn {
    fn stage_file(&mut self, dest: &Path, data: &[u8]) -> Result<(), String> {
        let new = sibling(dest, NEW);
        let _ = remove_any(&new);
        // pushed first, so a half-written `new` is cleaned up on drop
        self.items.push(Item { dest: dest.to_path_buf(), new: new.clone(), old: None, placed: false });
        write_synced(&new, data).map_err(|e| format!("cannot write {}: {e}", new.display()))
    }

    /// `new` (already filled) becomes `dest`.
    fn stage_dir(&mut self, dest: &Path, new: PathBuf) {
        self.items.push(Item { dest: dest.to_path_buf(), new, old: None, placed: false });
    }

    fn commit(&mut self) -> Result<(), String> {
        for i in 0..self.items.len() {
            if let Err(e) = Self::place(&mut self.items[i]) {
                self.rollback();
                return Err(e);
            }
        }
        self.committed = true;
        Ok(())
    }

    fn place(item: &mut Item) -> Result<(), String> {
        if exists(&item.dest) {
            let mut old = sibling(&item.dest, OLD);
            if remove_any(&old).is_err() {
                // an old copy still running from an earlier update
                old = sibling(&item.dest, &format!("{OLD}-{}", std::process::id()));
                let _ = remove_any(&old);
            }
            std::fs::rename(&item.dest, &old).map_err(|e| in_use(&item.dest, e))?;
            item.old = Some(old);
        }
        std::fs::rename(&item.new, &item.dest).map_err(|e| in_use(&item.dest, e))?;
        item.placed = true;
        Ok(())
    }

    /// Put every destination back as it was and delete what was staged.
    fn rollback(&mut self) {
        for item in self.items.iter_mut().rev() {
            if item.placed && std::fs::rename(&item.dest, &item.new).is_ok() {
                item.placed = false;
            }
            if let Some(old) = item.old.take() {
                if !item.placed || remove_any(&item.dest).is_ok() {
                    let _ = std::fs::rename(&old, &item.dest);
                }
            }
            let _ = remove_any(&item.new);
        }
        self.items.clear();
    }

    /// After a commit: delete the previous copies (best effort: one still
    /// in use stays as `*.vibex-old*` until the next setup or uninstall).
    fn finish(mut self) {
        for item in &self.items {
            if let Some(old) = &item.old {
                let _ = remove_any(old);
            }
        }
        self.items.clear();
    }
}

impl Drop for Txn {
    fn drop(&mut self) {
        if !self.committed {
            self.rollback();
        }
    }
}

/// Delete leftovers of an interrupted setup (best effort).
fn sweep_leftovers(dir: &Path) {
    for sub in [dir.to_path_buf(), dir.join("shell")] {
        for e in std::fs::read_dir(&sub).into_iter().flatten().flatten() {
            if is_leftover(&e.file_name().to_string_lossy()) {
                let _ = remove_any(&e.path());
            }
        }
    }
}

/// Unpack the editors into `<dir>\tools.vibex-new` (Windows' own tar reads
/// the zip), ready to be swapped in.
#[cfg(windows)]
fn stage_tools(dir: &Path) -> Result<PathBuf, String> {
    let new = sibling(&dir.join("tools"), NEW);
    let zip = sibling(&dir.join("tools.zip"), NEW);
    let _ = remove_any(&new);
    let _ = remove_any(&zip);
    std::fs::create_dir_all(&new).map_err(|e| format!("cannot create {}: {e}", new.display()))?;
    // inside the (per-user) install folder, not a guessable %TEMP% name
    if let Err(e) = write_synced(&zip, TOOLS) {
        let _ = remove_any(&new);
        return Err(format!("cannot write {}: {e}", zip.display()));
    }
    let status = std::process::Command::new(system32("tar.exe")).arg("-xf").arg(&zip).arg("-C").arg(&new).creation_flags_no_window().status();
    let _ = remove_any(&zip);
    match status {
        Ok(s) if s.success() => Ok(new),
        Ok(_) => {
            let _ = remove_any(&new);
            Err("unpacking nano and Neovim failed".into())
        }
        Err(e) => {
            let _ = remove_any(&new);
            Err(format!("cannot unpack the editors: {e}"))
        }
    }
}

// ------------------------------------------------------- install/remove

fn vibex_running(exe: &Path) -> bool {
    // a running exe can't be opened for writing
    exe.is_file() && std::fs::OpenOptions::new().write(true).open(exe).is_err()
}

/// Give closing Vibex windows up to `timeout` to let go of the exe.
fn wait_until_free(exe: &Path, timeout: Duration) {
    let start = Instant::now();
    while vibex_running(exe) && start.elapsed() < timeout {
        std::thread::sleep(Duration::from_millis(250));
    }
}

#[cfg(windows)]
fn install(lock_wait: Duration) -> Result<PathBuf, String> {
    if HAS_PAYLOAD != "1" || PAYLOAD.is_empty() {
        return Err("This setup was built without Vibex inside it. Build it with tools\\package.ps1.".into());
    }
    let dir = install_dir()?;
    let exe = dir.join("vibex.exe");
    wait_until_free(&exe, lock_wait);
    if vibex_running(&exe) {
        return Err("Vibex is running. Close every Vibex window, then run setup again.".into());
    }
    let shell = dir.join("shell");
    std::fs::create_dir_all(&shell).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    sweep_leftovers(&dir);

    // 1. write everything beside where it goes
    let mut tx = Txn::default();
    tx.stage_file(&exe, PAYLOAD)?;
    tx.stage_file(&shell.join("vibex.ps1"), SHELL_PS1)?;
    tx.stage_file(&shell.join("vibex.bash"), SHELL_BASH)?;
    tx.stage_file(&shell.join("vibex-commands.ps1"), SHELL_COMMANDS)?;
    if !LAUNCHER.is_empty() {
        tx.stage_file(&dir.join("vibex.com"), LAUNCHER)?;
    }
    if !OPENCONSOLE.is_empty() {
        tx.stage_file(&dir.join("OpenConsole.exe"), OPENCONSOLE)?;
        tx.stage_file(&dir.join("OpenConsole-LICENSE.txt"), OPENCONSOLE_LICENSE.as_bytes())?;
    }
    // keep a copy of this setup as the uninstaller
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let uninstaller = dir.join("vibex-setup.exe");
    if !same_file(&me, &uninstaller) {
        let data = std::fs::read(&me).map_err(|e| format!("cannot read {}: {e}", me.display()))?;
        tx.stage_file(&uninstaller, &data)?;
    }
    if !TOOLS.is_empty() {
        let new = stage_tools(&dir)?;
        tx.stage_dir(&dir.join("tools"), new);
    }
    // 2. swap it all in, or none of it
    tx.commit()?;
    tx.finish();

    register(&dir, &exe, &uninstaller)?;
    Ok(dir)
}

#[cfg(windows)]
fn register(dir: &Path, exe: &Path, uninstaller: &Path) -> Result<(), String> {
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

    edit_user_path(dir, true)?;
    if let Some(lnk) = start_menu_lnk() {
        create_shortcut(&lnk, exe)?;
    }
    broadcast_changes();
    Ok(())
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
    std::thread::sleep(Duration::from_millis(300));
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Lower-cased, canonical when it exists, without a trailing `\`.
fn path_key(p: &Path) -> String {
    let p = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    p.to_string_lossy().trim_end_matches(['\\', '/']).to_lowercase()
}

fn is_inside(file: &Path, dir: &Path) -> bool {
    let (f, d) = (path_key(file), path_key(dir));
    f.strip_prefix(&d).is_some_and(|rest| rest.starts_with('\\') || rest.starts_with('/'))
}

#[cfg(windows)]
fn uninstall() -> Result<(), String> {
    let dir = install_dir()?;
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
    if is_inside(&me, &dir) {
        let tmp = std::env::temp_dir().join(format!("vibex-uninstall-{}.exe", std::process::id()));
        std::fs::copy(&me, &tmp).map_err(|e| e.to_string())?;
        std::process::Command::new(&tmp).arg("--remove-files").arg(&dir).creation_flags_no_window().spawn().map_err(|e| e.to_string())?;
        return Ok(());
    }
    remove_files(&dir);
    Ok(())
}

/// Delete what setup put in `dir` (and leftovers of interrupted setups);
/// true when all of it is gone. Anything else in the folder stays, and so
/// does the folder then. The config (%USERPROFILE%\.config\vibex) is never
/// in here.
fn remove_owned(dir: &Path) -> bool {
    let mut all = true;
    for rel in OWNED_FILES.iter().chain(OWNED_DIRS) {
        all &= remove_any(&dir.join(rel)).is_ok();
    }
    for sub in [dir.to_path_buf(), dir.join("shell")] {
        for e in std::fs::read_dir(&sub).into_iter().flatten().flatten() {
            if is_leftover(&e.file_name().to_string_lossy()) {
                all &= remove_any(&e.path()).is_ok();
            }
        }
    }
    // only removed when empty
    let _ = std::fs::remove_dir(dir.join("shell"));
    let _ = std::fs::remove_dir(dir);
    all
}

fn remove_files(dir: &Path) {
    // wait for the original uninstaller to exit, then delete
    for _ in 0..50 {
        if remove_owned(dir) {
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Command line. Unknown arguments are ignored.
#[derive(Debug, Default, PartialEq)]
struct Opts {
    quiet: bool,
    uninstall: bool,
    remove_files: Option<String>,
    wait_pid: Option<u32>,
    relaunch: bool,
}

impl Opts {
    fn parse(args: &[String]) -> Opts {
        let value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
        Opts {
            quiet: args.iter().any(|a| a == "--quiet" || a.eq_ignore_ascii_case("/S")),
            uninstall: args.iter().any(|a| a == "--uninstall"),
            remove_files: value("--remove-files"),
            wait_pid: value("--wait-pid").and_then(|p| p.parse().ok()).filter(|&p| p != 0),
            relaunch: args.iter().any(|a| a == "--relaunch"),
        }
    }
}

/// Start the installed Vibex (after an update), from the home folder like
/// the Start menu shortcut does.
#[cfg(windows)]
fn relaunch() {
    let Ok(dir) = install_dir() else { return };
    let exe = dir.join("vibex.exe");
    if exe.is_file() {
        let mut cmd = std::process::Command::new(exe);
        if let Some(home) = std::env::var_os("USERPROFILE") {
            cmd.current_dir(home);
        }
        let _ = cmd.spawn();
    }
}

#[cfg(windows)]
fn fail(quiet: bool, e: &str) -> ! {
    if quiet {
        eprintln!("{e}");
    } else {
        ui::error(e);
    }
    std::process::exit(1);
}

#[cfg(windows)]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opts = Opts::parse(&args);
    let quiet = opts.quiet;
    if let Some(dir) = &opts.remove_files {
        // internal (the uninstaller's temp copy): only ever our own folder
        if install_dir().is_ok_and(|d| path_key(Path::new(dir)) == path_key(&d)) {
            remove_files(Path::new(dir));
        }
        return;
    }
    if opts.uninstall {
        if !quiet && !ui::ask("Remove Vibex Terminal?\n\nYour config in %USERPROFILE%\\.config\\vibex is kept.") {
            return;
        }
        match uninstall() {
            Ok(()) if !quiet => ui::info("Vibex has been removed."),
            Ok(()) => {}
            Err(e) => fail(quiet, &e),
        }
        return;
    }

    // an update started by Vibex: wait for it to exit first
    if let Some(pid) = opts.wait_pid {
        wait_for_exit(pid);
    }
    let dir = match install_dir() {
        Ok(d) => d,
        Err(e) => fail(quiet, &e),
    };
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
        if opts.relaunch {
            relaunch();
        }
        return;
    }
    // after an update, other Vibex windows may still be closing
    let lock_wait = if opts.wait_pid.is_some() { Duration::from_secs(20) } else { Duration::ZERO };
    match install(lock_wait) {
        Ok(_) if opts.relaunch => relaunch(),
        Ok(dir) => {
            if !quiet {
                ui::info(&format!(
                    "Vibex is installed.\n\nStart it from the Start menu, type vibex in Win+R, or right-click a folder → Open in Vibex.\n\n(Installed in {})",
                    dir.display()
                ));
            }
        }
        Err(e) => {
            // Vibex closed itself for this update: bring the old one back
            if opts.relaunch {
                relaunch();
            }
            fail(quiet, &e);
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

    /// A fresh, empty folder under %TEMP% for one test.
    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("vibex-setup-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn read(p: &Path) -> String {
        std::fs::read_to_string(p).unwrap()
    }

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
        let d = install_dir().unwrap();
        assert!(d.ends_with(r"Programs\Vibex"));
        assert!(d.starts_with(std::env::var_os("LOCALAPPDATA").unwrap()));
    }

    /// How Windows (CommandLineToArgvW, and so Rust's `std::env::args`)
    /// splits a command line.
    #[cfg(windows)]
    fn argv(cmdline: &str) -> Vec<String> {
        use windows_sys::Win32::UI::Shell::CommandLineToArgvW;
        let w: Vec<u16> = cmdline.encode_utf16().chain(Some(0)).collect();
        let mut n = 0;
        unsafe {
            let list = CommandLineToArgvW(w.as_ptr(), &mut n);
            assert!(!list.is_null());
            let args = (0..n as usize)
                .map(|i| {
                    let p = *list.add(i);
                    let len = (0..).take_while(|&j| *p.add(j) != 0).count();
                    String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
                })
                .collect();
            windows_sys::Win32::Foundation::LocalFree(list as _);
            args
        }
    }

    /// Explorer's substitution of `%V` / `%W` / `%1`.
    fn expand_verb(cmd: &str, exe: &str, folder: &str, file: &str) -> String {
        cmd.replace("{exe}", exe).replace("%V", folder).replace("%W", folder).replace("%1", file)
    }

    #[cfg(windows)]
    #[test]
    fn verbs_survive_spaces_and_drive_roots() {
        let exe = r"C:\Users\A B\AppData\Local\Programs\Vibex\vibex.exe";
        for (key, _, cmd) in VERBS {
            for (folder, file) in [(r"C:\", r"C:\run me.bat"), (r"D:\my files", r"D:\my files\it's & 100%.ps1")] {
                let args = argv(&expand_verb(cmd, exe, folder, file));
                assert_eq!(args[0], exe, "{key}");
                assert_eq!(args[1], "--cwd");
                // what Vibex makes of it (it trims quotes and a trailing `\`)
                let cwd = args[2].trim_matches('"').trim_end_matches('\\');
                let cwd = std::path::absolute(if cwd.ends_with(':') { format!("{cwd}\\") } else { cwd.to_string() }).unwrap();
                assert_eq!(cwd, Path::new(folder), "{key}: {args:?}");
                if key.contains("RunInVibex") {
                    assert_eq!(args[3], "--", "{key}: {args:?}");
                    assert_eq!(args.last().unwrap(), file, "{key}: {args:?}");
                } else {
                    assert_eq!(args.len(), 3, "{key}: {args:?}");
                }
            }
        }
    }

    #[test]
    fn path_add_and_remove() {
        let none = |s: &str| s.to_string();
        let dir = r"C:\Users\me\AppData\Local\Programs\Vibex";
        assert_eq!(merge_path("", dir, true, &none).as_deref(), Some(dir));
        assert_eq!(merge_path(r"C:\a;C:\b", dir, true, &none), Some(format!(r"C:\a;C:\b;{dir}")));
        assert_eq!(merge_path(r"C:\a;", dir, true, &none), Some(format!(r"C:\a;{dir};")), "no `;;`");
        // already there, in any spelling: nothing to do
        for spelled in [dir.to_string(), format!("{dir}\\"), dir.to_uppercase(), format!("\"{dir}\""), format!(" {dir} ")] {
            assert_eq!(merge_path(&format!(r"C:\a;{spelled};C:\b"), dir, true, &none), None, "{spelled}");
        }
        let vars = |s: &str| expand_vars(s, &|n| (n == "LOCALAPPDATA").then(|| r"C:\Users\me\AppData\Local".to_string()));
        assert_eq!(merge_path(r"%LOCALAPPDATA%\Programs\Vibex;C:\a", dir, true, &vars), None, "unexpanded entry counts");
        // removal keeps everything else exactly, empty entries and %VARS% included
        assert_eq!(merge_path(&format!(r"%SystemRoot%\x;;{dir};C:\b;{dir}\"), dir, false, &none).as_deref(), Some(r"%SystemRoot%\x;;C:\b"));
        assert_eq!(merge_path(r"C:\a;C:\b", dir, false, &none), None);
        assert_eq!(merge_path(dir, dir, false, &none).as_deref(), Some(""));
        // a long PATH is never cut
        let long: String = (0..400).map(|i| format!(r"C:\tools\some-long-folder-name-{i};")).collect();
        let merged = merge_path(&long, dir, true, &none).unwrap();
        assert!(merged.len() > 10_000 && merged.starts_with(&long) && merged.ends_with(&format!("{dir};")));
        assert_eq!(merge_path(&merged, dir, false, &none), Some(long.clone()), "add then remove gives the same PATH back");
        assert_eq!(merge_path(r"C:\a", dir, true, &none).and_then(|p| merge_path(&p, dir, false, &none)).as_deref(), Some(r"C:\a"));
    }

    #[test]
    fn env_vars_expand() {
        let look = |n: &str| match n {
            "A" => Some("x".to_string()),
            "B" => Some("%A%".to_string()),
            _ => None,
        };
        assert_eq!(expand_vars("%A%\\%B%", &look), "x\\%A%", "expanded once, not recursively");
        assert_eq!(expand_vars("100% %NOPE% %A%", &look), "100% %NOPE% x");
        assert_eq!(expand_vars("%%A%", &look), "%x");
        assert_eq!(expand_vars("trailing %", &look), "trailing %");
    }

    #[test]
    fn shortcut_script_has_no_paths_in_it() {
        assert!(SHORTCUT_PS.contains("$env:VIBEX_SETUP_LNK") && SHORTCUT_PS.contains("$env:VIBEX_SETUP_TARGET"));
        assert!(!SHORTCUT_PS.contains('"'));
    }

    /// Makes a real .lnk in a temp folder (not the Start menu) through a
    /// path full of PowerShell syntax.
    #[cfg(windows)]
    #[test]
    fn shortcut_paths_cant_inject() {
        let d = scratch("lnk");
        let marker_name = format!("vibex-pwned-{}", std::process::id());
        let marker = PathBuf::from(std::env::var_os("TEMP").unwrap()).join(&marker_name);
        let evil = d.join(format!("it's ’q’ $(New-Item -Path $env:TEMP -Name {marker_name}) ; x.lnk"));
        create_shortcut(&evil, &system32("notepad.exe")).unwrap();
        assert!(evil.is_file(), "the shortcut is made at exactly that path");
        assert!(!marker.exists(), "nothing in the path ran");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn txn_replaces_everything() {
        let d = scratch("txn-ok");
        std::fs::write(d.join("a"), "old a").unwrap();
        std::fs::create_dir_all(d.join("tools")).unwrap();
        std::fs::write(d.join("tools").join("old.txt"), "x").unwrap();
        let fresh = d.join("tools.vibex-new");
        std::fs::create_dir_all(&fresh).unwrap();
        std::fs::write(fresh.join("new.txt"), "y").unwrap();

        let mut tx = Txn::default();
        tx.stage_file(&d.join("a"), b"new a").unwrap();
        tx.stage_file(&d.join("b"), b"new b").unwrap();
        tx.stage_dir(&d.join("tools"), fresh);
        assert_eq!(read(&d.join("a")), "old a", "nothing changes before the commit");
        tx.commit().unwrap();
        tx.finish();
        assert_eq!(read(&d.join("a")), "new a");
        assert_eq!(read(&d.join("b")), "new b");
        assert!(d.join("tools").join("new.txt").is_file() && !d.join("tools").join("old.txt").exists());
        let names: Vec<String> = std::fs::read_dir(&d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert!(names.iter().all(|n| !n.contains(".vibex-")), "no leftovers: {names:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn txn_dropped_before_commit_leaves_nothing() {
        let d = scratch("txn-drop");
        std::fs::write(d.join("a"), "old a").unwrap();
        {
            let mut tx = Txn::default();
            tx.stage_file(&d.join("a"), b"new a").unwrap();
            tx.stage_file(&d.join("b"), b"new b").unwrap();
        }
        assert_eq!(read(&d.join("a")), "old a");
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The second file is held open with no sharing (like a busy file):
    /// the commit fails and the first file is put back.
    #[cfg(windows)]
    #[test]
    fn txn_rolls_back_when_a_file_is_busy() {
        use std::os::windows::fs::OpenOptionsExt;
        let d = scratch("txn-busy");
        std::fs::write(d.join("a"), "old a").unwrap();
        std::fs::write(d.join("b"), "old b").unwrap();
        let mut tx = Txn::default();
        tx.stage_file(&d.join("a"), b"new a").unwrap();
        tx.stage_file(&d.join("b"), b"new b").unwrap();
        tx.stage_file(&d.join("c"), b"new c").unwrap();
        let lock = std::fs::OpenOptions::new().read(true).share_mode(0).open(d.join("b")).unwrap();
        let e = tx.commit().unwrap_err();
        assert!(e.contains("cannot replace") && e.contains("Close Vibex"), "{e}");
        drop(tx);
        drop(lock);
        assert_eq!(read(&d.join("a")), "old a");
        assert_eq!(read(&d.join("b")), "old b");
        assert!(!d.join("c").exists());
        let names: Vec<String> = std::fs::read_dir(&d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names.len(), 2, "{names:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn uninstall_removes_only_its_own_files() {
        let d = scratch("owned").join("Vibex");
        std::fs::create_dir_all(d.join("shell")).unwrap();
        std::fs::create_dir_all(d.join("tools").join("nvim")).unwrap();
        for f in OWNED_FILES {
            std::fs::write(d.join(f), "x").unwrap();
        }
        std::fs::write(d.join("vibex.exe.vibex-old-123"), "x").unwrap();
        std::fs::write(d.join("my-notes.txt"), "keep me").unwrap();
        std::fs::write(d.join("shell").join("mine.ps1"), "keep me").unwrap();
        assert!(remove_owned(&d));
        assert_eq!(read(&d.join("my-notes.txt")), "keep me");
        assert_eq!(read(&d.join("shell").join("mine.ps1")), "keep me");
        let left: Vec<String> = std::fs::read_dir(&d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(left.len(), 2, "{left:?}");
        // with only our files, the folder goes too
        std::fs::remove_file(d.join("my-notes.txt")).unwrap();
        std::fs::remove_file(d.join("shell").join("mine.ps1")).unwrap();
        std::fs::write(d.join("vibex.exe"), "x").unwrap();
        assert!(remove_owned(&d));
        assert!(!d.exists());
        let _ = std::fs::remove_dir_all(d.parent().unwrap());
    }

    #[test]
    fn leftovers_are_recognised() {
        assert!(is_leftover("vibex.exe.vibex-new"));
        assert!(is_leftover("vibex.exe.vibex-old-4242"));
        assert!(is_leftover("tools.vibex-old"));
        assert!(is_leftover("tools.zip.vibex-new"));
        assert!(is_leftover("vibex.ps1.vibex-new"));
        assert!(!is_leftover("notes.txt.vibex-new"), "not ours");
        assert!(!is_leftover("vibex.exe"));
    }

    #[test]
    fn is_inside_ignores_case_and_prefix_tricks() {
        let d = scratch("inside");
        let f = d.join("vibex-setup.exe");
        std::fs::write(&f, "x").unwrap();
        assert!(is_inside(&f, &d));
        assert!(is_inside(Path::new(&f.to_string_lossy().to_uppercase()), &d));
        let twin = PathBuf::from(format!("{}x", d.display()));
        std::fs::create_dir_all(&twin).unwrap();
        std::fs::write(twin.join("a.exe"), "x").unwrap();
        assert!(!is_inside(&twin.join("a.exe"), &d), "Vibex2 isn't inside Vibex");
        assert!(!is_inside(&d, &d));
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(&twin);
    }

    #[test]
    fn flags() {
        let s = |v: &[&str]| v.iter().map(|a| a.to_string()).collect::<Vec<_>>();
        assert_eq!(Opts::parse(&[]), Opts::default());
        let o = Opts::parse(&s(&["--quiet", "--wait-pid", "1234", "--relaunch"]));
        assert_eq!(o, Opts { quiet: true, wait_pid: Some(1234), relaunch: true, ..Opts::default() });
        assert!(Opts::parse(&s(&["/S"])).quiet && Opts::parse(&s(&["/s"])).quiet);
        assert!(Opts::parse(&s(&["--uninstall", "/S"])).uninstall);
        assert_eq!(Opts::parse(&s(&["--wait-pid"])).wait_pid, None);
        assert_eq!(Opts::parse(&s(&["--wait-pid", "x"])).wait_pid, None);
        assert_eq!(Opts::parse(&s(&["--wait-pid", "0"])).wait_pid, None);
        assert_eq!(Opts::parse(&s(&["--wait-pid", "-5"])).wait_pid, None);
        assert_eq!(Opts::parse(&s(&["--remove-files", r"C:\x y"])).remove_files.as_deref(), Some(r"C:\x y"));
    }

    #[test]
    fn remove_files_refuses_other_folders() {
        // the guard in main: only the install folder itself
        let d = install_dir().unwrap();
        assert_eq!(path_key(&d), path_key(Path::new(&d.to_string_lossy().to_uppercase())));
        assert_ne!(path_key(Path::new(r"C:\Users")), path_key(&d));
        assert_ne!(path_key(&d.join("..")), path_key(&d));
    }
}
