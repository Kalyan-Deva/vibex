# Vibex shell integration for bash (WSL / Linux).
#
# Adds OSC 133 prompt marks and OSC 7 working-directory reports. Only
# active inside Vibex (Vibex forwards TERM_PROGRAM into WSL via WSLENV).
#
# Use it from ~/.bashrc:
#   [ -f /mnt/c/path/to/Vibex-terminal/shell/vibex.bash ] && . /mnt/c/path/to/Vibex-terminal/shell/vibex.bash

if [ "$TERM_PROGRAM" = "Vibex" ] && [ -z "$__VIBEX_INTEGRATED" ]; then
  __VIBEX_INTEGRATED=1

  __vibex_prompt() {
    local ec=$?
    printf '\e]133;D;%s\a' "$ec"
    printf '\e]7;file://%s%s\a' "${HOSTNAME:-localhost}" "$PWD"
  }

  case ";${PROMPT_COMMAND};" in
    *";__vibex_prompt;"*) ;;
    *) PROMPT_COMMAND="__vibex_prompt${PROMPT_COMMAND:+;$PROMPT_COMMAND}" ;;
  esac

  # config.prompt = "dragon": Kali Linux's two-line prompt (its .bashrc,
  # colour for colour), the title user@host: dir, and a blank line before
  # every prompt but the first
  if [ "$VIBEX_PROMPT" = dragon ]; then
    __vibex_pc='\[\033[;32m\]'
    __vibex_ic='\[\033[1;34m\]'
    if [ "$EUID" -eq 0 ]; then
      __vibex_pc='\[\033[;94m\]'
      __vibex_ic='\[\033[1;31m\]'
    fi
    VIRTUAL_ENV_DISABLE_PROMPT=1
    PS1='\[\e]0;\u@\h: \w\a\]'$__vibex_pc'┌──${VIRTUAL_ENV:+(\[\033[0;1m\]$(basename $VIRTUAL_ENV)'$__vibex_pc')}('$__vibex_ic'\u㉿\h'$__vibex_pc')-[\[\033[0;1m\]\w'$__vibex_pc']\n'$__vibex_pc'└─'$__vibex_ic'\$\[\033[0m\] '
    unset __vibex_pc __vibex_ic
    __vibex_gap() {
      if [ -n "$__VIBEX_PROMPT_SHOWN" ]; then echo; fi
      __VIBEX_PROMPT_SHOWN=1
    }
    PROMPT_COMMAND="${PROMPT_COMMAND:+$PROMPT_COMMAND;}__vibex_gap"
  fi

  # mark the prompt start and end (\[ \] keep bash's line-length maths
  # right); PS0 prints when a command starts, so output begins there
  PS1='\[\e]133;A\a\]'"$PS1"'\[\e]133;B\a\]'
  PS0='\e]133;C\a'"${PS0:-}"
fi

# vshow FILE...: images and Markdown drawn in the terminal (vibex show).
if [ -n "$VIBEX_EXE" ] && [ -e "$VIBEX_EXE" ]; then
  vshow() {
    local args=() f
    for f in "$@"; do
      if command -v wslpath >/dev/null 2>&1 && [ -e "$f" ]; then args+=("$(wslpath -w "$f")"); else args+=("$f"); fi
    done
    "$VIBEX_EXE" show --cols "${COLUMNS:-$(tput cols 2>/dev/null || echo 100)}" --rows "${LINES:-$(tput lines 2>/dev/null || echo 30)}" "${args[@]}"
  }
fi

