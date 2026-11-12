# Vibex shortcuts

These are the defaults. You can rebind any of them, or turn them off, in Settings → Shortcuts or in `vibex.lua` (see [docs/CONFIG.md](docs/CONFIG.md#keys)).

If you forget one, open the **command palette** (<kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>P</kbd>). It lists every action with its key.

## Essentials

| Action | Keys |
|---|---|
| Command palette | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>P</kbd> |
| Settings | <kbd>Ctrl</kbd>+<kbd>,</kbd> |
| Copy | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>C</kbd> (or right-click → Copy) |
| Paste | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>V</kbd>, <kbd>Shift</kbd>+<kbd>Insert</kbd>, or middle-click |
| Select all | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>A</kbd> |
| Find (regex: <kbd>Ctrl</kbd>+<kbd>R</kbd>, match case: <kbd>Alt</kbd>+<kbd>C</kbd>) | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>F</kbd> |
| Right-click menu | right-click (every item shows its shortcut) |

## Tabs and windows

| Action | Keys |
|---|---|
| New tab | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>T</kbd> (right-click **+** for profiles) |
| Next / previous tab | <kbd>Ctrl</kbd>+<kbd>Tab</kbd> / <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>Tab</kbd> |
| Go to tab 1–8 / last | <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>1</kbd> … <kbd>8</kbd> / <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>9</kbd> |
| Reopen the last closed tab | <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>T</kbd> |
| New window | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>N</kbd> |
| Detach the window (shells keep running) | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>D</kbd>, then bring it back by starting Vibex again or with `vibex attach` |
| Fullscreen | <kbd>Alt</kbd>+<kbd>Enter</kbd> |
| Focus mode (full screen, no bars) | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>Enter</kbd> |
| Dropdown terminal, from anywhere | <kbd>Win</kbd>+<kbd>`</kbd> |

Mouse: drag a tab to reorder it, or drag it out of the bar to move it to another window or a new one. Double-click a tab to rename it, middle-click to close it, and right-click it for more.

## Panes

| Action | Keys |
|---|---|
| Split right / down | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>=</kbd> / <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>-</kbd> |
| Close pane | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>W</kbd> |
| Move between panes | <kbd>Alt</kbd>+<kbd>←</kbd> <kbd>→</kbd> <kbd>↑</kbd> <kbd>↓</kbd> |
| Resize a pane | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>←</kbd> <kbd>→</kbd> <kbd>↑</kbd> <kbd>↓</kbd> (or drag the divider) |
| Zoom a pane | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>Z</kbd> |
| Floating pane | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>F</kbd> |
| Notes beside the pane | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>N</kbd> (<kbd>Esc</kbd> goes back to the terminal) |
| Type into every pane of the tab | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>B</kbd> |

## Commands and history

| Action | Keys |
|---|---|
| Search everything you've run | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>H</kbd> (<kbd>Enter</kbd> types it, <kbd>Shift</kbd>+<kbd>Enter</kbd> runs it) |
| Take the dim suggestion | <kbd>→</kbd> or <kbd>End</kbd> (<kbd>Ctrl</kbd>+<kbd>→</kbd> takes one word) |
| Commands and files that fit here | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>Space</kbd> |
| Previous / next prompt | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>↑</kbd> / <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>↓</kbd> |
| Copy the last command's output | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>O</kbd> |
| SSH hosts | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>S</kbd> |

Prompt jumping, the last command's output and the sticky command header need [shell integration](docs/SHELL_INTEGRATION.md).

## Selecting and copying

| Action | Keys |
|---|---|
| Quick select (type a label to copy a URL, path, hash or IP) | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>Q</kbd> |
| Copy mode (vi keys: `v` select, `V` lines, <kbd>Ctrl</kbd>+<kbd>V</kbd> block, `y` copy) | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>X</kbd> |
| Select a word / a line | double-click / triple-click |
| Select inside mouse apps (vim, tmux) | hold <kbd>Shift</kbd> while dragging |
| Open a link or `file:line` | <kbd>Ctrl</kbd>+click |

## Screenshots, saving and recording

| Action | Keys |
|---|---|
| Screenshot the pane (saved and copied) | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>S</kbd> |
| Screenshot the last command, with a title bar | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>C</kbd> |
| Mark a start line, then the end line | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>M</kbd> (twice) |
| Save lines as PNG / text / ANSI / HTML / SVG / Markdown | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>E</kbd> |
| Record the pane (start / stop) | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>R</kbd> |
| Engagement loot | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>L</kbd> |

## View

| Action | Keys |
|---|---|
| Bigger / smaller / reset text | <kbd>Ctrl</kbd>+<kbd>=</kbd> / <kbd>Ctrl</kbd>+<kbd>-</kbd> / <kbd>Ctrl</kbd>+<kbd>0</kbd> |
| Scroll a page | <kbd>Shift</kbd>+<kbd>PgUp</kbd> / <kbd>Shift</kbd>+<kbd>PgDn</kbd> (or the mouse wheel) |
| Top / bottom of history | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>Home</kbd> / <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>End</kbd> |
| Reload the config | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>R</kbd> (it also reloads by itself when you save) |

## Title bar

Drag empty space to move the window, double-click it to maximise, and drag any edge to resize.

The full reference, including the optional leader-key layer from the starter `vibex.lua`, is in [docs/KEYS.md](docs/KEYS.md).
