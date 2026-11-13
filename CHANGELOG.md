# Changelog

## Unreleased

- **Linux beta:** a per-user tarball install (`tools/package.sh` builds it, `install.sh` installs into `~/.local` with an app-menu entry and icons, `--uninstall` removes it), the single-instance hand-off over a Unix socket, and `vibex quake` to show or hide the dropdown from a desktop shortcut.
- Launches during startup now wait for the starting Vibex instead of opening a second one.
- CI builds and tests the whole workspace on Linux.

## 0.1.0: first public release

Everything in the [README](README.md#features). The headline features:

- **Engine:** a from-scratch VT parser, screen model with reflow, ConPTY / OpenConsole layer, and a wgpu renderer with a CPU fallback, ligatures, colour emoji and grapheme clusters.
- **Windows:** tabs (drag between windows), splits, floating panes, a Quake dropdown, workspaces, layouts, detach and attach, and session restore with scrollback.
- **Commands:** command blocks, history search across sessions, suggestions and completions, the command palette, quick select, copy mode, broadcast input and an SSH host picker.
- **Both worlds:** Linux commands in PowerShell and cmd, Windows commands in WSL, and nano plus Neovim bundled with weekly, SHA-256-checked updates.
- **Capture:** screenshots (pane, command, selection), saving lines as PNG, text, ANSI, HTML, SVG or Markdown, asciicast recording, GIF export, and `vshow` for images and Markdown.
- **Security:** the secret guard, risky-command confirmation, a hash-chained audit log, and engagement mode (scope guard, loot, report).
- **Configuration:** Lua with hot reload, events, triggers and a status bar, plus a Settings window and `vibex cli` remote control.
- **Install:** a per-user installer with Explorer integration, signed updates and an uninstaller.
