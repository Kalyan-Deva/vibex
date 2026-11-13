# Shell integration

With shell integration on, Vibex knows where each prompt starts, whether the
last command failed, and which folder the shell is in:

- **Prompt marks**: a thin mark in the left gutter on every prompt — green
  when the command succeeded, red when it failed, grey while it runs.
- **Jump between prompts**: Ctrl+Shift+↑ / Ctrl+Shift+↓.
- **New tabs and splits open in the current folder.**
- **Exit code in the status bar** (`✗ 1`) and `win:cwd()`, `win:last_exit_code()`
  in Lua.

It uses standard escape sequences (OSC 133 prompt marks, OSC 7 / OSC 9;9 for the
directory), so the scripts are tiny and do nothing outside Vibex.

## PowerShell

Add to your profile (`notepad $PROFILE`):

```powershell
$vibexIntegration = "<path to Vibex-terminal>\shell\vibex.ps1"
if (Test-Path $vibexIntegration) { . $vibexIntegration }
```

It wraps your existing `prompt` function, so a custom prompt keeps working.

## bash (Linux)

The Linux install puts the script in `~/.local/share/vibex/shell/`. Add to `~/.bashrc`:

```bash
__vibex_sh="$HOME/.local/share/vibex/shell/vibex.bash"
[ -f "$__vibex_sh" ] && . "$__vibex_sh"
```

(Use `/usr/local/share/vibex/shell/vibex.bash` if you installed with `--prefix /usr/local`.) Vibex sets `TERM_PROGRAM=Vibex` for its shells, so the script does nothing in other terminals.

## bash (WSL)

Add to `~/.bashrc`:

```bash
__vibex_sh="/mnt/c/<path to Vibex-terminal>/shell/vibex.bash"
[ -f "$__vibex_sh" ] && . "$__vibex_sh"
```

Vibex forwards `TERM_PROGRAM` into WSL through `WSLENV`, which is how the
script knows it is inside Vibex.

## Turning the gutter marks off

```lua
config.shell_marks = false
```

## Command blocks

The scripts now also send:

| Mark | When | Used for |
|---|---|---|
| `OSC 133;B` | the prompt ends, typing starts | reading the command text (bash) |
| `OSC 633;E;<command>` | Enter is pressed (PowerShell, via PSReadLine) | the exact command line, VS Code's escaping |
| `OSC 133;C` | the command starts running | where its output begins; how long it ran |
| `OSC 133;D;<exit>` | the next prompt | exit code |

That powers the sticky command header, copying / rerunning the last command, block selection and "long command finished" notifications. Open a new tab (or re-run `. $PROFILE`) after updating Vibex so the shell loads the new script.

