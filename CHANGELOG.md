# Changelog

## Unreleased

- **53 themes:** 49 popular schemes (Catppuccin, Dracula, Monokai, Nord, Gruvbox, One Dark, Tomorrow Night, Cobalt2, Synthwave, Zenburn, Campbell, Rosé Pine, Kanagawa, Everforest, GitHub, Solarized, Tokyo Night and more), with proper names and colour swatches in the theme picker, grouped into Vibex, dark and light.
- **Light themes:** the tab bar, Settings, menus and dialogs now read properly on light backgrounds (darker text, deeper accents, readable text on coloured tabs).
- **Settings:** long paths keep their file name; the theme list opens at the current theme.
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
