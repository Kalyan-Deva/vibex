# Vibex Terminal — architecture

A from-scratch terminal emulator in Rust. Windows first (ConPTY); the core is
portable and Linux / macOS use a Unix pty. Everything that makes it a
terminal is written here: PTY layer, VT parser, terminal state machine,
renderer. Crates are used only for OS windowing, font rasterising and the
clipboard.

```
 keyboard/mouse ──► vibex (app) ──bytes──► vibex-pty ──► ConPTY ──► pwsh / wsl / ssh
                        ▲                                   │
                        │ redraw                            │ output bytes
                 vibex-render ◄── vibex-term ◄── vibex-vt ◄─┘
                 (pixels)        (grid/state)   (parser)
```

## Crates

| Crate | Role | Written from scratch |
|---|---|---|
| `vibex-vt` | DEC/ANSI escape-sequence parser (Paul Williams state machine) + UTF-8 decoder. Emits `print / execute / csi / esc / osc / dcs` events. | yes, no deps |
| `vibex-term` | Terminal state: screen grid, scrollback ring, alternate screen, cursor, scroll regions, tab stops, modes, SGR colours, replies (DSR/DA). Implements `vibex_vt::Perform`. | yes |
| `vibex-pty` | ConPTY: pipes, `CreatePseudoConsole`, process spawn with the pseudoconsole attribute, reader thread, resize. | yes (raw Win32 via `windows-sys`) |
| `vibex-render` | `scene` turns the grid into an ordered list of rects + glyphs; `gpu` (wgpu, atlas, instanced quads) or `cpu` (softbuffer fallback) draws it. `raster` is a hand-written outline rasteriser; `font` does fallback chains, shaping and COLR emoji. | yes (`ttf-parser` parses fonts, `rustybuzz` shapes, `wgpu` talks to the GPU) |
| `vibex` | The app: `main` (window, title bar, tabs, input routing), `pane` (one terminal + shell, selection, search), `layout` (split tree), `palette`, `config`, `keys`, `lua` (config engine, `win` API, events, hot reload). | yes (`winit`, `softbuffer`, `arboard`, `mlua`) |

## Data flow

1. A reader thread blocks on the ConPTY output pipe and sends chunks to the
   UI thread, waking the event loop.
2. The UI thread feeds bytes through `Parser::advance(&mut terminal, byte)`.
3. The terminal mutates its grid; any replies (cursor position report,
   device attributes) are queued and written back to the PTY.
4. A redraw is requested; the renderer paints the visible rows (scrollback
   offset applied) into the window surface.
5. Key presses are encoded to VT sequences (respecting application-cursor
   mode, xterm modifier encoding) and written to the PTY input pipe.

## Subsystems

- **Rendering** (`vibex-render`): one instanced wgpu draw over an RGBA glyph atlas, with a CPU fallback that presents only changed scanlines. Glyphs come from a hand-written outline rasteriser, shaped with rustybuzz for ligatures; COLR v0/v1 colour emoji; DirectComposition transparency, acrylic and glass; a Gaussian blur pass (`blur.wgsl`) behind dialogs.
- **Configuration** (`lua.rs`, `config.rs`, `settings.rs`): `vibex.lua` runs in Lua 5.4 (`mlua`) and returns a table; hot reload keeps the old config on errors. Settings-window changes go to a separate `settings.lua` layered on top, so hand-written config is never rewritten.
- **Images** (`vibex-term/src/image.rs`, `sixel.rs`, `kitty.rs`): a placement is cut into one slice per text row and stored on the `Row`, so images scroll, enter scrollback, reflow and get evicted with their text. Renderers draw `Item::Image`: the CPU path samples straight into the frame; the GPU path keeps one texture per image (cached by uid, evicted after 600 unused frames or past 512 MB).
- **OpenConsole** (`vibex-pty/src/openconsole.rs`): the built-in console host drops DCS/APC, so Vibex launches the newer OpenConsole itself (what `conpty.dll` does: a ConDrv server handle, `--headless --signal --server`, a `\Reference` handle) when it sits next to `vibex.exe`.
- **Damage**: `Terminal::generation()` moves on anything visible; output that only answers queries doesn't redraw. The CPU backend presents only changed scanlines.
- **Windows**: `App` holds one `Session` per window; later launches hand their request to the first process over a per-user named pipe (`single.rs`).
- **Accessibility** (`access.rs`): AccessKit tree — window → one `Terminal` per pane → one text run per row with character positions; the cursor is the caret.
- **Copy mode** (`copymode.rs`), **SSH hosts** (`ssh.rs`), **updates** (`vibex-update`, `update.rs`, `vibex-release`).
- **Unix** (`vibex-pty/src/unix.rs`): `posix_openpt`, `setsid` + `TIOCSCTTY`, `TIOCSWINSZ` resizes.
- **Commands everywhere** (`commands.rs`, `shell/vibex-commands.ps1`, `shell/vibex.bash`): Vibex finds Git for Windows' GNU tools and tells the shells. PowerShell gets Linux-style commands (shims that keep plain PowerShell behaviour, and piped input runs through a small `tr -d '\r'` pipeline so CR LF never reaches the tools), cmd gets typed-only doskey macros, WSL gets Windows commands and `.exe`-less programs.
- **Bundled editors** (`bundle.rs`, `tools/fetch-bundle.ps1`, `tools/bundle.lock`): nano (MSYS2) and Neovim, pinned by SHA-256 and appended to PATH in Vibex shells. The weekly check compares the lock with Neovim's latest GitHub release and MSYS2's package database; an update writes a new lock, runs the bundled `update.ps1` into a fresh folder and swaps it in.
- **Export** (`export.rs`, `export_ui.rs`): a line range, selection or command block is copied into a fresh `Terminal` with secrets masked, then written as text, ANSI, HTML, SVG or Markdown, or drawn by the CPU renderer to PNG (and to the clipboard as a bitmap and PNG).
- **Safety** (`secrets.rs`, `audit.rs`, `engagement.rs`): the secret guard's patterns, the hash-chained audit log, and engagement mode's scope guard, loot collector and report.