# Windows commands next to the Linux ones (WSL and Git Bash): where, cls,
# ipconfig, tasklist, start, explorer, clip, copy/move/del/md/rd, findstr,
# and any Windows program without typing .exe. Off with
# config.linux_commands = false (Vibex passes VIBEX_COMMANDS=0).
if [ "$TERM_PROGRAM" = "Vibex" ] && [ "${VIBEX_COMMANDS:-1}" != 0 ] && [ -z "$__VIBEX_COMMANDS" ]; then
  __VIBEX_COMMANDS=1

  # a Windows path for a file here (WSL: wslpath, Git Bash: cygpath)
  __vibex_winpath() {
    if command -v wslpath >/dev/null 2>&1; then wslpath -w "$1" 2>/dev/null || printf '%s' "$1"
    elif command -v cygpath >/dev/null 2>&1; then cygpath -w "$1"
    else printf '%s' "$1"; fi
  }
  # cmd.exe complains when started in a \wsl$ folder: run it from C:
  __vibex_cmd() {
    if [ -d /mnt/c ]; then (cd /mnt/c && cmd.exe /d /c "$@"); else cmd.exe //d //c "$@"; fi
  }

  # ipconfig, tasklist, notepad, winget ... without .exe (then whatever
  # was there before: Ubuntu's "did you mean" package hints)
  if declare -F command_not_found_handle >/dev/null; then
    eval "__vibex_cnf_before() $(declare -f command_not_found_handle | tail -n +2)"
  fi
  command_not_found_handle() {
    if [[ "$1" != */* ]] && command -v "$1.exe" >/dev/null 2>&1; then
      local c="$1.exe"; shift; "$c" "$@"; return $?
    fi
    if declare -F __vibex_cnf_before >/dev/null; then __vibex_cnf_before "$@"; return $?; fi
    printf '%s: command not found\n' "$1" >&2
    return 127
  }

  # where NAME: everything bash would run for it, plus Windows programs
  # (Windows' PATH often lists a folder twice: each line once)
  where() {
    local out
    out="$(__vibex_where "$@")"
    [ -n "$out" ] || return 1
    printf '%s\n' "$out" | awk '!seen[$0]++'
  }
  __vibex_where() {
    local rc=1 n k
    for n in "$@"; do
      case "$n" in /*) continue ;; esac
      for k in $(type -a -t "$n" 2>/dev/null | sort -u); do
        case "$k" in
          alias) alias "$n" ;;
          function) printf '%s: shell function\n' "$n" ;;
          builtin) printf '%s: shell builtin\n' "$n" ;;
          keyword) printf '%s: shell keyword\n' "$n" ;;
        esac
        rc=0
      done
      type -a -P "$n" 2>/dev/null && rc=0
      if [[ "$n" != *.exe ]]; then type -a -P "$n.exe" 2>/dev/null && rc=0; fi
    done
    return $rc
  }
  cls() { clear; }
  ver() { __vibex_cmd ver; }
  # start / open FILE|FOLDER|URL in Windows (start alone: a Windows prompt here)
  start() {
    if [ $# -eq 0 ]; then __vibex_cmd start cmd.exe; return; fi
    local t
    for t in "$@"; do
      if [ -e "$t" ]; then t="$(__vibex_winpath "$t")"; fi
      __vibex_cmd start "" "$t"
    done
  }
  command -v open >/dev/null 2>&1 || open() { start "${@:-.}"; }
  explorer() { explorer.exe "$(__vibex_winpath "${1:-.}")"; return 0; }
  notepad() { if [ $# -gt 0 ]; then notepad.exe "$(__vibex_winpath "$1")"; else notepad.exe; fi; }
  pbcopy() { clip.exe; }
  pbpaste() { powershell.exe -NoProfile -Command Get-Clipboard | tr -d '\r'; }

  # cmd's file commands, switches and all
  __vibex_noswitch() { local a; for a in "$@"; do case "$a" in /[A-Za-z]|/[A-Za-z]:*|/[-A-Za-z][A-Za-z]) ;; *) printf '%s\0' "$a" ;; esac; done; }
  __vibex_has() { local want="$1" a; shift; for a in "$@"; do case "${a,,}" in "$want") return 0 ;; esac; done; return 1; }
  copy() { local -a f; mapfile -d '' f < <(__vibex_noswitch "$@"); cp -i "${f[@]}"; }
  move() { local -a f; mapfile -d '' f < <(__vibex_noswitch "$@"); mv -i "${f[@]}"; }
  ren() { mv -i "$@"; }
  del() {
    local -a f; mapfile -d '' f < <(__vibex_noswitch "$@")
    if __vibex_has /q "$@"; then rm -f "${f[@]}"; else rm -i "${f[@]}"; fi
  }
  erase() { del "$@"; }
  md() { mkdir -p "$@"; }
  rd() {
    local -a f; mapfile -d '' f < <(__vibex_noswitch "$@")
    if __vibex_has /s "$@"; then
      if __vibex_has /q "$@"; then rm -rf "${f[@]}"; else rm -rI "${f[@]}"; fi
    else rmdir "${f[@]}"; fi
  }
  # findstr /i /s /n /v "text" files  ->  grep
  findstr() {
    local -a g=(--color=auto) f; local a
    for a in "$@"; do
      case "${a,,}" in
        /i) g+=(-i) ;; /s) g+=(-r) ;; /n) g+=(-n) ;; /v) g+=(-v) ;; /c) g+=(-c) ;; /m) g+=(-l) ;;
        /r) g+=(-E) ;; /l) g+=(-F) ;; /x) g+=(-x) ;; /b) ;; /e) ;;
        *) f+=("$a") ;;
      esac
    done
    grep "${g[@]}" "${f[@]}"
  }
  # systeminfo, tasklist, taskkill, netsh, winget, ipconfig ... run through
  # command_not_found_handle above
fi
