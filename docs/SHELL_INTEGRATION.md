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

It wraps your existing `prompt` function, so a custom prompt (oh-my-posh,
starship, your own) keeps working: it still sees the last command's `$?` and
`$LASTEXITCODE`, and can't change `$LASTEXITCODE` for the next command. It
also works under `Set-StrictMode -Version Latest`.

A command cancelled with Ctrl+C is marked with exit code 130 (Windows
PowerShell leaves it out of history, so the mark doesn't depend on history);
pressing Enter on an empty line marks nothing.

## bash (WSL)

Add to `~/.bashrc`:

```bash
__vibex_sh="/mnt/c/<path to Vibex-terminal>/shell/vibex.bash"
[ -f "$__vibex_sh" ] && . "$__vibex_sh"
```

Vibex forwards `TERM_PROGRAM` into WSL through `WSLENV`, which is how the
script knows it is inside Vibex. The same file works in Git Bash.

It is safe with `set -u` / `set -e` and to source more than once, and it gets
along with whatever else uses `PROMPT_COMMAND` (starship, bash-preexec, your
own): its hook runs first and hands the command's `$?` on unchanged, the marks
are put back on `PS1` at every prompt (for prompts that rebuild `PS1` each
time), and bash-preexec's `__bp_interactive_mode` stays last.

## Turning the gutter marks off

```lua
config.shell_marks = false
```

## Command blocks

The scripts now also send:

| Mark | When | Used for |
|---|---|---|
| `OSC 133;B` | the prompt ends, typing starts | reading the command text (bash) |
| `OSC 633;E;<command>` | Enter is pressed (PowerShell, via PSReadLine) | the exact command line, VS Code's escaping (`\\`, and `\xHH` for `;`, control characters and every non-ASCII UTF-8 byte, so it is plain ASCII) |
| `OSC 133;C` | the command starts running | where its output begins; how long it ran |
| `OSC 133;D;<exit>` | the next prompt | exit code (130 after Ctrl+C in PowerShell) |

The folder goes as `OSC 9;9;"C:\path"` from PowerShell, or as a
percent-encoded `OSC 7;file://host/C:/path` when the path has a `;` (OSC
splits its parameters there). bash always sends OSC 7, percent-encoding `%`,
`;`, spaces and control characters; Git Bash sends `/c/Users` as `/C:/Users`.

That powers the sticky command header, copying / rerunning the last command, block selection and "long command finished" notifications. Open a new tab (or re-run `. $PROFILE`) after updating Vibex so the shell loads the new script.

