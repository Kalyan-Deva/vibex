# Installing Vibex

## Build the installer

```powershell
powershell -ExecutionPolicy Bypass -File tools\package.ps1
```

This builds the release `vibex.exe` and a single-file installer around it:
`dist\Vibex-Setup-<version>.exe`. Run it again later to update in place
(close Vibex first).

## What setup does (per user, no admin)

| | |
|---|---|
| Files | `%LOCALAPPDATA%\Programs\Vibex\`: `vibex.exe`, `vibex.com` (the console-side `vibex` command), `OpenConsole.exe`, `vibex-setup.exe` (the uninstaller), `shell\` (the integration scripts), `tools\` (the bundled nano and Neovim) |
| Start menu | **Vibex** shortcut with the icon |
| Explorer | **Open in Vibex** on folders, drives and folder backgrounds; **Run in Vibex** on `.ps1`, `.bat`, `.cmd` (on Windows 11 these are under *Show more options*) |
| Run / PATH | `vibex` works in Win+R (App Paths) and in any terminal (user PATH) |
| Apps & features | **Vibex Terminal** entry with uninstall |

`vibex-setup.exe --quiet` installs without dialogs; `--uninstall [--quiet]`
removes everything above. Your config in `%USERPROFILE%\.config\vibex` is
never touched.

## Default terminal

Windows' *Default terminal application* setting only lists MSIX-packaged
terminals that implement the `ITerminalHandoff` COM interface through a
registered proxy DLL, so Vibex can't be selected there yet. What covers the
same ground today:

- `vibex -- <program> [args]` runs any program in a Vibex tab (it stays open
  after the program exits; Enter closes it),
- **Run in Vibex** on scripts, **Open in Vibex** on folders, Win+R `vibex`.

## Command line

```
vibex                      start (reopens the last session)
vibex --profile wsl        start with a profile
vibex --cwd D:\code        start in a folder
vibex -- python app.py     run a program in a tab
vibex --no-restore         don't reopen the last session
vibex --check-config       validate vibex.lua
vibex --preview out.bmp    render a sample window, no GUI (--palette / --quick)
vibex --version
```

## Linux and macOS

The same code builds on Linux and macOS: the pseudoterminal is a `posix_openpt` pty (the child gets its own session and controlling terminal, so job control and Ctrl+C work, and resizes send SIGWINCH). Defaults there: a login shell from `$SHELL` plus `sh`, fonts from `/usr/share/fonts`, `~/.local/share/fonts` or `/Library/Fonts` (DejaVu Sans Mono, Liberation Mono, Noto Sans Mono, Ubuntu Mono or Menlo), links opened with `xdg-open` / `open`, notifications through `notify-send`.

    cargo build --release -p vibex
    ./target/release/vibex

Status: Linux is built and tested (unit tests, a real pty, headless rendering) in WSL Ubuntu 24.04. macOS compiles but hasn't been run (that needs a Mac). Not yet on Unix: the single-instance handoff (each launch is its own process) and an installer.

## Updates

Vibex can update itself from a signed feed. Set where the feed lives in vibex.lua:

    config.update_feed = "https://example.com/vibex/update.txt"
    config.check_updates = true   -- once a day (default)

Palette: **Check for updates**, **Install update and restart**. The installer is downloaded, checked against an Ed25519 signature made with the release key (the version is signed too, so an old installer can't be passed off as new), then it waits for Vibex to close, installs quietly and reopens it. Nothing is downloaded until `update_feed` is set.

## Releasing

1. One time: `cargo run -p vibex-release -- keygen` makes `%USERPROFILE%\.vibex\update-signing.key` (never commit it; back it up) and prints the public key, which goes in `crates/vibex/src/update.rs`.
2. Build, optionally sign, and write the feed:

       powershell -ExecutionPolicy Bypass -File tools\package.ps1 -FeedUrl https://example.com/vibex/Vibex-Setup-0.2.0.exe [-SignThumbprint <cert SHA1>]

3. Upload `dist\Vibex-Setup-<v>.exe` to that URL and `dist\update.txt` to `config.update_feed`.
4. winget: `tools\winget.ps1 -Url <installer URL>` writes manifests to `dist\winget\` for a PR to microsoft/winget-pkgs.

Code signing needs a code-signing certificate (an OV/EV certificate from a CA, or Azure Trusted Signing); `tools\sign.ps1` uses it through signtool. Unsigned installers work but SmartScreen warns on first run.
