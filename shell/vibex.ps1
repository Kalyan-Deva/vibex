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
