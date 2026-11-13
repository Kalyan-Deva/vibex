# Changelog

## Unreleased

- **The Kali Linux look:**
  - Dragon Dark, Kali's QTerminal colours
  - the Dragon prompt, Kali's two-line `┌──(user㉿host)-[~]` prompt for PowerShell and bash, with Kali's `ls`
  - a classic title bar: a centred title, round window buttons, a Session · Actions · Edit · View · Help menu bar, and tabs only when there's more than one
  - all of it with one click: Settings → Appearance → **Style** (*Vibex* or *Dragon (Kali Linux)*), the palette, or `vibex cli style`
- **Settings:** the prompt, title-bar and weekly editor-update choices were forgotten after a restart; they're saved properly now (with a test that keeps the saved and read lists in sync)

- **54 themes:** 50 popular schemes (Catppuccin, Dracula, Monokai, Nord, Gruvbox, One Dark, Tomorrow Night, Cobalt2, Synthwave, Zenburn, Campbell, Rosé Pine, Kanagawa, Everforest, GitHub, Solarized, Tokyo Night and more), with proper names and colour swatches in the theme picker, grouped into Vibex, dark and light.
- **Light themes:** the tab bar, Settings, menus and dialogs now read properly on light backgrounds (darker text, deeper accents, readable text on coloured tabs).
- **Settings:** long paths keep their file name; the theme list opens at the current theme.
- **Config:** `vibex.lua` and `settings.lua` saved with a byte-order mark (Notepad does this) load fine instead of failing.
- **Images:** after `vshow` drew an image, the prompt came back inside the picture and Vibex lost track of what was typed, so suggestions and the risky-command check could miss. `vibex show` now makes the image's rows first, so the cursor always lands below it. An image taller than the window is shrunk to fit.
- **Remote control:** `vibex cli send-text -- TEXT` types TEXT as it is, even when it starts with `--`.
- **Palette:** fuzzy search prefers close-together matches, so `set lig` finds ligatures rather than a theme.

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
