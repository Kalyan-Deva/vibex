//! `vibex.com`, installed next to `vibex.exe`.
//!
//! `vibex.exe` is a window program, and shells don't wait for those: type
//! `vibex --help` in PowerShell or cmd and the prompt comes straight back,
//! then the help text lands on top of it. Shells look for `.com` before
//! `.exe`, so typing `vibex` runs this small console program instead. For
//! commands that print something (`--help`, `cli`, `show`, `audit`, …) it
//! runs `vibex.exe`, waits, and passes its output and exit code through;
//! anything else opens the window and returns at once.
//!
//! Arguments go through as given (`args_os`, re-quoted by `std::process`
//! with the Windows rules), so spaces, quotes and any Unicode survive.

use std::path::PathBuf;
use std::process::{Command, Stdio};

/// First arguments whose output the shell should wait for.
const WAITS: &[&str] = &["cli", "show", "audit", "cast2gif", "editors", "--help", "-h", "--version", "-V", "--check-config", "--preview"];

fn waits(first: Option<&str>) -> bool {
    first.is_some_and(|a| WAITS.contains(&a))
}

/// `vibex.exe` beside this program, or why it can't be run.
fn target(me: std::io::Result<PathBuf>) -> Result<PathBuf, String> {
    let me = me.map_err(|e| format!("vibex: can't tell where vibex.com is: {e}"))?;
    let exe = me.with_file_name("vibex.exe");
    if exe.is_file() {
        Ok(exe)
    } else {
        Err(format!("vibex: {} is missing; reinstall Vibex", exe.display()))
    }
}

/// While `vibex.exe` runs in this console, Ctrl+C / Ctrl+Break reach it
/// too (it attaches to the console to print). This program ignores them
/// and keeps waiting, so the shell gets the prompt back only after
/// `vibex.exe` has stopped, with its exit code. A handler (not
/// `SetConsoleCtrlHandler(NULL, TRUE)`) because that "ignore" flag would be
/// inherited by `vibex.exe` and the shells it starts.
#[cfg(windows)]
mod ctrl {
    const CTRL_C_EVENT: u32 = 0;
    const CTRL_BREAK_EVENT: u32 = 1;

    #[link(name = "kernel32")]
    extern "system" {
        fn SetConsoleCtrlHandler(handler: Option<unsafe extern "system" fn(u32) -> i32>, add: i32) -> i32;
    }

    pub(crate) unsafe extern "system" fn handler(event: u32) -> i32 {
        // close / logoff / shutdown: default handling (exit)
        (event == CTRL_C_EVENT || event == CTRL_BREAK_EVENT) as i32
    }

    pub fn ignore_while_waiting() {
        unsafe { SetConsoleCtrlHandler(Some(handler), 1) };
    }
}

fn main() {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let exe = match target(std::env::current_exe()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let mut cmd = Command::new(&exe);
    cmd.args(&args);
    if waits(args.first().and_then(|a| a.to_str())) {
        // same console: output arrives in order, before the next prompt
        #[cfg(windows)]
        ctrl::ignore_while_waiting();
        match cmd.status() {
            // the whole 32-bit code (e.g. 0xC000013A after Ctrl+C) goes through
            Ok(s) => std::process::exit(s.code().unwrap_or(1)),
            Err(e) => {
                eprintln!("vibex: can't start {}: {e}", exe.display());
                std::process::exit(2);
            }
        }
    }
    // the window: let it go, give the prompt back
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    if let Err(e) = cmd.spawn() {
        eprintln!("vibex: can't start {}: {e}", exe.display());
        std::process::exit(2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn which_commands_wait() {
        assert!(waits(Some("--help")));
        assert!(waits(Some("cli")));
        assert!(waits(Some("audit")));
        assert!(!waits(None), "a plain `vibex` opens a window");
        assert!(!waits(Some("--profile")));
        assert!(!waits(Some("attach")), "it may open a window");
        assert!(!waits(Some("CLI")), "vibex.exe matches exactly too");
    }

    #[test]
    fn missing_vibex_exe_is_reported() {
        let dir = std::env::temp_dir().join(format!("vibex-launcher-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let me = dir.join("vibex.com");
        let e = target(Ok(me.clone())).unwrap_err();
        assert!(e.contains("vibex.exe is missing"), "{e}");
        std::fs::write(dir.join("vibex.exe"), b"").unwrap();
        assert_eq!(target(Ok(me)).unwrap(), dir.join("vibex.exe"));
        assert!(target(Err(std::io::Error::other("x"))).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn ctrl_c_and_break_are_left_to_vibex() {
        unsafe {
            assert_eq!(ctrl::handler(0), 1);
            assert_eq!(ctrl::handler(1), 1);
            assert_eq!(ctrl::handler(2), 0, "closing the console still ends us");
        }
    }

    /// What the launcher relies on: `Command::args` quoting, read back by
    /// a Rust program's `std::env::args` (as vibex.exe does). The child is
    /// this test binary, running only `echo_args`.
    #[test]
    fn awkward_arguments_survive() {
        let awkward = ["a b", r#"say "hi""#, r"C:\trailing\", r"C:\two\\", "ünï ☃ 日本", "%PATH%", "x & y | z ^ <", "tab\there"];
        let me = std::env::current_exe().unwrap();
        let out = Command::new(me)
            .args(["--exact", "tests::echo_args", "--nocapture", "--include-ignored"])
            .args(awkward)
            .env("VIBEX_ECHO_ARGS", "1")
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        let got: Vec<String> = text.lines().filter_map(|l| l.strip_prefix("ARG ")).map(unhex).collect();
        assert_eq!(got[got.len() - awkward.len()..], awkward, "{text}");
    }

    #[test]
    #[ignore]
    fn echo_args() {
        if std::env::var_os("VIBEX_ECHO_ARGS").is_some() {
            for a in std::env::args().skip(1) {
                println!("ARG {}", a.bytes().map(|b| format!("{b:02x}")).collect::<String>());
            }
        }
    }

    fn unhex(s: &str) -> String {
        String::from_utf8((0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()).unwrap()
    }
}
