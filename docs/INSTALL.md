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

## Linux

### Install from the tarball

Download `vibex-<version>-linux-x86_64.tar.gz` (and its `.sha256`) from [Releases](https://github.com/Kalyan-Deva/vibex/releases/latest), then:

```bash
sha256sum -c vibex-<version>-linux-x86_64.tar.gz.sha256
tar xzf vibex-<version>-linux-x86_64.tar.gz
cd vibex-<version>-linux-x86_64
./install.sh
```

`install.sh` installs per user into `~/.local`, with no root needed. Run it again with a newer tarball to update; a running Vibex keeps working until you restart it.

| | |
|---|---|
| Files | `~/.local/bin/vibex`; `~/.local/share/vibex/shell/vibex.bash` (shell integration); docs and licences in `~/.local/share/doc/vibex/` |
| App menu | **Vibex** under System / Terminal, with **New Window** and **Quake Dropdown** actions on its right-click menu |
| Icons | `~/.local/share/icons/hicolor/<size>/apps/vibex.png`, 16 to 512 px |

Options:

- `./install.sh --prefix /usr/local` installs system-wide (run it with `sudo`).
- `./install.sh --uninstall` removes exactly the files the install put there (add `--prefix` if you used one). Your config in `~/.config/vibex` is never touched.

If `~/.local/bin` isn't on your `PATH`, the installer says so. The menu entry works either way, because it uses the full path.

### Quake dropdown

Linux has no app-level global hotkey, so bind the command `vibex quake` to a key in your desktop's keyboard-shortcut settings (GNOME: Settings → Keyboard → Custom Shortcuts; KDE: System Settings → Shortcuts → Custom Shortcuts; Xfce: Keyboard → Application Shortcuts). It shows or hides the dropdown, and starts Vibex if it isn't running.

### Build the tarball

```bash
tools/package.sh
```

This builds the release binary and writes `dist/vibex-<version>-linux-<arch>.tar.gz` plus a `.sha256`. No system libraries are needed to build: the binary links only libc and loads X11 / Wayland at run time.

### How it runs

The pseudoterminal is a `posix_openpt` pty (the child gets its own session and controlling terminal, so job control and Ctrl+C work, and resizes send SIGWINCH). Defaults: a login shell from `$SHELL` plus `sh`, fonts from `/usr/share/fonts` or `~/.local/share/fonts` (DejaVu Sans Mono, Liberation Mono, Noto Sans Mono or Ubuntu Mono), links opened with `xdg-open`, notifications through `notify-send`. Launching `vibex` while it's running opens a new window in the running process (through a socket in `$XDG_RUNTIME_DIR`), the same as on Windows.

Status: tested on X11 (Kali). Wayland, GNOME and KDE haven't been tried yet. Updating from inside Vibex is Windows-only for now; on Linux, install the new tarball.

## macOS

The same code compiles on macOS (`cargo build --release -p vibex`), with fonts from `/Library/Fonts` (Menlo) and links opened with `open`. It hasn't been run yet (that needs a Mac), and there's no app bundle.

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
