# Vibex shell integration for bash (WSL / Linux / Git Bash).
#
# Adds OSC 133 prompt marks and OSC 7 working-directory reports. Only
# active inside Vibex (Vibex forwards TERM_PROGRAM into WSL via WSLENV).
#
# Use it from ~/.bashrc:
#   [ -f /mnt/c/path/to/Vibex-terminal/shell/vibex.bash ] && . /mnt/c/path/to/Vibex-terminal/shell/vibex.bash
#
# Safe with `set -u` / `set -e`, and to source more than once. Works next
# to starship, bash-preexec and anything else in PROMPT_COMMAND: the hook
# runs first (so it sees the command's $? and hands it on unchanged), the
# marks go back on PS1 at every prompt (prompts that rebuild PS1 each time
# keep them), and bash-preexec's __bp_interactive_mode stays last.

if [ "${TERM_PROGRAM:-}" = "Vibex" ]; then
  __VIBEX_INTEGRATED=1

  # first in PROMPT_COMMAND: the exit mark and the folder
  __vibex_prompt() {
    local ec=$?
    printf '\e]133;D;%s\a' "$ec"
    __vibex_cwd
    __vibex_hook
    # hand $? on to the rest of PROMPT_COMMAND (not under set -e, where a
    # non-zero return would end the shell)
    [[ $- == *e* ]] || return "$ec"
  }

  # OSC 7 file://host/path, percent-encoded where it has to be (% ; space
  # and control characters would break the URL or the escape). Git Bash's
  # /c/Users is sent as /C:/Users, which Vibex reads as C:\Users.
  __vibex_cwd() {
    local p="$PWD" d
    case "${OSTYPE:-}" in
      msys* | cygwin*)
        case "$p" in
          /[a-zA-Z]) d="${p:1:1}"; p="/${d^^}:/" ;;
          /[a-zA-Z]/*) d="${p:1:1}"; p="/${d^^}:${p:2}" ;;
          /cygdrive/[a-zA-Z] | /cygdrive/[a-zA-Z]/*) d="${p:10:1}"; p="/${d^^}:${p:11}" ;;
          *) if command -v cygpath >/dev/null 2>&1; then p="/$(cygpath -m "$p")"; fi ;;
        esac
        ;;
    esac
    case "$p" in
      *[%\;\ ]* | *[[:cntrl:]]*)
        local LC_ALL=C i c out=
        for ((i = 0; i < ${#p}; i++)); do
          c="${p:i:1}"
          case "$c" in %|\;|' '|[[:cntrl:]]) printf -v c '%%%02X' "'$c" ;; esac
          out+="$c"
        done
        p="$out"
        ;;
    esac
    printf '\e]7;file://%s%s\a' "${HOSTNAME:-localhost}" "$p"
  }

  # last in PROMPT_COMMAND: the dragon gap, and the marks around PS1 / in
  # PS0 when something replaced them
  __vibex_ps1() {
    local ec=$?
    if [ "${VIBEX_PROMPT:-}" = dragon ]; then
      if [ -n "${__VIBEX_PROMPT_SHOWN:-}" ]; then echo; fi
      __VIBEX_PROMPT_SHOWN=1
    fi
    # \[ \] keep bash's line-length maths right
    case "${PS1:-}" in *'133;A'*) ;; *) PS1='\[\e]133;A\a\]'"${PS1:-}"'\[\e]133;B\a\]' ;; esac
    # PS0 prints when a command starts, so output begins there
    case "${PS0:-}" in *'133;C'*) ;; *) PS0='\e]133;C\a'"${PS0:-}" ;; esac
    [[ $- == *e* ]] || return "$ec"
  }

  # Keep PROMPT_COMMAND as: __vibex_prompt, whatever else is there,
  # __vibex_ps1 (before bash-preexec's __bp_interactive_mode). Idempotent;
  # runs at every prompt, so tools set up after this file get moved too.
  __vibex_hook() {
    local e
    if [ -n "${PROMPT_COMMAND+set}" ] && [ "${#PROMPT_COMMAND[@]}" -gt 1 ]; then
      # bash 5.1+ array form
      local -a keep=() tail=()
      for e in "${PROMPT_COMMAND[@]}"; do
        case "$e" in
          __vibex_prompt | __vibex_ps1 | __vibex_gap) ;;
          __bp_interactive_mode) tail=(__bp_interactive_mode) ;;
          *) keep+=("$e") ;;
        esac
      done
      PROMPT_COMMAND=(__vibex_prompt "${keep[@]}" __vibex_ps1 "${tail[@]}")
      return 0
    fi
    local pc="${PROMPT_COMMAND:-}" tail=''
    local nl=$'\n'
    for e in __vibex_prompt __vibex_ps1 __vibex_gap; do
      pc="${pc//"$e;"/}"; pc="${pc//";$e"/}"; pc="${pc//"$e$nl"/}"; pc="${pc//"$nl$e"/}"; pc="${pc//"$e"/}"
    done
    # no leading / trailing ';' or blank lines (`;` alone is a syntax error)
    while :; do case "$pc" in [[:space:]]* | ';'*) pc="${pc#?}" ;; *) break ;; esac; done
    while :; do case "$pc" in *[[:space:]] | *';') pc="${pc%?}" ;; *) break ;; esac; done
    case "$pc" in
      *__bp_interactive_mode)
        pc="${pc%__bp_interactive_mode}"; tail="${nl}__bp_interactive_mode"
        while :; do case "$pc" in *[[:space:]] | *';') pc="${pc%?}" ;; *) break ;; esac; done
        ;;
    esac
    PROMPT_COMMAND="__vibex_prompt${pc:+$nl$pc}${nl}__vibex_ps1$tail"
  }

  # config.prompt = "dragon": Kali Linux's two-line prompt (its .bashrc,
  # colour for colour), the title user@host: dir, and a blank line before
  # every prompt but the first
  if [ "${VIBEX_PROMPT:-}" = dragon ]; then
    __vibex_pc='\[\033[;32m\]'
    __vibex_ic='\[\033[1;34m\]'
    if [ "${EUID:-1}" -eq 0 ]; then
      __vibex_pc='\[\033[;94m\]'
      __vibex_ic='\[\033[1;31m\]'
    fi
    VIRTUAL_ENV_DISABLE_PROMPT=1
    PS1='\[\e]0;\u@\h: \w\a\]'$__vibex_pc'┌──${VIRTUAL_ENV:+(\[\033[0;1m\]$(basename "$VIRTUAL_ENV")'$__vibex_pc')}('$__vibex_ic'\u㉿\h'$__vibex_pc')-[\[\033[0;1m\]\w'$__vibex_pc']\n'$__vibex_pc'└─'$__vibex_ic'\$\[\033[0m\] '
    unset __vibex_pc __vibex_ic
  fi

  __vibex_hook
  __vibex_ps1 || :
fi

# vshow FILE...: images and Markdown drawn in the terminal (vibex show).
# Like vibex.ps1, it runs the console launcher vibex.com next to vibex.exe
# when there is one.
if [ -n "${VIBEX_EXE:-}" ] && [ -e "$VIBEX_EXE" ]; then
  vshow() {
    local args=() f exe="$VIBEX_EXE"
    [ -e "${VIBEX_EXE%.*}.com" ] && exe="${VIBEX_EXE%.*}.com"
    for f in "$@"; do
      if command -v wslpath >/dev/null 2>&1 && [ -e "$f" ]; then args+=("$(wslpath -w "$f")"); else args+=("$f"); fi
    done
    "$exe" show --cols "${COLUMNS:-$(tput cols 2>/dev/null || echo 100)}" --rows "${LINES:-$(tput lines 2>/dev/null || echo 30)}" "${args[@]}"
  }
fi

# Windows commands next to the Linux ones (WSL and Git Bash): where, cls,
# ipconfig, tasklist, start, explorer, clip, copy/move/del/md/rd, findstr,
# and any Windows program without typing .exe. Off with
# config.linux_commands = false (Vibex passes VIBEX_COMMANDS=0).
if [ "${TERM_PROGRAM:-}" = "Vibex" ] && [ "${VIBEX_COMMANDS:-1}" != 0 ] && [ -z "${__VIBEX_COMMANDS:-}" ]; then
  __VIBEX_COMMANDS=1

  # a Windows path for a file here (WSL: wslpath, Git Bash: cygpath)
  __vibex_winpath() {
    if command -v wslpath >/dev/null 2>&1; then wslpath -w "$1" 2>/dev/null || printf '%s' "$1"
    elif command -v cygpath >/dev/null 2>&1; then cygpath -w "$1"
    else printf '%s' "$1"; fi
  }
  # cmd.exe complains when started in a \\wsl$ (or other UNC) folder: run
  # it from C: then. In a Windows folder it stays put.
  __vibex_cmd() {
    if [ -d /mnt/c ]; then
      case "$PWD" in
        /mnt/[a-zA-Z] | /mnt/[a-zA-Z]/*) cmd.exe /d /c "$@" ;;
        *) (cd /mnt/c && cmd.exe /d /c "$@") ;;
      esac
    else
      case "$PWD" in
        //*) (cd / && cmd.exe //d //c "$@") ;;
        *) cmd.exe //d //c "$@" ;;
      esac
    fi
  }
  # an argument for cmd.exe: & | < > ^ escaped when it goes unquoted (no
  # spaces: WSL and MSYS only quote arguments with spaces)
  __vibex_cmdarg() {
    local s="$1"
    case "$s" in *[[:space:]]*) printf '%s' "$s"; return ;; esac
    s="${s//^/^^}"; s="${s//&/^&}"; s="${s//|/^|}"; s="${s//</^<}"; s="${s//>/^>}"
    printf '%s' "$s"
  }

  # Git Bash rewrites arguments that look like POSIX paths before a
  # Windows program sees them: `ipconfig /all` arrives as
  # "C:/Program Files/Git/all", `taskkill /f` as F:/. These take their
  # switches as typed.
  if [ -n "${MSYSTEM:-}" ]; then
    for __vibex_w in ipconfig tasklist taskkill netsh systeminfo sc schtasks robocopy xcopy icacls whoami shutdown reg wmic driverquery attrib chkdsk sfc powercfg bcdedit; do
      if command -v "$__vibex_w.exe" >/dev/null 2>&1; then
        eval "$__vibex_w() { MSYS_NO_PATHCONV=1 MSYS2_ARG_CONV_EXCL='*' command $__vibex_w.exe \"\$@\"; }"
      fi
    done
    unset __vibex_w
  fi

  # ipconfig, tasklist, notepad, winget ... without .exe (then whatever
  # was there before: Ubuntu's "did you mean" package hints)
  if declare -F command_not_found_handle >/dev/null; then
    case "$(declare -f command_not_found_handle)" in
      *__vibex_cnf_before*) ;;
      *) eval "__vibex_cnf_before() $(declare -f command_not_found_handle | tail -n +2)" ;;
    esac
  fi
  command_not_found_handle() {
    if [[ "$1" != */* ]] && command -v "$1.exe" >/dev/null 2>&1; then
      local c="$1.exe"; shift; "$c" "$@"; return $?
    fi
    if declare -F __vibex_cnf_before >/dev/null; then __vibex_cnf_before "$@"; return $?; fi
    printf 'bash: %s: command not found\n' "$1" >&2
    return 127
  }

  # where NAME: everything bash would run for it, plus Windows programs
  # (Windows' PATH often lists a folder twice, and Git Bash finds X and
  # X.exe: each program once)
  where() {
    local out
    out="$(__vibex_where "$@")"
    [ -n "$out" ] || return 1
    printf '%s\n' "$out" | awk '{ k = tolower($0); sub(/\.exe$/, "", k) } !seen[k]++'
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
      __vibex_cmd start "" "$(__vibex_cmdarg "$t")"
    done
  }
  command -v open >/dev/null 2>&1 || open() { start "${@:-.}"; }
  explorer() { explorer.exe "$(__vibex_winpath "${1:-.}")"; return 0; }
  notepad() { if [ $# -gt 0 ]; then notepad.exe "$(__vibex_winpath "$1")"; else notepad.exe; fi; }
  # clip.exe reads UTF-16 with a BOM correctly (plain bytes as the OEM code page)
  pbcopy() { if command -v iconv >/dev/null 2>&1; then { printf '\377\376'; iconv -f UTF-8 -t UTF-16LE; } | clip.exe; else clip.exe; fi; }
  pbpaste() { powershell.exe -NoProfile -Command Get-Clipboard | tr -d '\r'; }

  # cmd's file commands, switches and all
  __vibex_noswitch() { local a; for a in "$@"; do case "$a" in /[A-Za-z]|/[A-Za-z]:*|/[-A-Za-z][A-Za-z]) ;; *) printf '%s\0' "$a" ;; esac; done; }
  __vibex_has() { local want="$1" a; shift; for a in "$@"; do case "${a,,}" in "$want") return 0 ;; esac; done; return 1; }
  # del / rd on a drive root, /, your home folder, . or .. : refused (GNU
  # rm only guards / itself; Git Bash's /c and WSL's /mnt/c are whole drives)
  __vibex_refuse() {
    local p="$1" q r h
    q="$p"
    while [ "${#q}" -gt 1 ] && [[ "$q" == *[\\/] ]]; do q="${q%?}"; done
    case "$q" in
      . | .. | */. | */.. | *\\. | *\\..) printf '%s\n' "refusing to remove '.' or '..': '$p'" >&2; return 0 ;;
      [A-Za-z]: | / | \\) printf '%s\n' "refusing to remove '$p': a drive root" >&2; return 0 ;;
    esac
    r="$(realpath -- "$p" 2>/dev/null)" || r="$q"
    h="$(realpath -- "${HOME:-/nonexistent}" 2>/dev/null)" || h="${HOME:-}"
    case "$r" in
      / | // | /[a-zA-Z] | /mnt/[a-zA-Z] | /cygdrive/[a-zA-Z]) printf '%s\n' "refusing to remove '$p': a drive root" >&2; return 0 ;;
    esac
    if [ -n "$h" ] && [ "$r" = "$h" ]; then printf '%s\n' "refusing to remove '$p': your home folder" >&2; return 0; fi
    return 1
  }
  # (no operand left after the switches, as in `rd /s /q /c` in Git Bash,
  # where /c is both a switch and the C: drive: nothing is removed)
  __vibex_guard() {
    local a
    if [ $# -eq 0 ]; then printf '%s\n' "nothing to remove (a /x argument is taken as a switch)" >&2; return 0; fi
    for a in "$@"; do if __vibex_refuse "$a"; then return 0; fi; done; return 1
  }
  copy() { local -a f; mapfile -d '' f < <(__vibex_noswitch "$@"); if __vibex_has /y "$@"; then cp -f "${f[@]}"; else cp -i "${f[@]}"; fi; }
  move() { local -a f; mapfile -d '' f < <(__vibex_noswitch "$@"); if __vibex_has /y "$@"; then mv -f "${f[@]}"; else mv -i "${f[@]}"; fi; }
  ren() { mv -i "$@"; }
  del() {
    local -a f; mapfile -d '' f < <(__vibex_noswitch "$@")
    if __vibex_guard "${f[@]}"; then return 1; fi
    if __vibex_has /q "$@"; then rm -f "${f[@]}"; else rm -i "${f[@]}"; fi
  }
  erase() { del "$@"; }
  md() { mkdir -p "$@"; }
  rd() {
    local -a f; mapfile -d '' f < <(__vibex_noswitch "$@")
    if __vibex_guard "${f[@]}"; then return 1; fi
    if __vibex_has /s "$@"; then
      if __vibex_has /q "$@"; then rm -rf "${f[@]}"; else rm -rI "${f[@]}"; fi
    else rmdir "${f[@]}"; fi
  }
  # findstr /i /s /n /v "text" files, findstr /c:"two words" file  ->  grep
  findstr() {
    local -a g=(--color=auto) f=(); local a
    for a in "$@"; do
      case "$a" in
        /[cC]:*) g+=(-F -e "${a:3}"); continue ;;
        /[gG]:*) g+=(-f "${a:3}"); continue ;;
      esac
      case "${a,,}" in
        /i) g+=(-i) ;; /s) g+=(-r) ;; /n) g+=(-n) ;; /v) g+=(-v) ;; /c) g+=(-c) ;; /m) g+=(-l) ;;
        /r) g+=(-E) ;; /l) g+=(-F) ;; /x) g+=(-x) ;; /b) ;; /e) ;;
        *) f+=("$a") ;;
      esac
    done
    grep "${g[@]}" "${f[@]}"
  }
  # systeminfo, tasklist, taskkill, netsh, winget, ipconfig ... run through
  # command_not_found_handle above (WSL) or the wrappers (Git Bash)
fi
