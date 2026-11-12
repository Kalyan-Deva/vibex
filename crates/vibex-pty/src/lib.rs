//! Pseudoterminal backend.
//!
//! Windows: ConPTY (`CreatePseudoConsole`, or a bundled OpenConsole).
//! Unix (Linux, macOS): a `posix_openpt` pseudoterminal.

use std::io;
use std::sync::mpsc::Sender;

#[cfg(windows)]
mod conpty;
#[cfg(windows)]
mod openconsole;
#[cfg(windows)]
pub use conpty::Pty;
#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::Pty;

/// What to run inside the terminal.
#[derive(Clone, Debug)]
pub struct Command {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    /// Extra environment variables (added to the inherited environment).
    pub env: Vec<(String, String)>,
}

impl Command {
    pub fn new(program: impl Into<String>) -> Self {
        Self { program: program.into(), args: Vec::new(), cwd: None, env: Vec::new() }
    }

    pub fn arg(mut self, a: impl Into<String>) -> Self {
        self.args.push(a.into());
        self
    }

    pub fn env(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.env.push((k.into(), v.into()));
        self
    }
}

/// Events from the PTY reader threads.
#[derive(Debug)]
pub enum PtyEvent {
    Output(Vec<u8>),
    /// The child process exited (with its exit code).
    Exited(u32),
}

/// Something that can be woken when PTY output arrives (the UI event loop).
pub trait Waker: Send + 'static {
    fn wake(&self);
}

impl<F: Fn() + Send + 'static> Waker for F {
    fn wake(&self) {
        self()
    }
}

pub(crate) fn send(tx: &Sender<PtyEvent>, ev: PtyEvent, waker: &dyn Waker) -> bool {
    if tx.send(ev).is_err() {
        return false;
    }
    waker.wake();
    true
}

pub type Result<T> = io::Result<T>;
