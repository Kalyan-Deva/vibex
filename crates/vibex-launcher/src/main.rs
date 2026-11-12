//! `vibex.com`, installed next to `vibex.exe`.
//!
//! `vibex.exe` is a window program, and shells don't wait for those: type
//! `vibex --help` in PowerShell or cmd and the prompt comes straight back,
//! then the help text lands on top of it. Shells look for `.com` before
//! `.exe`, so typing `vibex` runs this small console program instead. For
//! commands that print something (`--help`, `cli`, `show`, `audit`, …) it
//! runs `vibex.exe`, waits, and passes its output and exit code through;
//! anything else opens the window and returns at once.

use std::process::{Command, Stdio};

/// First arguments whose output the shell should wait for.
const WAITS: &[&str] = &["cli", "show", "audit", "cast2gif", "editors", "--help", "-h", "--version", "-V", "--check-config", "--preview"];

fn waits(first: Option<&str>) -> bool {
    first.is_some_and(|a| WAITS.contains(&a))
}

fn main() {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let exe = match std::env::current_exe() {
        Ok(p) => p.with_file_name("vibex.exe"),
        Err(e) => {
            eprintln!("vibex: {e}");
            std::process::exit(2);
        }
    };
    let mut cmd = Command::new(&exe);
    cmd.args(&args);
    if waits(args.first().and_then(|a| a.to_str())) {
        // same console: output arrives in order, before the next prompt
        match cmd.status() {
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
    #[test]
    fn which_commands_wait() {
        assert!(super::waits(Some("--help")));
        assert!(super::waits(Some("cli")));
        assert!(super::waits(Some("audit")));
        assert!(!super::waits(None), "a plain `vibex` opens a window");
        assert!(!super::waits(Some("--profile")));
        assert!(!super::waits(Some("attach")), "it may open a window");
    }
}
