# Contributing to Vibex

Thanks for helping. Bug reports, ideas and pull requests are all welcome.

## Reporting a bug

[Open an issue](https://github.com/Kalyan-Deva/vibex/issues/new/choose) and include:

- the Vibex version (`vibex --version`) and your Windows version
- the shell you were using (PowerShell 5.1 or 7, cmd, WSL and which distro, ssh)
- what you did, what you expected, and what happened instead
- your `vibex.lua` if it matters (`vibex --check-config` validates it)
- for drawing glitches, a screenshot: <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>S</kbd> saves one to `Pictures\Vibex`

Security problems go privately, as described in [SECURITY.md](SECURITY.md), not in an issue.

## Building

You need [Rust](https://rustup.rs) (stable) and Windows 10 or 11. There are no other build-time requirements.

```powershell
cargo build -p vibex                  # debug build: target\debug\vibex.exe
cargo build --release -p vibex        # what you'd ship
cargo test --workspace                # every crate's tests
cargo clippy --workspace              # lints
```

A few useful ways to see your change without clicking around:

```powershell
vibex --check-config                  # validate vibex.lua
vibex --preview out.bmp               # render a sample window, no GUI
$env:VIBEX_PREVIEW_SETTINGS="appearance"; vibex --preview out.bmp   # a Settings page
python tools\imgtest.py               # sixel + kitty images in a running Vibex
cargo run -p vibex-render --example vtdrive    # drive vttest-style sequences
```

To try a change without touching your own Vibex, run a second instance with its own config folder:

```powershell
$env:VIBEX_INSTANCE = "dev"; $env:VIBEX_CONFIG_DIR = "$env:TEMP\vibex-dev"
target\debug\vibex.exe
```

The installer: `tools\package.ps1` builds `dist\Vibex-Setup-<version>.exe`. The first time, it downloads the bundled nano and Neovim through `tools\fetch-bundle.ps1`, checking each file against the SHA-256 in `tools\bundle.lock`.

## Code

- Start with [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for how the crates fit together.
- Run `cargo fmt` before committing. Keep `cargo clippy` quiet for the code you touched.
- Add a test next to the code. Terminal behaviour goes in `crates/vibex-term/src/tests.rs`, parsing in `vibex-vt`, and features in the module's own `#[cfg(test)]` block.
- Write comments and messages in plain words, and say *why* when it isn't obvious.
- New options need four things: a field in `config.rs`, parsing in `lua.rs`, a row in Settings (`settings.rs` and `settings_ui.rs`) when it's an everyday option, and a line in `docs/CONFIG.md`.
- New actions go in `keys.rs` (name, label, parser), and in `palette.rs` if they belong in the palette.

## Pull requests

- Keep each pull request to one change, and describe what it fixes or adds and how you tested it.
- Make sure `cargo test --workspace` passes. CI runs it on Windows, and runs the portable crates on Linux.
- Screenshots help for anything visual.

By contributing, you agree that your contribution is dual-licensed under MIT OR Apache-2.0, like the rest of Vibex (see [README.md](README.md#license)).
