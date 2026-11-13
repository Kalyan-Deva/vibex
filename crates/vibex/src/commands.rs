//! Both kinds of commands in every shell (`config.linux_commands`):
//! Linux ones (`ls -la`, `grep -rn`, `rm -rf`, `which` …) in PowerShell
//! and cmd, Windows ones (`where`, `ipconfig`, `cls` …) in WSL.
//!
//! The work is in the shell scripts (`shell/vibex-commands.ps1`,
//! `shell/vibex.bash`); here Vibex finds the GNU tools (Git for Windows'
//! `usr\bin`), tells the shells about them and gives cmd its macros.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use vibex_pty::Command;

/// Git for Windows' GNU tools (grep, sed, awk, find, ls …), if installed.
/// `VIBEX_GNU` points somewhere else (any folder with grep.exe).
pub fn gnu_dir() -> Option<&'static Path> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(find_gnu).as_deref()
}

fn find_gnu() -> Option<PathBuf> {
    let has = |d: &Path| d.join("grep.exe").is_file() && d.join("ls.exe").is_file();
    if let Some(d) = std::env::var_os("VIBEX_GNU").map(PathBuf::from).filter(|d| has(d)) {
        return Some(d);
    }
    let mut tries: Vec<PathBuf> = Vec::new();
    for var in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
        if let Some(p) = std::env::var_os(var) {
            tries.push(PathBuf::from(p).join("Git").join("usr").join("bin"));
        }
    }
    if let Some(p) = std::env::var_os("LOCALAPPDATA") {
        tries.push(PathBuf::from(p).join("Programs").join("Git").join("usr").join("bin"));
    }
    // next to git.exe (…\Git\cmd\git.exe -> …\Git\usr\bin)
    if let Some(path) = std::env::var_os("PATH") {
        for d in std::env::split_paths(&path) {
            if d.join("git.exe").is_file() {
                if let Some(root) = d.parent() {
                    tries.push(root.join("usr").join("bin"));
                }
            }
        }
    }
    tries.into_iter().find(|d| has(d))
}

