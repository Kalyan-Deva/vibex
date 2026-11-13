# Configuring Vibex

Vibex reads `%USERPROFILE%\.config\vibex\vibex.lua` (or `--config PATH`). It is
plain Lua 5.4 that **returns a table**. Save the file and Vibex reloads it
immediately. If it has a mistake, the old config stays active and the error is
shown in the window until you fix it.

```
vibex --check-config          # validate without opening a window
vibex --preview out.bmp       # render a sample screen with your config
vibex --profile wsl           # start with a profile
```

Unknown options and unknown fields are errors, so typos are caught
(`config.scrolback` → *unknown option config.scrolback*).

## Options

| Option | Type | Default |
|---|---|---|
| `font` | family name, or `{ family, size, ligatures, regular, bold, italic, bold_italic, fallbacks }` | PlemolJP Console NF if installed, else Cascadia Code (in `~/.config/vibex/fonts`), else Consolas |
| `font_size` | points | `11` |
| `ligatures` | bool | `true` |
| `theme` | a built-in theme by id or name (`"catppuccin-mocha"` or `"Catppuccin Mocha"`; the list is below), or a table (below) | `"tokyo-night"` |
| `window` | `{ width, height, padding, opacity, renderer }` | `1100×680`, padding `{x=10,y=8}`, opacity `0.4` (acrylic, same darkness as Windows Terminal's 40%), `"auto"` |
| `opacity` / `padding` | shortcuts for the `window` fields | |
| `title_bar` | `"tabs"`: tabs in the title bar, like Windows Terminal. `"classic"`: a centred title, round window buttons and a menu bar (Session, Actions, Edit, View, Help), with tabs below only when there's more than one, like QTerminal on Kali Linux. Also Settings → Appearance | `"tabs"` |
| `prompt` | `"default"`: the shell's own prompt. `"dragon"`: Kali Linux's two-line `┌──(user㉿host)-[~]` / `└─$` prompt in PowerShell and bash (WSL), with Kali's `ls` (columns, colours, sorting). Also Settings → Terminal | `"default"` |
| `right_click` | `"menu"`: right-click opens a menu (link actions, copy / paste / select all / find, splits, tabs, the last command, Settings) with each shortcut shown; Shift+right-click works inside mouse apps like vim. `"paste"`: Windows Terminal's copy-if-selected-else-paste | `"menu"` |
| `glass` | glass look: the desktop shows through blurred and in colour (Windows' acrylic blur-behind, no grey layer), smoky title and status bars, a faint sheen and light edge; stays see-through when the window isn't focused. Also Settings → Appearance → Glass | `false` |
| `cursor` | `{ style = "block"\|"bar"\|"underline", blink, blink_ms }` | block, blinking, 530 ms |
| `scrollback` | lines | `10000` |
| `scroll` | `{ wheel_lines, smooth }` | `3`, `true` |
| `status_bar` | bool | `false` |
| `multi_click_ms` | double/triple-click window | `400` |
| `profiles` | `{ name = { cmd, cwd, env } }` | `powershell`, `cmd`, `wsl` |
| `default_profile` | profile name | `"powershell"` |
| `leader` | `{ key, mods, timeout_ms }` | none |
| `keys` | list of `{ key, mods, action }` | see defaults |
| `disable_default_keys` | bool | `false` |
| `window.decorations` | `"vibex"` (own title bar with tabs + window buttons) or `"native"` | `"vibex"` |
| `tab_bar` | `"auto"`, `"always"`, `"never"` (native decorations only; the Vibex title bar is always shown) | `"auto"` |
| `shell_marks` | prompt marks in the left gutter ([SHELL_INTEGRATION.md](SHELL_INTEGRATION.md)) | `true` |
| `dim_inactive_panes` | fade panes that don't have focus | `true` |
| `restore_session` | reopen tabs, splits, folders and window size from `~/.config/vibex/session.lua` | `true` |
| `clipboard_read` | let programs read your clipboard with OSC 52 (writing is always allowed; reading is off because any program could read what you last copied) | `false` |
| `notifications` | show OSC 9 / OSC 777 notifications: a toast when focused, a Windows notification + taskbar flash when not | `true` |

**Settings from the palette.** `Ctrl+Shift+P` → *Settings: …* changes a
setting immediately and saves it to `~/.config/vibex/settings.lua`, which is
applied **after** `vibex.lua` (theme, font size, opacity, ligatures, cursor,
status bar, smooth scrolling, prompt marks, dimming, session restore).
*Settings: reset all* deletes it. Vibex never rewrites your `vibex.lua`.

**Fonts.** `family` is looked up as `~/.config/vibex/fonts/<Family>-Regular.ttf`
(spaces removed, so `"Cascadia Code"` finds `CascadiaCode-Regular.ttf`, with its
`-Bold`, `-Italic` and `-BoldItalic` siblings), then as a Windows font
(Consolas, Courier New, Lucida Console). Or give file paths directly with
`regular = "C:/path/Font-Regular.ttf"`. Characters a font lacks fall back to
`fallbacks`, then to Segoe UI Symbol / Emoji / CJK fonts.

**Built-in themes** (Settings → Appearance shows them with their colours): Vibex's own `tokyo-night`, `vibex-night`, `vibex-day`, `vibex-ember`, and `ayu-dark`, `ayu-light`, `ayu-mirage`, `campbell`, `catppuccin-frappe`, `catppuccin-latte`, `catppuccin-macchiato`, `catppuccin-mocha`, `cobalt2`, `dracula`, `dragon-dark`, `everforest-dark`, `everforest-light`, `github-dark`, `github-dark-dimmed`, `github-light`, `gruvbox-dark`, `gruvbox-light`, `gruvbox-material`, `horizon`, `iceberg`, `kanagawa-dragon`, `kanagawa-lotus`, `kanagawa-wave`, `material-ocean`, `monokai`, `moonfly`, `night-owl`, `nightfox`, `nord`, `nord-light`, `one-dark`, `one-half-dark`, `one-half-light`, `one-light`, `oxocarbon`, `palenight`, `poimandres`, `rose-pine`, `rose-pine-dawn`, `rose-pine-moon`, `snazzy`, `solarized-dark`, `solarized-light`, `synthwave`, `tokyo-night-day`, `tokyo-night-moon`, `tokyo-night-storm`, `tomorrow-night`, `zenburn`. A theme's shown name works too: `theme = "Catppuccin Mocha"`.

**Themes.** A table may start from a built-in and override any colour (`"#rrggbb"`):

```lua
config.theme = {
  base = "tokyo-night",
  background = "#0f0f14", foreground = "#e8e6e3",
  cursor = "#ff8a3d", cursor_text = "#0f0f14",
  selection = "#33304a", search_match = "#5c4320",
  ansi    = { "#1d1c24", "#e5534b", "#57ab5a", "#e0a84a", "#539bf5", "#b083f0", "#39c5cf", "#cdd0d6" },
  brights = { "#545d68", "#ff7b72", "#7ee787", "#f6c15b", "#79c0ff", "#d2a8ff", "#56d4dd", "#f0f3f6" },
  bold_is_bright = true,
}
```

**Profiles.** `cmd` is a list (`{ "wsl.exe", "-d", "Ubuntu" }`) or a string split
on spaces (`"ssh me@box"`). `cwd` accepts `~`. `env` adds variables.

**Opacity.** Transparency needs the GPU renderer started with `opacity < 1`. If
Vibex started opaque, a reload to `< 1` asks for a restart.

## Keys

`key` is a single character (`"c"`, `"="`, `"1"`) or a name: `Enter Tab Escape
Backspace Delete Insert Home End PageUp PageDown Up Down Left Right Space F1`–`F12`.
Keys match what you physically pressed, so `Ctrl+Shift+C` is `key = "c", mods =
"CTRL|SHIFT"`.

`mods` combines `CTRL SHIFT ALT SUPER LEADER` with `|`.

**Leader.** With `config.leader = { key = "Space", mods = "CTRL|SHIFT" }`, press
the leader and then, within `timeout_ms`, a key bound with `mods = "LEADER"`.
The status bar shows `LEADER` while it is armed. A key that matches no LEADER
binding just cancels it.

**Actions** (`local act = require("vibex").action`):

| Action | |
|---|---|
| `act.Copy`, `act.Paste` | clipboard |
| `act.Search` | toggle the search bar |
| `act.ReloadConfig` | reload vibex.lua |
| `act.IncreaseFontSize`, `act.DecreaseFontSize`, `act.ResetFontSize` | zoom |
| `act.ScrollPageUp`, `act.ScrollPageDown`, `act.ScrollToTop`, `act.ScrollToBottom` | history |
| `act.ClearScrollback` | drop history |
| `act.ToggleFullscreen` | |
| `act.SendString("text\r")` | type into the shell |
| `act.SpawnWindow("profile")` | new Vibex window (profile optional) |
| `act.NewTab`, `act.NewTabWith("profile")` | new tab (opens in the current folder with shell integration) |
| `act.CloseTab`, `act.NextTab`, `act.PrevTab`, `act.ActivateTab(n)` | tabs (`n` ≥ 9 = last) |
| `act.SplitRight`, `act.SplitDown`, `act.SplitRightWith("p")`, `act.SplitDownWith("p")` | split the focused pane |
| `act.ClosePane`, `act.TogglePaneZoom` | |
| `act.FocusPane("Left"\|"Right"\|"Up"\|"Down")`, `act.ResizePane(dir)` | move between / resize panes |
| `act.CommandPalette` | searchable list of every command |
| `act.PreviousPrompt`, `act.NextPrompt` | jump between shell prompts |
| `act.QuickSelect` | label URLs, paths, hashes, IPs and UUIDs on screen; type a label to copy (capital = copy and type it) |
| `act.PassThrough` | send the key to the shell (unbinds a default) |
| `function(win) ... end` | anything (see `win` below) |

A user binding replaces a default with the same key and mods.

**Defaults:** see [KEYS.md](KEYS.md) for the full list (copy/paste, search,
tabs, splits, palette, prompts, font size, scrolling).

## Events

```lua
vibex.on("update-status", function(win) return { left = ..., right = ... } end)
vibex.on("title-changed", function(win, title) end)
vibex.on("focus-changed", function(win, focused) end)
vibex.on("bell", function(win) end)
vibex.on("startup", function(win) end)
vibex.on("config-reloaded", function(win) end)
```

`update-status` runs every second and after changes. Return a string, or
`{ left = X, right = Y }` where each side is a string or a list of segments
`{ text = "...", fg = "#rrggbb", bg = "#rrggbb", bold = true }`. With no
handler, Vibex shows the profile, the title, `LEADER` and the grid size.

## The `win` object

Reads (about the focused pane): `win:title()` `win:selection()` `win:profile()`
`win:size()` (cols, rows) `win:is_focused()` `win:leader_active()`
`win:font_size()` `win:opacity()` `win:is_alt_screen()` `win:scrollback_lines()`
`win:tab_count()` `win:tab_index()` `win:pane_count()` `win:cwd()`
`win:last_exit_code()` (the last two need shell integration)

Actions: `win:send_text(s)` `win:copy(s)` `win:paste()` `win:toast(msg, secs)`
`win:set_title(s)` `win:spawn_window(profile)` `win:set_font_size(pt)`
`win:set_opacity(x)` `win:set_theme(name_or_table)` `win:scroll(lines)`
`win:search(query)` `win:reload_config()` `win:perform(act.X)`

Actions are applied right after your function returns.

## Environment overrides

`VIBEX_RENDERER=cpu|gpu`, `VIBEX_OPACITY=0.0–1.0` and
`VIBEX_FONT=C:\path\Foo-Regular.ttf` override the file (handy for quick tests).

## Links, clipboard and notifications from programs

- **OSC 8 hyperlinks.** Programs (ls --hyperlink, gcc, delta, ...) can mark text as a link. Hold Ctrl to underline it, Ctrl+click to open. Only `http`, `https` and `mailto` links open; `file:` and custom schemes are refused so a program can't launch things through a link.
- **OSC 52.** `printf '\e]52;c;%s\a' "$(printf hi | base64)"` copies "hi" — this is what makes tmux/neovim copy work over ssh. Reads (`\e]52;c;?\a`) get an empty answer unless `clipboard_read = true`.
- **OSC 9 / 777.** `printf '\e]9;build done\a'` or `printf '\e]777;notify;Tests;all passed\a'`. Turn off with `notifications = false`.

## Inline images

Vibex shows **sixel** images and the **kitty graphics protocol** (PNG, RGB/RGBA, zlib, chunked transfers, placements, delete, `C=1`, z-index — not Unicode placeholders). Try it: `python tools/imgtest.py`. Tools that work: `chafa`, `timg`, `img2sixel`, `viu`, `yazi` previews, matplotlib's sixel backends.

Images need **OpenConsole.exe** next to `vibex.exe` (the installer bundles it). Windows' built-in console host drops image data; OpenConsole, the newer host from Windows Terminal, passes it through. `VIBEX_CONPTY=inbox` forces the built-in host; `VIBEX_OPENCONSOLE=<path>` points at another copy.

Kitty images are only accepted inline (`t=d`): Vibex never reads a file or shared memory a program names.

## Screen readers

Vibex exposes each pane to UI Automation (Narrator, NVDA, JAWS) as a document with a text pattern: screen readers can read by line, word and character, follow the cursor as the caret, and new output is announced (panes are polite live regions). Focusing a pane from the screen reader moves Vibex's focus there. Nothing is built or sent unless a screen reader is actually listening.

## Settings tab

Ctrl+, (or ⌄ next to + → Settings) opens Settings as a tab: Appearance, Terminal, Profiles, Shortcuts, Updates and About. Everything changed there is saved in `~/.config/vibex/settings.lua`, applied after `vibex.lua`, which is never rewritten. Each changed row has a **Reset** that goes back to `vibex.lua`'s value; **Reset all** clears the file.

Keys: ↑ ↓ / Tab move between rows, ← → change a value (with no row selected they switch pages), Enter or Space toggles and presses, Esc goes back to the terminal. On **Shortcuts**, type to search; select a shortcut and press the new keys (Esc cancels, Backspace removes it). A key that another action had moves to the new one, and Vibex says so.

Shortcut overrides are stored as `keys = { ["NewTab"] = { key = "t", mods = "CTRL|SHIFT" }, ["Copy"] = false }` in settings.lua.

## Floating panes and popups

Alt+Shift+F shows or hides a floating pane over the current tab (a shell the first time). Clicking a split behind it sends it away; it keeps running. `vibex.action.Popup("lazygit")` (or `vibex cli popup -- lazygit`) runs a program in the floating pane and closes it when the program exits, like tmux's `display-popup`.

## Command blocks

With shell integration each command and its output is a block (docs/SHELL_INTEGRATION.md):

- **Sticky header:** scroll into a long output and the command that produced it stays pinned at the top with how long it took (or "running …") and a failed exit code. Click it to jump to the prompt.
- **Select a block:** click the left margin beside it. Ctrl+Shift+C then copies it.
- **Copy the last output:** Ctrl+Shift+O. The palette also has *Copy the last command* and *Run the last command again*.

## Notifications

| Option | What | Default |
|---|---|---|
| `notify_long_commands` | notify when a command that ran at least this many seconds finishes while you're in another window or tab (0 = off) | `10` |

Palette → *Notify when this pane goes quiet* arms a pane: after it has printed something and then stays silent for 10 seconds, you get a notification (a build finishing, a log stopping).

## Triggers

Regexes (Rust `regex` syntax) matched against every new line of output:

```lua
config.triggers = {
  { pattern = "\\berror(\\[E\\d+\\])?", action = "highlight", color = "#5c2b36" },
  { pattern = "[Pp]assword:", action = "notify", title = "Password prompt" },
  { pattern = "Finished `release`", action = function(win, text) win:toast("done: " .. text) end },
}
```

`highlight` colours the match (it stays with the line in history); `notify` and `toast` show the matched text; a function gets `(win, text)`. Each trigger fires at most once every 3 seconds.

## Opening file:line

Hold Ctrl over `src/main.rs:12:5`, `--> crates/x.rs:3`, `foo.cpp(12,5)` or `C:\x\y.py:3` to underline it; Ctrl+click opens it. Relative paths resolve against the pane's folder (shell integration reports it).

```lua
config.editor = {
  cmd = { "nvim", "+{line}", "{file}" },   -- {file} {line} {col}
  env = { NVIM_APPNAME = "nvim-dev" },
  open_in = "popup",                       -- popup | tab | split | app
}
-- or: config.editor = "code -g {file}:{line}:{col}"   (open_in = "popup" by default)
```

Without `editor`, VS Code (`code -g`) is used when it's installed, else Windows opens the file with its default app. In a WSL folder the editor runs through `wsl.exe`.

## Restored scrollback

| Option | What | Default |
|---|---|---|
| `restore_scrollback` | on exit, save each pane's last 2000 lines (with colours) to `~/.config/vibex/buffers/`, and show them again under a "restored" line next time | `true` |

## Workspaces, layouts and detaching

Every window is a workspace ("main", "workspace-2", or a layout's name). The palette lists the others (*Workspace: name*) to switch to.

- **Detach:** Ctrl+Shift+D hides the window; its shells keep running. Launching Vibex again (Start menu, Win+R) brings it back, as does `vibex attach [name]` or the palette in another window. *Quit Vibex* closes everything, detached workspaces included.
- **Layouts:** palette → *Save this window as a layout* writes `~/.config/vibex/layouts/<workspace>.lua`; *Open layout: name* or `vibex --layout name` opens it as a new workspace. Layout files are plain Lua you can write by hand: the same shape as `session.lua`, and a pane can `run` a command once its shell starts:

```lua
return {
  tabs = {
    { title = "code", root = { split = "right", ratio = 0.6,
        a = { pane = { profile = "Dev", cwd = "C:/code/my-project", run = "nvim ." } },
        b = { pane = { profile = "Dev", cwd = "C:/code/my-project", run = "cargo watch -x test" } } } },
    { title = "wsl", root = { pane = { profile = "wsl" } } },
  },
}
```

## Quake-style dropdown

Win+` (from any app) slides a Vibex down across the top of the screen; press it again to tuck it away. It's borderless, always on top and off the taskbar.

| Option | What | Default |
|---|---|---|
| `quake` | `{ key, mods, height }` (height is a fraction of the screen), or `false` | `` { key = "`", mods = "WIN", height = 0.45 } `` |

If another program owns the combination (Windows Terminal's Quake mode uses Win+`), Vibex says so; pick another key.

## Remote control

| Option | What | Default |
|---|---|---|
| `remote_control` | allow `vibex cli ...` (also in Settings → Terminal) | `false` |

Off by default because any program running in a pane could then read and type into your other panes. Every shell gets `VIBEX_PANE` (its pane id), which `vibex cli` uses as the default target:

```
vibex cli list                                  # JSON lines: window, workspace, tab, pane, title, cwd
vibex cli spawn --cwd C:\src -- cargo watch     # new tab; prints the pane id
vibex cli split --down -- htop
vibex cli send-text --pane 4 --enter "git pull"
vibex cli get-text --pane 4 --lines 50
vibex cli focus --pane 4
vibex cli set-tab-title "build"
vibex cli popup -- lazygit
vibex cli notify "Deploy" "finished"
vibex cli settings shortcuts                    # open the Settings dialog on a page
```

`VIBEX_CONFIG_DIR` (another config folder) and `VIBEX_INSTANCE` (a separate Vibex with its own pipe) help when scripting or testing.

## Tabs between windows

Right-click a tab: rename, colour, duplicate, move to a new window or into another open window, close others, close to the right, close. Or drag a tab out of the tab bar: drop it on another Vibex window's tab bar to move it there, anywhere else for a new window. The shells keep running through the move. `vibex cli move-tab [--pane N] [--window W | --new-window]` does the same from a script.

## Command history

Every command that finishes (with shell integration) is kept in `~/.config/vibex/history.tsv`: from every pane, window and past session, with its exit code, how long it ran and the folder. **Ctrl+Shift+H** searches it (fuzzy). Enter types the command at the prompt; Shift+Enter runs it. Start a command with a space to keep it out (bash's `ignorespace`).

| Option | What | Default |
|---|---|---|
| `command_history` | keep finished commands (also Settings → Terminal) | `true` |

## Suggestions and completions

While you type at a prompt, the newest past command starting with what you've typed shows dim after the cursor (one run in the same folder wins). **Right** or **End** takes it, **Ctrl+Right** takes the next word. It stays out of the way when the shell draws its own prediction after the cursor (PSReadLine's), and inside full-screen apps.

**Ctrl+Shift+Space** opens a popup at the cursor: matching commands from history, then files and folders matching the last word (relative to the pane's folder). Up / Down choose, Enter or Tab types it, any other key closes it and goes to the shell. (Ctrl+Space stays PSReadLine's.)

| Option | What | Default |
|---|---|---|
| `suggestions` | the dim suggestion while typing (also Settings → Terminal) | `true` |
| `linux_commands` | Linux commands in PowerShell and cmd, Windows ones in WSL ([below](#linux-and-windows-commands)) | `true` |
| `editor_updates` | look for new nano / Neovim releases once a week ([below](#nano-and-neovim)) | `true` |

## SSH hosts

Vibex notices `ssh [options] [user@]host` typed at a prompt (and hosts opened from the SSH picker, Ctrl+Shift+S), remembers them (`ssh_recent.tsv`; the picker lists them first, with when you last connected) and can make some look different:

```lua
config.ssh_hosts = {
  { match = "*prod*", color = "#c42b1c", label = "PROD" },        -- red tab + red border round the pane
  { match = "pi",     color = "#57ab5a", theme = "vibex-ember" },  -- its own colours while connected
}
```

`match` is a `*` / `?` pattern (with or without the `user@`); the first rule that matches wins. Until you set `ssh_hosts`, anything with "prod" in its name is red with a PROD label.

## Recording

**Alt+Shift+R** starts recording the focused pane (● in the tab title) and stops it. Recordings are asciicast v2 files in `~/.config/vibex/recordings/`, so asciinema can play them too. The palette has *Play recording: …* (a new tab replays it on its own clock, long pauses shortened; any key closes it) and *Export recording as GIF: …*. From a shell:

```
vibex cast2gif session.cast [out.gif] [--fps 12] [--speed 1] [--idle 2] [--open]
vibex cli play session.cast
vibex cli action ToggleRecording
```

The GIF is drawn with your font and theme, one 256-colour palette, only the part of each frame that changed.

## More `vibex cli`

```
vibex cli action [--pane N] NAME     # any action by name: SplitRight, HistorySearch, ToggleRecording ...
vibex cli prompt [--pane N]          # what's typed at the prompt + the suggestion (JSON)
vibex cli move-tab --pane 4 --new-window
```

## Secret guard

API keys, tokens and private keys are masked on screen as they appear (AWS, GitHub, GitLab, Slack, Stripe, Google, `sk-…` keys like OpenAI's and Anthropic's, Hugging Face, npm, JSON web tokens, PEM private keys, and `password=` / `api_key:` style assignments). Palette → *Reveal masked secrets* shows them until you toggle it back. Pasting one asks first. They're replaced with `‹redacted›` before anything is saved: command history, recordings (so GIFs too) and the audit log.

| Option | What | Default |
|---|---|---|
| `secret_guard` | all of the above (also Settings → Terminal → Safety) | `true` |
| `secret_patterns` | more kinds: `{ ["company token"] = "MYCO-[0-9a-f]{32}" }` | |

## Asking before risky commands

Enter on a prompt line that matches one of these asks first (Cancel has the keyboard, so a second Enter doesn't run it): `rm -rf /` or `~` or `*`, `Remove-Item -Recurse -Force` on a drive root, `git push --force`, `git reset --hard` / `git clean -fd`, `DROP TABLE` / `TRUNCATE` / `DELETE FROM x;`, `mkfs` / `format C:` / `Format-Volume`, `dd of=/dev/sd…`, `shutdown` / `Stop-Computer`, a fork bomb, `chmod 777 /`, `kubectl delete ns|all`, `terraform destroy`.

```lua
config.confirm_commands = { ["touches prod data"] = "prod_db", "\bnpm publish\b" }  -- added to the built-in ones
-- config.confirm_commands = false   -- none at all
config.ssh_hosts = { { match = "*prod*", color = "#c42b1c", label = "PROD", confirm = true } }  -- ask before every command there
```

The built-in `*prod*` host rule has `confirm = true`. Over SSH (no shell integration on the far side) the typed text is read from the cursor's line after the prompt (`$ `, `# `, `> `, `% `).

## Closing with something running, and reopening

Closing a pane, tab or window while a command runs (or a full-screen app is open) asks first (`confirm_close`, default `true`). Closed tabs come back with **Ctrl+Alt+Shift+T**, or the tab menu / palette → *Reopen closed tab*: the same splits, folders, title, colour and scrollback (the last 10 tabs in this window).

## Audit log

Settings → Terminal → *Audit log* (or `config.audit_log = true`) keeps every command in `~/.config/vibex/audit.jsonl`: when, machine and user, workspace and pane, SSH host, folder, the command, exit code, how long it took, how many lines it printed and their SHA-256 (`config.audit_output = true` also keeps the output itself, up to 64 KB, secrets redacted). Each line holds the SHA-256 of the line before it and of its own contents, so changing, removing or reordering anything breaks the chain from that point.

```
vibex audit verify [FILE]                                  # exit 0 intact, 1 broken (and where)
vibex audit report [FILE] [--out report.html] [--since 2026-11-01] [--open]
```

The palette has *Export audit report* (writes `audit-reports/audit-….html` and opens it) and *Verify the audit log*.

## Focus mode

**Ctrl+Shift+Enter**: full screen, no tab bar or status bar, splits that don't have focus faded, and notifications held until you leave; then you get a summary of what came in.

## Notes beside a pane

**Alt+Shift+N** opens a notes panel on the right of the pane (and again closes it); click it or press the keys again to write, Esc goes back to the terminal. Plain text, saved as you type in `~/.config/vibex/notes/`, one file per folder, so a project's notes come back whenever you're in it. `vibex cli notes TEXT` adds a line from a script.

## Pictures and Markdown in the terminal

```
vibex show README.md diagram.png        # or, in a Vibex shell: vshow README.md diagram.png
```

PNG images are drawn inline (kitty graphics, so kitty / WezTerm / Ghostty show them too); Markdown is rendered with headings, lists, check boxes, quotes, code blocks, tables and clickable links. `vshow` comes with the shell integration scripts.

Image size (the aspect is always kept):

| Flag | Size |
|---|---|
| `--width N` / `--height N` | N columns wide / N rows tall |
| `--fit` | as big as fits the window, both ways |
| `--scale 50%` (or `0.5`, `2x`) | a share of its own size; the picture is resized to exactly that many pixels |
| `--px N` | N pixels wide |
| `--actual` | its own size, pixel for pixel |
| `--center` | centred (default: left) |

With no flag it shows at its own size, shrunk to the window's width when wider.

## The Kali Linux look

Vibex can look like Kali Linux's terminal (QTerminal), colour for colour. Everything comes from Kali's own configuration.

**One click:** Settings → Appearance → **Style** → *Dragon (Kali Linux)* sets all of it at once (in settings.lua); *Vibex* takes it all back off. From a script: `vibex cli style dragon` / `vibex cli style vibex`. New tabs get the prompt.

**Or piece by piece**, in vibex.lua:

```lua
config.theme = "dragon-dark"          -- Kali's Kali-Dark colours
config.prompt = "dragon"              -- ┌──(user㉿host)-[~]  └─$
config.title_bar = "classic"          -- title, round buttons, Session · Actions · Edit · View · Help
config.font = { family = "Fira Code", size = 10 }   -- Kali's font and size
config.window = { padding = { x = 5, y = 5 }, opacity = 0.95 }   -- its 5 px margin and 5% transparency
config.shell_marks = false            -- Kali has no gutter marks (command blocks still work)
config.status_bar = false
```

- **Dragon Dark** is Kali's 16 colours (from its `kali-themes` package) on the Kali-Dark background (`#23252e`), with white text and a white block cursor.
- **The Dragon prompt** copies Kali's `.bashrc` and `.zshrc`:
  - green lines with the user, host and `$` in bold blue, and the path in bold
  - light blue lines, bold red names and `#` when run as Administrator
  - `(venv)` when a Python virtual environment is on
  - long paths shortened to `first\…\last four`
  - a blank line before every prompt but the first
  - the window title `user@host: path`
- **With the Dragon prompt, `ls` is Kali's `ls` in PowerShell:** plain `ls` and `ls folder` print coloured columns, using `dircolors`' database plus Kali's `ow=30;44` and its case-insensitive sorting. Piped `ls` and PowerShell parameters (`ls -Recurse`) still work the PowerShell way.
- **Fira Code** is free (SIL Open Font License). Put `FiraCode-Regular.ttf` and `FiraCode-Bold.ttf` from [its releases](https://github.com/tonsky/FiraCode/releases) in `%USERPROFILE%\.config\vibex\fonts\`, or install it for Windows.

## Linux and Windows commands

Every shell understands both kinds (`config.linux_commands = true`, also Settings → Terminal; new tabs pick it up).

**PowerShell** (5.1 and 7) gets the Linux ones, from Git for Windows' GNU tools when Git is installed (Vibex finds `usr\bin` itself, or set `VIBEX_GNU`), else PowerShell versions of the everyday ones:

| | |
|---|---|
| files | `ls -la` `ll` `la` `cat -n` `cp -r` `mv` `rm -rf` `mkdir -p` `touch` `ln -s` (a junction for folders when symlinks aren't allowed) `find . -name '*.rs'` `du -sh` `diff a b` |
| text | `grep -rn` `sed` `awk` `head` `tail -f` `wc -l` `sort` `uniq -c` `cut` `tr` `xargs` `tee -a` `less` `base64` `sha256sum` … (pipe anything in: `Get-Process | grep chrome`) |
| where things are | `which` (also says when a name is a cmdlet, alias or Vibex command) · `where git` (where.exe) |
| processes | `ps aux` / `ps -ef` · `kill -9 PID` · `pkill` / `killall` / `pgrep -l` · `top` / `htop` (q quits) |
| system | `df -h` `free -h` `uptime` `nproc` `uname -a` `ifconfig` `ip a` `ip r` `ss -tlnp` `netstat -tulpn` `lsof -i :3000` |
| shell | `export X=1` · `export PATH=$PATH:C:\tools` · `env` · `env X=1 cmd` · `printenv` · `unset` · `alias ll='ls -la'` · `cd -` · `pwd` · `sudo cmd` (Windows' sudo when it's on, else an elevated window) · `open .` / `xdg-open` · `wget URL` · `man grep` |
| one-liners | `a && b`, `a || b`, `2>/dev/null`, `&>/dev/null` work in PowerShell 5.1 too (rewritten as you press Enter) |
| anything else | a name PowerShell can't find that Git's `usr\bin` has (`nano`, `vim`, `file`, `stat`, `realpath` …) runs from there |

Plain PowerShell keeps working: `ls` alone is still `Get-ChildItem` (objects), `ls -r` still recurses, `ls | sort Length`, `Get-Process | where Name -eq x`, `kill -Name notepad`, `cat file` and `diff $a $b` (Compare-Object) do what they always did. Only Linux-looking flags (`-la`, `-rf`, `--color`) switch a command to the Linux behaviour. cmd's own switches still go to cmd: `dir /s /b`, `del /q`, `rd /s /q`, `copy /y`, `find /i "text" file`.

**cmd** gets the same names as typed macros (`ls`, `ll`, `grep`, `cat`, `rm`, `cp`, `mkdir`, `which`, `pwd`, `clear`, `export`, `ps`, `uname` …); batch files never see them.

**WSL and Git Bash** get the Windows ones: `where` · `cls` · `ver` · `start FILE|URL` · `open` · `explorer .` · `notepad FILE` · `pbcopy` / `pbpaste` · `copy` / `move` / `del` / `ren` / `md` / `rd /s /q` · `findstr /i /n` (grep underneath), and any Windows program without typing `.exe`: `ipconfig`, `tasklist`, `taskkill`, `systeminfo`, `winget`, `netsh` …

## nano and Neovim

Vibex comes with both editors, so `nano FILE` and `nvim FILE` work in every Vibex shell, even without Git or WSL:

- **Neovim**: the official Windows build from github.com/neovim/neovim.
- **GNU nano**: MSYS2's build (GNU has no Windows build of its own; it's the one Git for Windows uses) with the runtime it needs, syntax colours for every language it knows, and mouse support. Its history goes in your profile folder.

They live in `tools\` next to `vibex.exe` and go at the **end** of PATH in Vibex's shells, so an editor you installed yourself (say `C:\Program Files\Neovim`) still wins. Licences and source links are in `tools\LICENSES`.

**Updates, weekly** (`config.editor_updates`, Settings → Updates → nano and Neovim): once a week Vibex looks at Neovim's latest GitHub release and MSYS2's package database. When something is newer, a toast says so; nothing installs by itself. **Update nano and Neovim** (palette, or the Update button in Settings) shows what changes, downloads it, checks every file against its published SHA-256 and swaps the folder in (close nano and Neovim first). From a shell:

```
vibex editors            # what's installed
vibex editors check      # anything newer?
vibex editors update     # get it
```

Building the package: `tools\fetch-bundle.ps1` downloads the versions pinned (with SHA-256) in `tools\bundle.lock` into `vendor\tools`, and `tools\package.ps1` runs it when needed and puts the editors in the setup.

## Screenshots and saving lines

| | |
|---|---|
| **Alt+Shift+S** | the pane as a PNG, saved to `Pictures\Vibex` and copied |
| **Alt+Shift+C** | the last command and its output, with a title bar: the command, a green / red dot, exit code and how long it ran (right-click a command for that one) |
| right-click a selection → **Copy as image** | the selection as a picture on the clipboard (PNG and bitmap, so chat apps and Paint both take it) |
| **Alt+Shift+M** | mark a start line; scroll; **Alt+Shift+M** again (or right-click → *Mark end here*) on the last line, then pick a format. The marked lines are tinted while you choose |
| **Alt+Shift+E** | save lines as: the selection if there is one, else this command, the screen or everything |

Formats: PNG (your theme and font), plain text, ANSI (`.ans`, keeps colours: `cat` it later), HTML (a styled page), SVG (sharp at any size) and a Markdown code block. Pictures of a selection or a command are trimmed to the text's width.

**Secrets never leave**: while the secret guard is on, every export masks what it matches (also across wrapped lines, and even with secrets revealed on screen), and pictures blur them on top. A toast says how many were masked.

From scripts: `vibex cli export --pane N [--format png|txt|ans|html|svg|md] [--out FILE] [--copy] [--block | --all | --last N | --from A --to B]` (lines counted from 1, the oldest kept).

## Engagement mode

For assessments and pentests. Settings → **Security** → *Start a new engagement* (type a name, Enter), or pick one from the *Engagement* dropdown; the tab bar shows `● engagement: name` while it's on. Everything goes in `~/.config/vibex/engagements/<name>/`:

| | |
|---|---|
| `audit.jsonl` | every command with its output, hash-chained (as the audit log, always on during an engagement) |
| `loot.tsv` | what was found in the output: NTLM / NetNTLMv2 / Kerberos / crypt hashes, credentials (hydra, netexec / crackmapexec), API keys and tokens, and open ports from nmap with their host |
| `recordings/` | each pane's session as a `.cast`, when *Record every pane* is on |
| `report.html` | *Export* on the Security page or the palette: scope, findings (masked), recordings and the verified command log |

**Scope.** On the Security page: addresses (`10.0.0.5`), ranges (`10.0.0.0/24`, `192.168.1.10-20`), domains (`acme.com` and `*.acme.com` cover subdomains), and `!x` to leave one out. With *Check the scope* on, Enter on a network tool aimed at anything outside it asks first: nmap, masscan, curl, wget, ssh, scp, nc, sqlmap, gobuster, ffuf, nuclei, hydra, netexec / crackmapexec, smbclient, evil-winrm, impacket tools, dig, ping and more. Output and wordlist files given with `-o`, `-oA`, `-w`, `-iL`… aren't mistaken for targets.

**Loot.** **Alt+Shift+L** lists it (masked; Enter copies the full value; palette → *Reveal masked secrets* shows them). A toast says when something new turns up.

```
vibex cli engagement start acme-q4 --scope "10.0.0.0/24,*.acme.local,!10.0.0.1"
vibex cli engagement scope 10.0.0.0/24 10.0.1.0/24
vibex cli engagement status        # name, folder, scope, how much loot
vibex cli engagement report        # writes report.html
vibex cli engagement stop
```

| Option | What | Default |
|---|---|---|
| `engagement` | the engagement that's on (a folder name), or nothing | |
| `scope_guard` | ask before network tools leave the scope | `true` |
| `collect_loot` | collect hashes, credentials, keys, open ports | `true` |
| `engagement_record` | record every pane | `false` |
| `confirm_risky` | ask before risky commands (`confirm_commands`) | `true` |

## Settings dropdowns

Every choice in Settings (theme, font, padding, cursor, scrollback, default profile, right-click, engagement) is a dropdown: click it, or Enter on the row; ↑ / ↓ and Enter pick, the wheel scrolls long lists like fonts. ← / → still step through the choices without opening it.
