# Vibex shell integration for PowerShell (5.1 and 7).
#
# Adds OSC 133 prompt marks (prompt jumping, green/red exit marks in the
# gutter, command blocks: copy / rerun the last command, sticky command
# header, "long command finished" notifications) and OSC 9;9 / OSC 7
# working-directory reports (new tabs and splits open where you are).
# Only active inside Vibex.
#
# Use it from your $PROFILE:
#   . "C:\path\to\Vibex-terminal\shell\vibex.ps1"
#
# (This file is plain ASCII on purpose: Windows PowerShell reads a file
# without a BOM in the ANSI code page. Non-ASCII characters are built from
# their codes.)

# (PSVariable.GetValue, not $global:__VibexIntegrated: a profile with
# Set-StrictMode would make reading an unset variable an error)
if ($env:TERM_PROGRAM -ne 'Vibex' -or $ExecutionContext.SessionState.PSVariable.GetValue('__VibexIntegrated')) { return }
$global:__VibexIntegrated = $true
$global:__VibexLastHistoryId = -1
$global:__VibexRan = $false        # a command line was entered (set by the PSReadLine wrapper)
$global:__VibexFail = $null        # an emulated Linux command failed as the line's last statement
$global:__VibexPromptShown = $false
$global:__VibexPrevCode = $ExecutionContext.SessionState.PSVariable.GetValue('LASTEXITCODE')