fn is_program(cmd: &Command, name: &str) -> bool {
    let file = Path::new(&cmd.program).file_name().map(|f| f.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    file == name || file == format!("{name}.exe")
}

/// Add what the shell scripts need to a shell about to start.
#[cfg(test)]
pub fn prepare(cmd: &mut Command, on: bool) {
    prepare_with(cmd, on, "default")
}

/// `prepare`, plus the prompt style the shell scripts should set
/// (`config.prompt`: "dragon" is Kali Linux's two-line prompt).
pub fn prepare_with(cmd: &mut Command, on: bool, prompt: &str) {
    cmd.env.push(("VIBEX_PROMPT".into(), prompt.into()));
    // the bundled nano and Neovim: after everything else on PATH, so an
    // editor installed separately still wins
    let extra = crate::bundle::path_dirs();
    if !extra.is_empty() {
        let base =
            cmd.env.iter().find(|(k, _)| k.eq_ignore_ascii_case("PATH")).map(|(_, v)| v.clone()).or_else(|| std::env::var("PATH").ok()).unwrap_or_default();
        let add: Vec<String> = extra.iter().map(|d| d.to_string_lossy().into_owned()).filter(|d| !base.split(';').any(|p| p.eq_ignore_ascii_case(d))).collect();
        if !add.is_empty() {
            let path = format!("{};{}", base.trim_end_matches(';'), add.join(";"));
            cmd.env.retain(|(k, _)| !k.eq_ignore_ascii_case("PATH"));
            cmd.env.push(("PATH".into(), path));
        }
    }
    cmd.env.push(("VIBEX_COMMANDS".into(), if on { "1" } else { "0" }.into()));
    // into WSL too
    let has_cell = cmd.env.iter().any(|(k, _)| k == "VIBEX_CELL_PX");
    if let Some((_, v)) = cmd.env.iter_mut().find(|(k, _)| k == "WSLENV") {
        for name in ["VIBEX_COMMANDS", "VIBEX_PROMPT", "VIBEX_CELL_PX"] {
            if name == "VIBEX_CELL_PX" && !has_cell {
                continue;
            }
            if !v.split(':').any(|p| p == name) {
                v.push(':');
                v.push_str(name);
            }
        }
    }
    if !on {
        return;
    }
    let gnu = gnu_dir();
    if let Some(g) = gnu {
        cmd.env.push(("VIBEX_GNU".into(), g.to_string_lossy().into_owned()));
    }
    // cmd: typed-command macros (batch files never see them). Only when
    // it's started plainly, not with /c or /k of its own.
    if is_program(cmd, "cmd") && cmd.args.is_empty() {
        if let Some(file) = write_macros(gnu) {
            cmd.args = vec!["/K".into(), "doskey".into(), format!("/macrofile={}", file.display())];
        }
    }
}

/// The doskey macro file, somewhere without spaces in its path (doskey
/// can't take a quoted one).
fn write_macros(gnu: Option<&Path>) -> Option<PathBuf> {
    let text = doskey_macros(gnu);
    let dirs = [crate::config::config_dir(), std::env::var_os("TEMP").map(PathBuf::from)];
    for dir in dirs.into_iter().flatten() {
        let file = dir.join("cmd-macros.doskey");
        if file.to_string_lossy().contains(' ') {
            continue;
        }
        if std::fs::read_to_string(&file).is_ok_and(|t| t == text) {
            return Some(file);
        }
        if std::fs::create_dir_all(&dir).is_ok() && std::fs::write(&file, &text).is_ok() {
            return Some(file);
        }
    }
    None
}

/// cmd macros: GNU tools by their full path (only those found), plus the
/// Linux names for cmd's own commands.
pub fn doskey_macros(gnu: Option<&Path>) -> String {
    let mut out = String::new();
    // Linux name -> a cmd command
    for (name, line) in [
        ("which", "where $*"),
        ("pwd", "cd"),
        ("clear", "cls"),
        ("ifconfig", "ipconfig $*"),
        ("ps", "tasklist $*"),
        ("env", "set"),
        ("printenv", "set $*"),
        ("export", "set $*"),
        ("history", "doskey /history"),
        ("open", "start \"\" $*"),
        ("xdg-open", "start \"\" $*"),
        ("uname", "ver"),
        ("killall", "taskkill /F /IM $1.exe"),
        ("pkill", "taskkill /F /IM $1.exe"),
    ] {
        out.push_str(&format!("{name}={line}\r\n"));
    }
    if let Some(g) = gnu {
        let tool = |t: &str| g.join(format!("{t}.exe"));
        // name, tool, fixed options
        let gnu_cmds: &[(&str, &str, &str)] = &[
            ("ls", "ls", "--color=auto"),
            ("ll", "ls", "-lh --color=auto"),
            ("la", "ls", "-lAh --color=auto"),
            ("grep", "grep", "--color=auto"),
            ("egrep", "grep", "-E --color=auto"),
            ("fgrep", "grep", "-F --color=auto"),
            ("cat", "cat", ""),
            ("head", "head", ""),
            ("tail", "tail", ""),
            ("less", "less", "-R"),
            ("wc", "wc", ""),
            ("sed", "sed", ""),
            ("awk", "awk", ""),
            ("cut", "cut", ""),
            ("tr", "tr", ""),
            ("uniq", "uniq", ""),
            ("xargs", "xargs", ""),
            ("diff", "diff", "--color=auto"),
            ("touch", "touch", ""),
            ("rm", "rm", ""),
            ("cp", "cp", ""),
            ("mv", "mv", ""),
            ("mkdir", "mkdir", ""),
            ("du", "du", ""),
            ("df", "df", ""),
            ("file", "file", ""),
            ("find", "find", ""),
            ("sort", "sort", ""),
            ("tee", "tee", ""),
            ("basename", "basename", ""),
            ("dirname", "dirname", ""),
            ("realpath", "realpath", ""),
            ("md5sum", "md5sum", ""),
            ("sha256sum", "sha256sum", ""),
            ("base64", "base64", ""),
            ("nano", "nano", ""),
            ("vim", "vim", ""),
            ("vi", "vim", ""),
        ];
        for (name, t, opts) in gnu_cmds {
            let exe = tool(t);
            if !exe.is_file() {
                continue;
            }
            let sep = if opts.is_empty() { "" } else { " " };
            out.push_str(&format!("{name}=\"{}\"{sep}{opts} $*\r\n", exe.display()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmd_gets_macros_only_when_plain() {
        let gnu = std::env::temp_dir().join("vibex-gnu-test");
        let _ = std::fs::create_dir_all(&gnu);
        for t in ["ls", "grep"] {
            let _ = std::fs::write(gnu.join(format!("{t}.exe")), b"");
        }
        let m = doskey_macros(Some(&gnu));
        assert!(m.contains("which=where $*\r\n"));
        assert!(m.contains(&format!("ll=\"{}\" -lh --color=auto $*", gnu.join("ls.exe").display())));
        // only tools that exist
        assert!(!m.contains("sed="));
        assert!(doskey_macros(None).lines().all(|l| !l.starts_with("grep=")));

        let mut own = Command::new("cmd.exe");
        own.args = vec!["/c".into(), "build.bat".into()];
        prepare(&mut own, true);
        assert_eq!(own.args, vec!["/c", "build.bat"]);
        let mut off = Command::new("cmd.exe");
        prepare(&mut off, false);
        assert!(off.args.is_empty());
        assert!(off.env.contains(&("VIBEX_COMMANDS".into(), "0".into())));
    }

    #[test]
    fn wslenv_forwards_the_switch() {
        let mut c = Command::new("wsl.exe");
        c.env.push(("WSLENV".into(), "TERM_PROGRAM:VIBEX_EXE/p".into()));
        prepare(&mut c, true);
        let w = &c.env.iter().find(|(k, _)| k == "WSLENV").unwrap().1;
        assert_eq!(w, "TERM_PROGRAM:VIBEX_EXE/p:VIBEX_COMMANDS:VIBEX_PROMPT");
    }
}
