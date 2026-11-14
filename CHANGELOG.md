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
- **Images, polished:**
  - `vshow` returns the prompt straight away (it waited up to a second before, with only a bare cursor under the picture)
  - JPEG photos as well as PNG
  - big pictures are sent at the size they're shown, so a 4K photo is quick
  - with no size given, a picture fills whole rows: no nearly empty row before the prompt
  - after `cls` or `clear` the prompt is tracked again (the risky-command check and suggestions were blind until the next command)
- **Status bar:** a long folder path is shortened from the front (`…\folder\end`) instead of running under the right-hand side.
- **Images:** after `vshow` drew an image, the prompt came back inside the picture and Vibex lost track of what was typed, so suggestions and the risky-command check could miss. `vibex show` now makes the image's rows first, so the cursor always lands below it. An image taller than the window is shrunk to fit.
- **Remote control:** `vibex cli send-text -- TEXT` types TEXT as it is, even when it starts with `--`.
- **Palette:** fuzzy search prefers close-together matches, so `set lig` finds ligatures rather than a theme.

### Stability (a full audit, plus a fuzz test of hostile output and resizes)

- **Crashes fixed** (a program's output, or hovering it, could close every window):
  - an OSC 633;E command line with `\x` before an accented letter
  - Ctrl+hover over text like `@(é(1)`
  - a cursor-position report with the cursor above the scroll region in origin mode (also a garbage reply before)
  - searching from the middle of a long wrapped line (secret masking and export do this)
- **No more freezes or memory blowups from hostile output:**
  - a flood of combining marks: clusters are capped
  - Ctrl+hover over a URL followed by thousands of dots: a single pass now
  - images: a byte budget and a count limit for stored and placed images, oldest first
  - an endless sixel parameter list
- **Screen glitches fixed:**
  - shrinking the window while a full-screen app is open no longer throws away shell lines, and the prompt comes back in the right place
  - after a width change, the next character no longer overwrites the last one on a full line
  - command text (blocks, history, "long command finished") no longer goes blank once the scrollback is full
  - `reset` keeps the cell size (images and size reports were wrong until the next resize) and the line numbering
  - accents no longer attach to a cell that scrolled away
  - inserting, deleting or erasing next to a double-width character no longer leaves half of it behind
  - CAN / SUB cancel an OSC or APC string instead of acting on the half that arrived
  - restored history stays off screen after a re-wrap that trims the scrollback
  - resizing the window while typing no longer loses track of the prompt (suggestions and the risky-command check kept working only until the next width change)
- **Memory:** every new window, font change and screenshot read the fallback fonts again and kept them, about 90 MB per window, never given back. Each font file is now read once: four windows opened and closed went from +370 MB to +9 MB.
- **A flood can't freeze the window any more:** `yes`-style output is taken in slices, with input and redraws in between, so Ctrl+C gets through and other windows keep drawing.
- **App crashes fixed:**
  - dragging a tab while another tab closes
  - a Lua trigger or event handler closing a pane or the last tab
  - Ctrl+Right on a suggestion containing a non-breaking space, and an SSH config line with one
  - an empty menu or font list
- **Lua can't hang Vibex:** a callback, a trigger, the status bar or `vibex.lua` itself that runs longer than 250 ms is stopped, with a message saying so.
- **The right tab, always:** "Close this tab?", the tab menu and renaming remember the tab itself, not its position, so a tab closing meanwhile can't make them act on another one. "Close other tabs" and "Close tabs to the right" ask first when something is still running.
- **Remote control:** `--pane N` for a pane that no longer exists is an error instead of acting on the focused pane; `move-tab` into its own window is a no-op instead of closing it; `vibex cli … | Select-Object -First 1` no longer crashes when the pipe closes early; a client that connects and sends nothing no longer blocks every later `vibex` launch.
- **Audit log:** a log over 256 KB with non-ASCII text no longer restarts its hash chain (which made `audit verify` report tampering).
- **IME:** text typed with an input method while a "Run this?" question is open no longer goes into the shell behind it.
- **OpenConsole:** a console host that failed half-way through starting is shut down instead of left running.
- **Crash log:** if Vibex ever does crash, the reason and a backtrace go to `crash.log` in the config folder.
- **Ctrl+C:** after stopping a command with Ctrl+C, Vibex thought it was still running, so the risky-command check let the next Enter straight through and the prompt wasn't tracked. A new prompt now always ends the last command, and the PowerShell script reports the stop (exit 130).
- **The risky-command check has a second source:** besides the screen, it checks the line as Vibex itself sent it since the last Enter (typing and pastes). Right after a big output, the console host can paint its last lines over the prompt once the shell is already waiting there; the screen then can't be read and the check could miss (4 of 29 tries in a soak test; 0 of 12 in the same test afterwards).
- **Pasting can't freeze the window:** input goes to each pane through a writer of its own, so a huge paste, or a console host that stops reading, holds up only that pane.
- **Popups keep your floating shell:** a popup (or a file opened in one) no longer replaces a floating pane that's busy; it opens in a tab instead.
- **Lua coroutines** are under the 250 ms limit too.
- **Links:** the shared link table keeps each URL once and has a size cap.
- **Graphics:** a lost GPU (driver reset, sleep, GPU switch) switches the window to the software renderer instead of freezing it; the glyph, shaping and image caches, the glyph atlas and the GPU buffers are all bounded; odd window and image sizes can't crash the renderer; wide characters are selected, highlighted and covered by the cursor as a whole; colour emoji in a single-width cell stay in it; changing the font family no longer shows the old font's glyphs.
- **A window started from a shell** (`vibex --cwd .`) no longer stays tied to that shell's console (closing the shell could close it).
- **Updates:** stricter feed checks (canonical hex, strict signatures, safe version strings, prereleases sort before releases), TLS 1.2+ only, an overall time limit on downloads, and old downloaded installers are cleaned up.
- **Installer:** files are staged and swapped in, with rollback if anything fails; the uninstaller removes only its own files; PATH is edited without ever losing other entries; shortcut and Explorer paths with quotes or `&` work and can't run code; DLLs load from System32 only; release builds no longer contain build-machine paths.
- **`vibex.com`:** Ctrl+C waits for `vibex.exe` to finish; a missing `vibex.exe` gets a clear message.

### Shell scripts (PowerShell and bash)

- **`rm -rf` is safe:** `rm -rf /`, `C:\`, `~`, a drive root or a wildcard at one is refused, as GNU `rm` refuses `/` (`--no-preserve-root` overrides); `rd /s /q` and `del /s /q` on a drive root too, in PowerShell and in bash.
- **`kill -WhatIf`** no longer kills the process.
- **Pipelines work:** `ls *.tmp | rm`, `Get-Process x | kill`, and `rm`, `cat`, `cp`, `mv`, `ps`, `dir`, `del`, `copy`, `move` take pipeline input; `where` and `tee` stream.
- **The `&&` / `||` rewriter** uses PowerShell's own tokenizer: lines with nothing to rewrite come back exactly as typed, comments and strings are left alone, and `$PATH = 1` no longer overwrites the real PATH.
- **Exit marks are right:** a custom prompt (oh-my-posh, starship) sees the real `$?`; an old exit code isn't reported for a later cmdlet error; failed emulated commands show red.
- **Loads in half the time** (about 90 ms), works under `Set-StrictMode` and without Git, and leaves no stray variables behind.
- **Paths:** non-ASCII command lines and folders with `;`, `%` or spaces are reported correctly; Git Bash reports `C:/…` folders Vibex can open.
- **bash:** works with `set -u`, keeps `$?` for other prompt hooks, keeps the marks with prompts that rebuild PS1 (starship), keeps PROMPT_COMMAND in order with bash-preexec, and Windows switches (`ipconfig /all`) survive Git Bash's path conversion.

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
