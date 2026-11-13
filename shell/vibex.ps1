# Vibex shell integration for PowerShell (5.1 and 7).
#
# Adds OSC 133 prompt marks (prompt jumping, green/red exit marks in the
# gutter, command blocks: copy / rerun the last command, sticky command
# header, "long command finished" notifications) and OSC 9;9
# working-directory reports (new tabs and splits open where you are).
# Only active inside Vibex.
#
# Use it from your $PROFILE:
#   . "C:\path\to\Vibex-terminal\shell\vibex.ps1"

if ($env:TERM_PROGRAM -ne 'Vibex' -or $global:__VibexIntegrated) { return }
$global:__VibexIntegrated = $true
$global:__VibexLastHistoryId = -1

# config.prompt = "dragon": Kali Linux's two-line prompt, colour for colour
# (its .bashrc: green lines, bold blue user@host and $, bold path; light
# blue lines, bold red names and # as Administrator), and the window title
# user@host: path. Characters are built from codes, so Windows PowerShell
# reads this file the same with or without a BOM.
if ($env:VIBEX_PROMPT -eq 'dragon') {
    function global:__VibexDragonPrompt {
        $e = [char]27
        $admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
        $line = if ($admin) { "$e[;94m" } else { "$e[;32m" }
        $info = if ($admin) { "$e[1;31m" } else { "$e[1;34m" }
        $sym = if ($admin) { '#' } else { '$' }
        $user = $env:USERNAME
        $hn = [System.Net.Dns]::GetHostName()
        # the path: ~ for home; six or more parts become first\…\last four
        $loc = $executionContext.SessionState.Path.CurrentLocation
        $path = if ($loc.Provider.Name -eq 'FileSystem') { $loc.ProviderPath } else { $loc.Path }
        $home_ = if ($env:USERPROFILE) { $env:USERPROFILE } else { [Environment]::GetFolderPath('UserProfile') }
        if ($path -eq $home_) { $path = '~' }
        elseif ($path.StartsWith($home_ + '\', [StringComparison]::OrdinalIgnoreCase)) { $path = '~' + $path.Substring($home_.Length) }
        $parts = @($path -split '\\' | Where-Object { $_ -ne '' })
        if ($parts.Count -ge 6) { $path = $parts[0] + '\' + [char]0x2026 + '\' + (($parts | Select-Object -Last 4) -join '\') }
        $h = [char]0x2500
        $venv = if ($env:VIRTUAL_ENV) { "($e[0;1m$(Split-Path $env:VIRTUAL_ENV -Leaf)$line)" } else { '' }
        $title = "$e]0;$user@${hn}: $path$([char]7)"
        "$title$line$([char]0x250C)$h$h$venv($info$user$([char]0x327F)$hn$e[0m$line)-[$e[0;1m$path$e[0m$line]`n$line$([char]0x2514)$h$info$sym$e[0m "
    }
    $function:prompt = $function:__VibexDragonPrompt
}
$global:__VibexOriginalPrompt = $function:prompt

function global:prompt {
    # capture success state before anything else runs
    $ok = $?
    $code = $global:LASTEXITCODE
    $esc = [char]27
    $bel = [char]7
    $out = ''

    # 133;D — the previous command finished (only if one actually ran)
    $last = Get-History -Count 1
    if ($last -and $last.Id -ne $global:__VibexLastHistoryId) {
        $global:__VibexLastHistoryId = $last.Id
        $exit = if ($ok) { 0 } elseif ($code) { $code } else { 1 }
        $out += "$esc]133;D;$exit$bel"
    }

    # Kali's NEWLINE_BEFORE_PROMPT: a blank line before every prompt but
    # the first (outside the prompt's own marks)
    if ($env:VIBEX_PROMPT -eq 'dragon') {
        if ($global:__VibexPromptShown) { $out += "`n" }
        $global:__VibexPromptShown = $true
    }

    # 9;9 — current directory (filesystem locations only)
    $loc = $executionContext.SessionState.Path.CurrentLocation
    if ($loc.Provider.Name -eq 'FileSystem') {
        $out += "$esc]9;9;`"$($loc.ProviderPath)`"$bel"
    }

    # 133;A — a new prompt starts here; 133;B — typing starts after it
    $out += "$esc]133;A$bel"
    $out + (& $global:__VibexOriginalPrompt) + "$esc]133;B$bel"
}

# 633;E + 133;C — when Enter is pressed: the exact command line, then
# "output starts here". Needs PSReadLine (on by default in PowerShell).
if (Test-Path function:PSConsoleHostReadLine) {
    $global:__VibexOriginalReadLine = $function:PSConsoleHostReadLine
    function global:PSConsoleHostReadLine {
        $line = & $global:__VibexOriginalReadLine
        $esc = [char]27
        $bel = [char]7
        # VS Code's escaping: backslash, ';' and control characters as \xHH
        $sb = [System.Text.StringBuilder]::new()
        foreach ($ch in [string]$line -split '') {
            if ($ch -eq '') { continue }
            $c = [int][char]$ch
            if ($ch -eq '\') { [void]$sb.Append('\\') }
            elseif ($ch -eq ';' -or $c -lt 32) { [void]$sb.Append(('\x{0:x2}' -f $c)) }
            else { [void]$sb.Append($ch) }
        }
        [Console]::Write("$esc]633;E;$($sb.ToString())$bel$esc]133;C$bel")
        # Linux one-liners (a && b, 2>/dev/null) made PowerShell: vibex-commands.ps1
        if ($global:__VibexRewrite) { try { $line = & $global:__VibexRewrite $line } catch { } }
        $line
    }
}

# vshow [--width N|--height N|--fit|--scale 50%|--px N|--actual|--center] FILE...:
# images and Markdown drawn in the terminal (vibex show).
# Start-Process -Wait keeps the prompt from coming back before it's done.
if ($env:VIBEX_EXE -and (Test-Path $env:VIBEX_EXE)) {
    function global:vshow {
        $width = try { $Host.UI.RawUI.WindowSize.Width } catch { 100 }
        $height = try { $Host.UI.RawUI.WindowSize.Height } catch { 30 }
        $files = $args | ForEach-Object { $rp = Resolve-Path -LiteralPath $_ -ErrorAction SilentlyContinue; if ($rp) { [char]34 + $rp.Path + [char]34 } else { [char]34 + $_ + [char]34 } }
        Start-Process -FilePath $env:VIBEX_EXE -ArgumentList (@('show', '--cols', $width, '--rows', $height) + $files) -NoNewWindow -Wait
    }
}

# Linux commands next to the PowerShell ones (ls -la, grep, rm -rf, which,
# ps aux ...): vibex-commands.ps1. Off with config.linux_commands = false.
$__vibexCmdFile = Join-Path $PSScriptRoot 'vibex-commands.ps1'
if ($env:VIBEX_COMMANDS -ne '0' -and (Test-Path -LiteralPath $__vibexCmdFile)) { . $__vibexCmdFile }
Remove-Variable __vibexCmdFile -ErrorAction SilentlyContinue
