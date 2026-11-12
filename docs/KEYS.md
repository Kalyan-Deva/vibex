# Vibex default keys and mouse

Every key below can be rebound or unbound in `vibex.lua`; see [CONFIG.md](CONFIG.md).
The command palette (**Ctrl+Shift+P**) lists everything with its key.

| Action | Binding |
|---|---|
| **Command palette** | Ctrl+Shift+P (type to filter, Enter runs, Esc closes) |
| Copy selection | Ctrl+Shift+C, or right-click with a selection |
| Paste | Ctrl+Shift+V, Shift+Insert, right-click (no selection), middle-click |
| Search | Ctrl+Shift+F, then type; Enter / ↑ = older, Shift+Enter / ↓ = newer, Esc closes |
| **New tab** | Ctrl+Shift+T, or the `+` button (right-click `+` for profiles) |
| Next / previous tab | Ctrl+Tab / Ctrl+Shift+Tab |
| Go to tab 1–8 / last | Ctrl+Alt+1 … Ctrl+Alt+8 / Ctrl+Alt+9 |
| Close tab | the tab's ✕, or middle-click the tab |
| **Split right / down** | Alt+Shift+= / Alt+Shift+- |
| Close pane | Ctrl+Shift+W (closes the tab with its last pane) |
| Move between panes | Alt+←/→/↑/↓, or click a pane |
| Resize pane | Alt+Shift+←/→/↑/↓, or drag the divider |
| Zoom pane | Ctrl+Shift+Z |
| **Previous / next prompt** | Ctrl+Shift+↑ / Ctrl+Shift+↓ (needs [shell integration](SHELL_INTEGRATION.md)) |
| **Quick select** | Ctrl+Shift+Q, then type the label to copy; a capital letter copies *and* types it; Esc cancels |
| **Copy mode** | Ctrl+Shift+X. Move with `h j k l`, `w b e`, `0 ^ $`, `gg G`, `H M L`, Ctrl+U/D, Ctrl+B/F (counts work: `5j`). `v` select, `V` lines, Ctrl+V block, `o` swap ends, `y`/Enter copy, `/` search, `n`/`N` next/previous hit, Esc clears the selection then leaves, `q` leaves |
| **Find options** | in the find bar: Ctrl+R toggles regex, Alt+C toggles match case |
| **Engagement** | Alt+Shift+L: the loot collected so far (Enter copies); Settings → Security to start one |
| **Safety** | risky commands ask before Enter runs them; pasting a secret asks; closing something busy asks; Ctrl+Alt+Shift+T reopens the last closed tab |
| **Focus mode** | Ctrl+Shift+Enter: full screen, no bars, notifications held |
| **Notes** | Alt+Shift+N: a notes panel beside the pane (Esc back to the terminal) |
| **Tabs** | right-click a tab for its menu; drag a tab out of the bar to move it to another window or a new one |
| **History** | Ctrl+Shift+H searches every command you've run; Enter types it, Shift+Enter runs it |
| **Suggestions** | Right / End takes the dim suggestion, Ctrl+Right one word; Ctrl+Shift+Space lists commands and files |
| **Screenshots** | Alt+Shift+S saves the pane as a PNG (and copies it); Alt+Shift+C the last command with a title bar (exit code, time); right-click a selection → Copy as image |
| **Save lines** | Alt+Shift+M marks a start line, scroll, Alt+Shift+M (or right-click → Mark end here) at the end; Alt+Shift+E saves a selection, command, screen or everything as PNG, text, ANSI, HTML, SVG or Markdown. Secrets are masked in all of them |
| **Recording** | Alt+Shift+R starts / stops recording the pane (play it or make a GIF from the palette) |
| **Right-click** | the context menu (shortcuts shown next to each item); Ctrl+Shift+A selects everything |
| **Floating pane** | Alt+Shift+F shows / hides a floating pane over the tab |
| **Last command** | Ctrl+Shift+O copies the last command's output; the palette can copy the command or run it again; click the left margin to select a block, click the sticky header to jump to its prompt |
| **Detach** | Ctrl+Shift+D hides the window with its shells still running; launch Vibex again (or `vibex attach`) to bring it back |
| **Dropdown** | Win+` from anywhere: the Quake-style Vibex |
| **Open file:line** | Ctrl+click `path:line[:col]` in output to open it in your editor (`config.editor`) |
| **Settings** | Ctrl+, opens the Settings tab (also in the ⌄ menu next to +, with your profiles, the palette and About) |
| **Broadcast input** | Ctrl+Shift+B toggles typing into every pane of the current tab (tab shows ⇶, a red banner while on) |
| **SSH hosts** | Ctrl+Shift+S lists hosts from `~/.ssh/config` (and its `Include`s) and `~/.ssh/known_hosts`; Enter opens a new tab running `ssh` to it |
| Reload config | Ctrl+Shift+R (also automatic on save) |
| New window | Ctrl+Shift+N |
| Fullscreen | Alt+Enter |
| Scroll history | Mouse wheel, Shift+PageUp / Shift+PageDown |
| Top / bottom of history | Ctrl+Shift+Home / Ctrl+Shift+End |
| Font size | Ctrl+= / Ctrl+- / Ctrl+0 (reset) |
| Select | Drag; double-click = word, triple-click = line |
| Open link | Hold Ctrl to underline it, Ctrl+click to open (http/https only) |
| Select inside mouse apps (vim, tmux) | Hold Shift while dragging |

**Title bar** (with `window.decorations = "vibex"`): drag empty space to move
the window, double-click it to maximize, drag any window edge to resize.

The starter `vibex.lua` adds a leader key (**Ctrl+Shift+Space**), then:
`c` new tab · `u` Ubuntu tab · `p` PowerShell tab · `w` new window ·
`v` split right · `s` split down · `x` close pane · `z` zoom · `h j k l` move
between panes · `r` reload · `Delete` clear scrollback · `f` fullscreen ·
`g` types `git status` · `t` cycles themes · `=` / `-` opacity.
