//! Unix backend (Linux, macOS): a pseudoterminal pair from `posix_openpt`.
//!
//! 1. Open the master, unlock it and open its slave.
//! 2. Start the child with the slave as stdin/stdout/stderr, in a new
//!    session (`setsid`) with the slave as its controlling terminal
//!    (`TIOCSCTTY`), so job control and Ctrl+C work.
//! 3. A reader thread drains the master; a waiter thread reports the exit.
//! 4. Resizing sets the master's window size; the kernel sends SIGWINCH.

use crate::{send, Command, PtyEvent, Result, Waker};
use std::ffi::CStr;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::Stdio;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;

pub struct Pty {
    master: File,
    pid: libc::pid_t,
}

fn last_err() -> io::Error {
    io::Error::last_os_error()
}

fn winsize(cols: u16, rows: u16) -> libc::winsize {
    libc::winsize { ws_row: rows, ws_col: cols, ws_xpixel: 0, ws_ypixel: 0 }
}

fn open_pair(cols: u16, rows: u16) -> Result<(OwnedFd, OwnedFd)> {
    unsafe {
        let m = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC);
        if m < 0 {
            return Err(last_err());
        }
        let master = OwnedFd::from_raw_fd(m);
        if libc::grantpt(m) != 0 || libc::unlockpt(m) != 0 {
            return Err(last_err());
        }
        // ptsname isn't thread-safe; the name is copied straight away
        static LOCK: Mutex<()> = Mutex::new(());
        let path = {
            let _g = LOCK.lock().unwrap();
            let p = libc::ptsname(m);
            if p.is_null() {
                return Err(last_err());
            }
            CStr::from_ptr(p).to_owned()
        };
        let s = libc::open(path.as_ptr(), libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC);
        if s < 0 {
            return Err(last_err());
        }
        let slave = OwnedFd::from_raw_fd(s);
        let ws = winsize(cols, rows);
        libc::ioctl(m, libc::TIOCSWINSZ, &ws);
        Ok((master, slave))
    }
}

impl Pty {
    pub fn spawn(cmd: &Command, cols: u16, rows: u16, events: Sender<PtyEvent>, waker: impl Waker) -> Result<Pty> {
        let (master, slave) = open_pair(cols, rows)?;

        let mut c = std::process::Command::new(&cmd.program);
        c.args(&cmd.args);
        c.env("TERM", "xterm-256color").env("COLORTERM", "truecolor").env("TERM_PROGRAM", "Vibex");
        for (k, v) in &cmd.env {
            c.env(k, v);
        }
        if let Some(dir) = &cmd.cwd {
            c.current_dir(dir);
        }
        c.stdin(Stdio::from(slave.try_clone()?));
        c.stdout(Stdio::from(slave.try_clone()?));
        c.stderr(Stdio::from(slave));
        unsafe {
            c.pre_exec(|| {
                // new session, then make the pty (now fd 0) its terminal
                if libc::setsid() < 0 {
                    return Err(io::Error::last_os_error());
                }
                if libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
                // undo signal dispositions a GUI parent may have set
                for sig in [libc::SIGINT, libc::SIGQUIT, libc::SIGTSTP, libc::SIGTTIN, libc::SIGTTOU, libc::SIGCHLD, libc::SIGPIPE] {
                    libc::signal(sig, libc::SIG_DFL);
                }
                Ok(())
            });
        }
        let mut child = c.spawn().map_err(|e| io::Error::new(e.kind(), format!("failed to start {}: {e}", cmd.program)))?;
        let pid = child.id() as libc::pid_t;
        drop(c); // closes our copies of the slave

        let master = File::from(master);
        let waker: Arc<dyn Waker + Sync> = Arc::new(SyncWaker(Mutex::new(Box::new(waker))));

        {
            let mut reader = master.try_clone()?;
            let tx = events.clone();
            let waker = waker.clone();
            thread::Builder::new().name("vibex-pty-reader".into()).spawn(move || {
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            if !send(&tx, PtyEvent::Output(buf[..n].to_vec()), &*waker) {
                                break;
                            }
                        }
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        // EIO once the last slave fd closes (the child exited)
                        Err(_) => break,
                    }
                }
            })?;
        }
        {
            let tx = events;
            thread::Builder::new().name("vibex-pty-wait".into()).spawn(move || {
                let code = match child.wait() {
                    Ok(st) => st.code().map_or_else(
                        || {
                            use std::os::unix::process::ExitStatusExt;
                            128 + st.signal().unwrap_or(0) as u32
                        },
                        |c| c as u32,
                    ),
                    Err(_) => 1,
                };
                send(&tx, PtyEvent::Exited(code), &*waker);
            })?;
        }
        Ok(Pty { master, pid })
    }

    pub fn write(&self, data: &[u8]) -> Result<()> {
        (&self.master).write_all(data)
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        let ws = winsize(cols, rows);
        if unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ, &ws) } != 0 {
            return Err(last_err());
        }
        Ok(())
    }

    pub fn kill(&self) {
        // hang up the session like closing a terminal window would
        unsafe {
            libc::kill(-self.pid, libc::SIGHUP);
            libc::kill(self.pid, libc::SIGHUP);
        }
    }

    pub fn host(&self) -> &'static str {
        "pty"
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        self.kill();
    }
}

struct SyncWaker(Mutex<Box<dyn Waker>>);

impl Waker for SyncWaker {
    fn wake(&self) {
        if let Ok(w) = self.0.lock() {
            w.wake();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    fn run(cmd: Command, cols: u16, rows: u16, resize: Option<(u16, u16)>) -> (String, Option<u32>) {
        let (tx, rx) = mpsc::channel();
        let pty = Pty::spawn(&cmd, cols, rows, tx, || {}).expect("spawn");
        if let Some((c, r)) = resize {
            std::thread::sleep(Duration::from_millis(200));
            pty.resize(c, r).unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut out = Vec::new();
        let mut code = None;
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(PtyEvent::Output(b)) => out.extend(b),
                Ok(PtyEvent::Exited(c)) => code = Some(c),
                Err(_) if code.is_some() => break,
                Err(_) => {}
            }
        }
        (String::from_utf8_lossy(&out).into_owned(), code)
    }

    #[test]
    fn runs_a_shell_on_a_tty() {
        let (out, code) = run(Command::new("/bin/sh").arg("-c").arg("tty -s && echo is-a-tty; echo $TERM; exit 3"), 80, 24, None);
        assert!(out.contains("is-a-tty"), "{out:?}");
        assert!(out.contains("xterm-256color"), "{out:?}");
        assert_eq!(code, Some(3));
    }

    #[test]
    fn resize_reaches_child() {
        let (out, _) = run(Command::new("/bin/sh").arg("-c").arg("sleep 0.6; stty size"), 80, 24, Some((101, 33)));
        assert!(out.contains("33 101"), "{out:?}");
    }
}