# config.prompt = "dragon": Kali Linux's two-line prompt, colour for colour
# (its .bashrc: green lines, bold blue user@host and $, bold path; light
# blue lines, bold red names and # as Administrator), and the window title
# user@host: path.
if ($env:VIBEX_PROMPT -eq 'dragon') {
    # neither changes during a session: worked out once, not every prompt
    $global:__VibexAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    $global:__VibexHostName = if ($env:VIBEX_HOSTNAME) { $env:VIBEX_HOSTNAME } else { [System.Net.Dns]::GetHostName() }

    # the path: ~ for home; six or more parts become first\...\last four;
    # a UNC path keeps its \\server\share; registry / Env: paths as they are
    function global:__VibexDragonPath([string]$path) {
        $path = $path -replace '^Microsoft\.PowerShell\.Core\\', ''
        $home_ = if ($env:USERPROFILE) { $env:USERPROFILE } else { [Environment]::GetFolderPath('UserProfile') }
        if ($path -eq $home_) { return '~' }
        if ($path.StartsWith($home_ + '\', [StringComparison]::OrdinalIgnoreCase)) { $path = '~' + $path.Substring($home_.Length) }
        $dots = [string][char]0x2026
        $m = [regex]::Match($path, '^\\\\[^\\]+\\[^\\]+')
        if ($m.Success) {
            $parts = @($path.Substring($m.Length) -split '\\' | Where-Object { $_ -ne '' })
            if ($parts.Count -ge 5) { return $m.Value + '\' + $dots + '\' + (($parts | Select-Object -Last 4) -join '\') }
            return $path
        }
        $parts = @($path -split '\\' | Where-Object { $_ -ne '' })
        if ($parts.Count -ge 6) { return $parts[0] + '\' + $dots + '\' + (($parts | Select-Object -Last 4) -join '\') }
        $path
    }

    function global:__VibexDragonPrompt {
        $e = [char]27
        $admin = $global:__VibexAdmin
        $line = if ($admin) { "$e[;94m" } else { "$e[;32m" }
        $info = if ($admin) { "$e[1;31m" } else { "$e[1;34m" }
        $sym = if ($admin) { '#' } else { '$' }
        $user = $env:USERNAME
        $hn = $global:__VibexHostName
        $loc = $executionContext.SessionState.Path.CurrentLocation
        $path = __VibexDragonPath $(if ($loc.Provider.Name -eq 'FileSystem') { $loc.ProviderPath } else { $loc.Path })
        $h = [char]0x2500
        $venv = if ($env:VIRTUAL_ENV) { "($e[0;1m$([IO.Path]::GetFileName($env:VIRTUAL_ENV.TrimEnd('\', '/')))$line)" } else { '' }
        $title = "$e]0;$user@${hn}: $path$([char]7)"
        "$title$line$([char]0x250C)$h$h$venv($info$user$([char]0x327F)$hn$e[0m$line)-[$e[0;1m$path$e[0m$line]`n$line$([char]0x2514)$h$info$sym$e[0m "
    }
    $function:prompt = $function:__VibexDragonPrompt
}
$global:__VibexOriginalPrompt = $function:prompt

function global:prompt {
    # capture success state before anything else runs
    $ok = $global:?
    Set-StrictMode -Off
    $code = $global:LASTEXITCODE
    $esc = [char]27
    $bel = [char]7
    $out = ''

    # 133;D: the previous command finished. A command ran when the
    # PSReadLine wrapper saw a line entered, or (without PSReadLine) when
    # history grew. Ctrl+C keeps a command out of history, so "entered but
    # not in history and failed" is an interrupted command: exit 130, as in
    # bash. A blank Enter is neither, so no mark.
    $ran = $global:__VibexRan
    $global:__VibexRan = $false
    $vf = $global:__VibexFail
    $global:__VibexFail = $null
    $last = Get-History -Count 1
    $newHist = $last -and $last.Id -ne $global:__VibexLastHistoryId
    if ($last) { $global:__VibexLastHistoryId = $last.Id }
    if ($ran -or $newHist) {
        $exit = if (-not $ok) {
            if ($ran -and -not $newHist -and -not ($code -and $code -ne $global:__VibexPrevCode)) { 130 }
            # a native program's code when it set one now; a stale one from
            # an earlier command doesn't count (Get-Item nope: 1, not the
            # last native exit code)
            elseif ($code -and $code -ne $global:__VibexPrevCode) { $code } else { 1 }
        } elseif ($vf) { $vf } else { 0 }
        $out += "$esc]133;D;$exit$bel"
    }
    $global:__VibexPrevCode = $code

    # Kali's NEWLINE_BEFORE_PROMPT: a blank line before every prompt but
    # the first (outside the prompt's own marks)
    if ($env:VIBEX_PROMPT -eq 'dragon') {
        if ($global:__VibexPromptShown) { $out += "`n" }
        $global:__VibexPromptShown = $true
    }

    # 9;9: current directory (filesystem locations only). OSC parameters
    # are split at ';', so a folder name with one goes as OSC 7's
    # percent-encoded file:// URL instead.
    $loc = $executionContext.SessionState.Path.CurrentLocation
    if ($loc.Provider.Name -eq 'FileSystem') {
        $pp = $loc.ProviderPath
        if ($pp.IndexOf(';') -ge 0 -and $pp -match '^[A-Za-z]:') {
            $url = (($pp -split '\\') | ForEach-Object { [Uri]::EscapeDataString($_) }) -join '/'
            $out += "$esc]7;file://$env:COMPUTERNAME/$url$bel"
        } else {
            $out += "$esc]9;9;`"$pp`"$bel"
        }
    }

    # 133;A: a new prompt starts here; 133;B: typing starts after it. The
    # wrapped prompt (oh-my-posh, starship, your own) gets $? and
    # $LASTEXITCODE as they were, and can't change $LASTEXITCODE for the
    # next command.
    $out += "$esc]133;A$bel"
    $global:LASTEXITCODE = $code
    if (-not $ok) { Write-Error 'x' -ErrorAction Ignore }
    $p = & $global:__VibexOriginalPrompt
    $global:LASTEXITCODE = $code
    $out + $p + "$esc]133;B$bel"
}

# OSC 633;E's escaping (VS Code's): over the UTF-8 bytes, '\' as '\\' and
# ';', control characters and anything not ASCII as \xHH. Plain ASCII
# goes out whatever the console code page is.
function global:__VibexEsc633([string]$s) {
    $sb = [System.Text.StringBuilder]::new($s.Length + 16)
    foreach ($b in [System.Text.Encoding]::UTF8.GetBytes($s)) {
        if ($b -eq 0x5C) { [void]$sb.Append('\\') }
        elseif ($b -lt 0x20 -or $b -ge 0x7F -or $b -eq 0x3B) { [void]$sb.Append('\x').Append($b.ToString('x2')) }
        else { [void]$sb.Append([char]$b) }
    }
    $sb.ToString()
}

# 633;E + 133;C: when Enter is pressed: the exact command line, then
# "output starts here". Needs PSReadLine (on by default in PowerShell);
# loaded here if the profile runs before the host has done it.
if (-not ${function:PSConsoleHostReadLine} -and $Host.Name -eq 'ConsoleHost' -and -not [Console]::IsInputRedirected -and
    -not ([Environment]::GetCommandLineArgs() -match '^-(NonI|Command$|c$|File$|f$)')) {
    Import-Module PSReadLine -ErrorAction SilentlyContinue
}
if (${function:PSConsoleHostReadLine}) {
    $global:__VibexOriginalReadLine = ${function:PSConsoleHostReadLine}
    function global:PSConsoleHostReadLine {
        $line = & $global:__VibexOriginalReadLine
        Set-StrictMode -Off
        # a blank Enter runs nothing: no C here, so no D at the next prompt
        if ([string]::IsNullOrWhiteSpace($line)) { return $line }
        $esc = [char]27
        $bel = [char]7
        [Console]::Write("$esc]633;E;$(__VibexEsc633 $line)$bel$esc]133;C$bel")
        $global:__VibexRan = $true
        # Linux one-liners (a && b, 2>/dev/null) made PowerShell: vibex-commands.ps1
        if ($global:__VibexRewrite) { try { $line = & $global:__VibexRewrite $line } catch { } }
        $line
    }
}

# vshow [--width N|--height N|--fit|--scale 50%|--px N|--actual|--center] FILE...:
# images and Markdown drawn in the terminal (vibex show).
# vibex.exe is a window program, which PowerShell doesn't wait for, so this
# runs vibex.com next to it (a console launcher PowerShell waits on like any
# command). Without one (a development build), it waits on the process
# itself: Start-Process -Wait only checks about once a second, and the
# prompt came back that much late.
if ($env:VIBEX_EXE -and [IO.File]::Exists($env:VIBEX_EXE)) {
    function global:vshow {
        $width = try { $Host.UI.RawUI.WindowSize.Width } catch { 100 }
        $height = try { $Host.UI.RawUI.WindowSize.Height } catch { 30 }
        $files = @($args | ForEach-Object { $rp = Resolve-Path -LiteralPath $_ -ErrorAction SilentlyContinue; if ($rp) { $rp.Path } else { "$_" } })
        $show = @('show', '--cols', $width, '--rows', $height) + $files
        $com = [IO.Path]::ChangeExtension($env:VIBEX_EXE, '.com')
        if (Test-Path -LiteralPath $com) {
            & $com @show
            return
        }
        $quoted = $show | ForEach-Object { [char]34 + ("$_" -replace '"', '\"') + [char]34 }
        $p = Start-Process -FilePath $env:VIBEX_EXE -ArgumentList $quoted -NoNewWindow -PassThru
        $p.WaitForExit()
    }
}

# Linux commands next to the PowerShell ones (ls -la, grep, rm -rf, which,
# ps aux ...): vibex-commands.ps1. Off with config.linux_commands = false.
if ($env:VIBEX_COMMANDS -ne '0' -and [IO.File]::Exists([IO.Path]::Combine($PSScriptRoot, 'vibex-commands.ps1'))) { . ([IO.Path]::Combine($PSScriptRoot, 'vibex-commands.ps1')) }
