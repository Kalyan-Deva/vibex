# Vibex: Linux commands in PowerShell, next to the PowerShell ones.
#
#   ls -la   grep -rn   find . -name   rm -rf   cp -r   mkdir -p   touch
#   which    where      ps aux         kill -9  top     df -h      free -h
#   export   env        lsof -i :3000  ss -tlnp uname -a  sudo  open  ...
#   a && b   a || b     2>/dev/null    (rewritten for PowerShell 5.1)
#
# The real GNU tools come from Git for Windows (usr\bin, found by Vibex
# and passed as VIBEX_GNU); without Git the common ones fall back to
# PowerShell versions. Plain PowerShell use keeps working: `ls` alone is
# still Get-ChildItem, `ls | sort Length`, `Get-Process | where Name -eq x`
# and `kill -Name notepad` do what they always did. Only flags that look
# like Linux ones (`-la`, `-rf`, `--color`) switch to the Linux behaviour.
#
# Loaded by vibex.ps1 inside Vibex. Turn off: Settings -> Terminal ->
# "Linux and Windows commands", or config.linux_commands = false.

if ($env:VIBEX_COMMANDS -eq '0' -or $global:__VibexCommands) { return }
$global:__VibexCommands = $true

# Vibex is a UTF-8 terminal: say so both ways, so piping into and out of
# the GNU tools keeps accents, emoji and box drawing intact.
$__vxUtf8 = New-Object System.Text.UTF8Encoding $false
try { [Console]::OutputEncoding = $__vxUtf8 } catch { }
# (Windows PowerShell also starts every pipe into a program with a BOM
# unless the console's input side is UTF-8 without one)
try { [Console]::InputEncoding = $__vxUtf8 } catch { }
$global:OutputEncoding = $__vxUtf8
Remove-Variable __vxUtf8

$global:__VibexGnu = $null
foreach ($d in @($env:VIBEX_GNU, "$env:ProgramFiles\Git\usr\bin", "${env:ProgramFiles(x86)}\Git\usr\bin", "$env:LOCALAPPDATA\Programs\Git\usr\bin")) {
    if ($d -and (Test-Path -LiteralPath (Join-Path $d 'grep.exe'))) { $global:__VibexGnu = $d; break }
}

function global:__vx_gnu([string]$name) {
    if (-not $global:__VibexGnu) { return $null }
    $p = Join-Path $global:__VibexGnu "$name.exe"
    if (Test-Path -LiteralPath $p) { $p } else { $null }
}

# Does any argument look like a Linux flag rather than one of $cmdlet's
# own parameters? -la, -rf, -1 and --color do; -Force, -Recurse and
# -Filter (or 4+ letter prefixes of them) don't.
function global:__vx_linuxy([string]$cmdlet, [object[]]$argv) {
    $names = $null
    foreach ($a in $argv) {
        if ($a -is [int] -and $a -lt 0) { return $true }
        if ($a -isnot [string]) { continue }
        if ($a -match '^--[A-Za-z]') { return $true }
        if ($a -match '^-[A-Za-z0-9]+$') {
            if ($null -eq $names) {
                $c = Get-Command $cmdlet -ErrorAction SilentlyContinue | Select-Object -First 1
                $names = @(if ($c) { foreach ($p in $c.Parameters.Values) { $p.Name; $p.Aliases } })
            }
            $n = $a.Substring(1)
            if ($names -contains $n) { continue }
            if ($n.Length -ge 4 -and @($names | Where-Object { $_ -like "$n*" }).Count -gt 0) { continue }
            return $true
        }
    }
    $false
}

# Split Linux-style arguments: short flags (case kept, "rf" for -rf), long
# flags without the dashes, values for the options in $takes, the rest.
function global:__vx_flags([object[]]$argv, [string[]]$takes = @()) {
    $r = @{ short = ''; long = @(); value = @{}; rest = @() }
    $done = $false
    for ($i = 0; $i -lt $argv.Count; $i++) {
        $a = $argv[$i]
        if ($a -is [int] -and $a -lt 0) { $a = [string]$a }
        if ($done -or $a -isnot [string]) { $r.rest += $a; continue }
        if ($a -eq '--') { $done = $true; continue }
        if ($a -match '^--([A-Za-z][\w-]*)(=(.*))?$') {
            $r.long += $Matches[1]
            if ($Matches[3]) { $r.value[$Matches[1]] = $Matches[3] }
            elseif ($takes -contains $Matches[1] -and $i + 1 -lt $argv.Count) { $i++; $r.value[$Matches[1]] = [string]$argv[$i] }
            continue
        }
        if ($a -match '^-[A-Za-z0-9]+$') {
            $s = $a.Substring(1)
            $last = $s.Substring($s.Length - 1)
            if ($takes -ccontains $last -and $i + 1 -lt $argv.Count) { $r.short += $s; $i++; $r.value[$last] = [string]$argv[$i]; continue }
            $r.short += $s
            continue
        }
        $r.rest += $a
    }
    $r
}

# Piping into a GNU tool. Windows PowerShell ends every line it sends a
# program with CR LF, which sed, awk and xargs take as part of the text; so
# the lines go through cmd: `tr -d '\r' | TOOL ARGS`, from a small batch
# file (which can quote anything). The tool still writes to the terminal
# itself (colours stay) and `$x = ... | grep y` still captures.
function global:__vx_q([string]$a) {
    if ($a -match '^[\w./:=+,@-]+$') { return $a }
    $o = '"'; $bs = 0
    foreach ($ch in $a.ToCharArray()) {
        if ($ch -eq '\') { $bs++; continue }
        if ($ch -eq '"') { $o += ('\' * ($bs * 2 + 1)) + '"'; $bs = 0; continue }
        $o += ('\' * $bs) + $ch; $bs = 0
    }
    $o + ('\' * ($bs * 2)) + '"'
}
function global:__vx_cmdesc([string]$line) {
    $out = New-Object System.Text.StringBuilder
    $inq = $false
    foreach ($ch in $line.ToCharArray()) {
        if ($ch -eq '"') { $inq = -not $inq; [void]$out.Append($ch); continue }
        if ($ch -eq '%') { [void]$out.Append('%%'); continue }
        if (-not $inq -and '&|<>^()'.Contains([string]$ch)) { [void]$out.Append('^') }
        [void]$out.Append($ch)
    }
    $out.ToString()
}
function global:__vx_pipe($in, [string]$exe, [object[]]$opts, [object[]]$argv, [bool]$withPath = $false) {
    $tr = __vx_gnu tr
    if (-not $tr) { $in | Out-String -Stream | & $exe @opts @argv; return }
    $words = @($exe) + @($opts) + @($argv | ForEach-Object { if ($_ -is [int] -and $_ -lt 0) { [string]$_ } else { [string]$_ } })
    $cmdline = __vx_cmdesc (($words | ForEach-Object { __vx_q $_ }) -join ' ')
    $pre = if ($withPath) { "@set `"PATH=$($global:__VibexGnu);%PATH%`"`r`n" } else { '' }
    $bat = Join-Path ([IO.Path]::GetTempPath()) ("vibex-pipe-{0}.cmd" -f [guid]::NewGuid().ToString('N').Substring(0, 12))
    [IO.File]::WriteAllText($bat, "$pre@`"$tr`" -d `"\r`" | $cmdline`r`n@exit /b %errorlevel%`r`n", (New-Object System.Text.UTF8Encoding $false))
    try { $in | Out-String -Stream | & $env:ComSpec /d /c $bat }
    finally { Remove-Item -LiteralPath $bat -ErrorAction SilentlyContinue }
}

# Aliases PowerShell puts in the way (functions can't beat aliases).
foreach ($a in 'ls', 'dir', 'cat', 'rm', 'del', 'erase', 'rd', 'cp', 'copy', 'mv', 'move', 'sort', 'diff', 'tee', 'kill', 'ps', 'curl', 'wget', 'where', 'cd', 'pwd', 'man', 'set') {
    if (Test-Path "Alias:$a") { Remove-Item "Alias:$a" -Force -ErrorAction SilentlyContinue }
}

# ------------------------------------------------------------ GNU text tools
# Pipe-aware: PowerShell objects are formatted as text first, so
# `Get-Process | grep chrome` and `ls | wc -l` work like on Linux.
$__vxNeedsPath = @('xargs')
$__vxText = [ordered]@{
    grep = @('grep', '--color=auto'); egrep = @('grep', '-E --color=auto'); fgrep = @('grep', '-F --color=auto')
    sed = @('sed', ''); awk = @('awk', ''); gawk = @('gawk', ''); wc = @('wc', ''); uniq = @('uniq', '')
    cut = @('cut', ''); tr = @('tr', ''); xargs = @('xargs', ''); head = @('head', ''); tail = @('tail', '')
    less = @('less', '-R'); nl = @('nl', ''); tac = @('tac', ''); rev = @('rev', ''); column = @('column', '')
    base64 = @('base64', ''); md5sum = @('md5sum', ''); sha1sum = @('sha1sum', ''); sha256sum = @('sha256sum', '')
    vi = @('vim', '')
}
foreach ($k in $__vxText.Keys) {
    $exe = __vx_gnu $__vxText[$k][0]
    if (-not $exe) { continue }
    $o = $__vxText[$k][1]
    $optList = ($o -split ' ' | Where-Object { $_ } | ForEach-Object { "'$_'" }) -join ', '
    $np = if ($__vxNeedsPath -contains $k) { '$true' } else { '$false' }
    $run = "`$o = @($optList)`n" +
        "if (`$MyInvocation.ExpectingInput) { __vx_pipe `$input `$exe `$o `$args $np; return }`n"
    if ($__vxNeedsPath -contains $k) { $run += '$old = $env:PATH; $env:PATH = $global:__VibexGnu + '';'' + $old; try { & $exe @o @args } finally { $env:PATH = $old }' }
    else { $run += '& $exe @o @args' }
    $body = "`$exe = '$($exe.Replace("'", "''"))'`n" + $run
    Set-Item -Path "function:global:$k" -Value ([scriptblock]::Create($body))
}
Remove-Variable __vxText, __vxNeedsPath, k, exe, o, optList, np, run, body -ErrorAction SilentlyContinue

# Without Git: PowerShell versions of the everyday ones.
if (-not (__vx_gnu grep)) {
    function global:grep {
        $f = __vx_flags $args @('e', 'm')
        $pat = if ($f.value.ContainsKey('e')) { $f.value['e'] } else { $f.rest[0]; $f.rest = @($f.rest | Select-Object -Skip 1) }
        if ($null -eq $pat) { Write-Error 'usage: grep [-inrvlcwF] PATTERN [FILE...]'; return }
        $p = @{ Pattern = $pat; CaseSensitive = -not $f.short.Contains('i'); NotMatch = $f.short.Contains('v') }
        if ($f.short.Contains('F')) { $p.SimpleMatch = $true }
        if ($f.short.Contains('w') -and -not $p.SimpleMatch) { $p.Pattern = "\b(?:$pat)\b" }
        $hits = if ($f.rest.Count -gt 0) {
            $files = if ($f.short -cmatch '[rR]') { Get-ChildItem -Path $f.rest -Recurse -File -ErrorAction SilentlyContinue } else { Get-Item -Path $f.rest -ErrorAction SilentlyContinue }
            $files | Select-String @p
        } else { $input | Out-String -Stream | Select-String @p }
        $many = $f.rest.Count -gt 1 -or $f.short -cmatch '[rR]'
        if ($f.short.Contains('l')) { $hits | Select-Object -ExpandProperty Path -Unique; return }
        if ($f.short.Contains('c')) { @($hits).Count; return }
        foreach ($h in $hits) {
            $pre = ''
            if ($many -and $h.Path -ne 'InputStream') { $pre += (Resolve-Path -Relative $h.Path) + ':' }
            if ($f.short.Contains('n')) { $pre += "$($h.LineNumber):" }
            "$pre$($h.Line)"
        }
        $global:LASTEXITCODE = if (@($hits).Count) { 0 } else { 1 }
    }
    function global:head {
        $f = __vx_flags $args @('n')
        $n = if ($f.value.ContainsKey('n')) { [int]$f.value['n'] } elseif ($f.short -match '^\d+$') { [int]$f.short } else { 10 }
        if ($f.rest.Count) { Get-Content -Path $f.rest -TotalCount $n } else { $input | Out-String -Stream | Select-Object -First $n }
    }
    function global:tail {
        $f = __vx_flags $args @('n')
        $n = if ($f.value.ContainsKey('n')) { [int]$f.value['n'] } elseif ($f.short -match '^\d+') { [int]($f.short -replace '\D', '') } else { 10 }
        if ($f.rest.Count) { Get-Content -Path $f.rest -Tail $n -Wait:($f.short.Contains('f')) } else { $input | Out-String -Stream | Select-Object -Last $n }
    }
    function global:wc {
        $f = __vx_flags $args
        $text = if ($f.rest.Count) { Get-Content -Path $f.rest -Raw } else { ($input | Out-String) }
        $lines = ([regex]::Matches($text, "`n")).Count
        $words = @($text -split '\s+' | Where-Object { $_ }).Count
        $bytes = [Text.Encoding]::UTF8.GetByteCount($text)
        if ($f.short.Contains('l')) { $lines } elseif ($f.short.Contains('w')) { $words } elseif ($f.short -match '[cm]') { $bytes } else { '{0,7} {1,7} {2,7}' -f $lines, $words, $bytes }
    }
}

# ------------------------------------------------------------ files

function global:ls {
    $gnu = __vx_gnu ls
    # `ls -r` alone is PowerShell's -Recurse (what PowerShell hands type it for)
    $onlyR = @($args | Where-Object { $_ -is [string] -and $_ -match '^-' }) -join ' ' -ceq '-r'
    if ($gnu -and -not $onlyR -and -not $MyInvocation.ExpectingInput -and (__vx_linuxy Get-ChildItem $args)) { & $gnu --color=auto @args; return }
    # the Dragon (Kali) style: plain `ls` and `ls folder` are Linux's too,
    # unless a PowerShell parameter is given
    $psParam = @($args | Where-Object { $_ -is [string] -and $_ -match '^-[A-Za-z]{2,}' }).Count -gt 0
    if ($gnu -and $env:VIBEX_PROMPT -eq 'dragon' -and -not $MyInvocation.ExpectingInput -and -not $psParam -and -not $onlyR) { & $gnu --color=auto @args; return }
    if ($MyInvocation.ExpectingInput) { $input | Get-ChildItem @args } else { Get-ChildItem @args }
}

# Kali sorts names the way its en_US.UTF-8 locale does (case doesn't come
# first: bugbounty before CTF)
if ($env:VIBEX_PROMPT -eq 'dragon' -and -not $env:LC_ALL -and -not $env:LC_COLLATE) { $env:LC_COLLATE = 'en_US.UTF-8' }

# Kali's ls colours: dircolors' database plus its fix for 777 folders
if ($env:VIBEX_PROMPT -eq 'dragon' -and -not $env:LS_COLORS) {
    $dc = __vx_gnu dircolors
    if ($dc) {
        $db = (& $dc -b) -join "`n"
        if ($db -match "LS_COLORS='([^']*)'") { $env:LS_COLORS = $Matches[1].TrimEnd(':') + ':ow=30;44:' }
    }
}
function global:ll { $gnu = __vx_gnu ls; if ($gnu) { & $gnu -lh --color=auto @args } else { Get-ChildItem -Force @args } }
function global:la { $gnu = __vx_gnu ls; if ($gnu) { & $gnu -lAh --color=auto @args } else { Get-ChildItem -Force @args } }

# dir /s /b (cmd's) or dir (PowerShell's)
function global:dir {
    if (@($args | Where-Object { $_ -is [string] -and $_ -match '^/[A-Za-z:-]+' }).Count) { & $env:ComSpec /d /c dir @args; return }
    Get-ChildItem @args
}

function global:cat {
    $gnu = __vx_gnu cat
    if ($gnu -and (__vx_linuxy Get-Content $args)) {
        if ($MyInvocation.ExpectingInput) { __vx_pipe $input $gnu @() $args } else { & $gnu @args }
        return
    }
    Get-Content @args
}

function global:rm {
    if (-not (__vx_linuxy Remove-Item $args)) { Remove-Item @args; return }
    $f = __vx_flags $args
    $recurse = $f.short -cmatch '[rR]' -or $f.long -contains 'recursive'
    $force = $f.short.Contains('f') -or $f.long -contains 'force'
    $verbose = $f.short.Contains('v') -or $f.long -contains 'verbose'
    $ask = $f.short.Contains('i')
    $status = 0
    foreach ($p in $f.rest) {
        $items = @(Get-Item -Path $p -Force -ErrorAction SilentlyContinue)
        if (-not $items.Count) {
            if (-not $force) { Write-Error "rm: cannot remove '$p': No such file or directory"; $status = 1 }
            continue
        }
        foreach ($it in $items) {
            if ($it.PSIsContainer -and -not $recurse -and -not $f.short.Contains('d')) { Write-Error "rm: cannot remove '$p': Is a directory"; $status = 1; continue }
            if ($ask -and (Read-Host "rm: remove '$($it.Name)'? [y/N]") -notmatch '^[yY]') { continue }
            Remove-Item -LiteralPath $it.FullName -Recurse:$recurse -Force -ErrorAction Continue
            if ($verbose) { "removed '$($it.Name)'" }
        }
    }
    $global:LASTEXITCODE = $status
}
foreach ($n in 'del', 'erase', 'rd') {
    # cmd's switches (del /q, rd /s /q) go to cmd; anything else is Remove-Item
    Set-Item "function:global:$n" -Value ([scriptblock]::Create(
        "if (@(`$args | Where-Object { `$_ -is [string] -and `$_ -match '^/[A-Za-z]' }).Count) { & `$env:ComSpec /d /c $n @args } else { Remove-Item @args }"))
}

function global:cp {
    if (-not (__vx_linuxy Copy-Item $args)) { Copy-Item @args; return }
    $f = __vx_flags $args
    if ($f.rest.Count -lt 2) { Write-Error 'usage: cp [-rfvn] SOURCE... DEST'; return }
    $dest = $f.rest[-1]
    $recurse = $f.short -cmatch '[rRa]' -or $f.long -contains 'recursive'
    foreach ($s in $f.rest[0..($f.rest.Count - 2)]) {
        foreach ($it in @(Get-Item -Path $s -Force -ErrorAction SilentlyContinue)) {
            if ($it.PSIsContainer -and -not $recurse) { Write-Error "cp: -r not specified; omitting directory '$s'"; continue }
            $target = if (Test-Path -LiteralPath $dest -PathType Container) { Join-Path $dest $it.Name } else { $dest }
            if ($f.short.Contains('n') -and (Test-Path -LiteralPath $target)) { continue }
            Copy-Item -LiteralPath $it.FullName -Destination $dest -Recurse:$recurse -Force:($f.short.Contains('f')) -ErrorAction Continue
            if ($f.short.Contains('v')) { "'$s' -> '$target'" }
        }
    }
}
function global:copy { if (@($args | Where-Object { $_ -is [string] -and $_ -match '^/[A-Za-z-]' }).Count) { & $env:ComSpec /d /c copy @args } else { Copy-Item @args } }

function global:mv {
    if (-not (__vx_linuxy Move-Item $args)) { Move-Item @args; return }
    $f = __vx_flags $args
    if ($f.rest.Count -lt 2) { Write-Error 'usage: mv [-fvn] SOURCE... DEST'; return }
    $dest = $f.rest[-1]
    foreach ($s in $f.rest[0..($f.rest.Count - 2)]) {
        $target = if (Test-Path -LiteralPath $dest -PathType Container) { Join-Path $dest (Split-Path $s -Leaf) } else { $dest }
        if ($f.short.Contains('n') -and (Test-Path -LiteralPath $target)) { continue }
        Move-Item -Path $s -Destination $dest -Force:($f.short.Contains('f')) -ErrorAction Continue
        if ($f.short.Contains('v')) { "renamed '$s' -> '$target'" }
    }
}
function global:move { if (@($args | Where-Object { $_ -is [string] -and $_ -match '^/[A-Za-z-]' }).Count) { & $env:ComSpec /d /c move @args } else { Move-Item @args } }

function global:mkdir {
    if (-not (__vx_linuxy New-Item $args)) { New-Item -ItemType Directory @args; return }
    $f = __vx_flags $args @('m')
    $parents = $f.short.Contains('p') -or $f.long -contains 'parents'
    foreach ($p in $f.rest) {
        if (Test-Path -LiteralPath $p) {
            if (-not $parents) { Write-Error "mkdir: cannot create directory '$p': File exists" }
            continue
        }
        New-Item -ItemType Directory -Path $p -Force:$parents -ErrorAction Continue | Out-Null
        if ($f.short.Contains('v')) { "mkdir: created directory '$p'" }
    }
}

function global:touch {
    $f = __vx_flags $args
    foreach ($p in $f.rest) {
        if (Test-Path -LiteralPath $p) { (Get-Item -LiteralPath $p -Force).LastWriteTime = Get-Date }
        elseif (-not $f.short.Contains('c')) { New-Item -ItemType File -Path $p -ErrorAction Continue | Out-Null }
    }
}

# ln -s TARGET LINK (a directory falls back to a junction, which needs no
# admin or Developer Mode); ln TARGET LINK is a hard link.
function global:ln {
    $f = __vx_flags $args
    if ($f.rest.Count -lt 2) { Write-Error 'usage: ln [-sf] TARGET LINK'; return }
    $target, $link = $f.rest[0], $f.rest[1]
    if (Test-Path -LiteralPath $link -PathType Container) { $link = Join-Path $link (Split-Path $target -Leaf) }
    if ($f.short.Contains('f') -and (Test-Path -LiteralPath $link)) { Remove-Item -LiteralPath $link -Force }
    if (-not $f.short.Contains('s')) { New-Item -ItemType HardLink -Path $link -Target $target | Out-Null; return }
    try { New-Item -ItemType SymbolicLink -Path $link -Target $target -ErrorAction Stop | Out-Null }
    catch {
        if (Test-Path -LiteralPath $target -PathType Container) { New-Item -ItemType Junction -Path $link -Target (Resolve-Path $target) | Out-Null; "ln: made a junction (symbolic links need Developer Mode or admin)" }
        else { Write-Error "ln: $($_.Exception.Message) (symbolic links need Developer Mode or admin)" }
    }
}

# find . -name '*.rs' (GNU); find "text" file or find /i ... (Windows')
function global:find {
    $argv = @($args)
    $first = $argv | Select-Object -First 1
    $winSwitch = @($argv | Where-Object { $_ -is [string] -and $_ -match '^/[vcniVCNI]$|^/off' }).Count -gt 0
    $winText = $argv.Count -ge 2 -and ($first -is [string]) -and $first -notmatch '^-' -and -not (Test-Path -LiteralPath $first) -and (Test-Path -LiteralPath ([string]$argv[1]))
    if ($winSwitch -or $winText) {
        # find.exe wants the text in quotes: put them back
        $quoted = $false
        $fa = foreach ($a in $argv) { $s = [string]$a; if (-not $quoted -and $s -notmatch '^/') { $quoted = $true; '"' + $s.Trim('"') + '"' } else { $s } }
        & "$env:SystemRoot\System32\find.exe" @fa
        return
    }
    $gnu = __vx_gnu find
    if ($gnu) {
        # -exec grep ... finds the GNU tools too
        $old = $env:PATH; $env:PATH = $global:__VibexGnu + ';' + $old
        try { if ($MyInvocation.ExpectingInput) { __vx_pipe $input $gnu @() $argv $true } else { & $gnu @argv } }
        finally { $env:PATH = $old }
        return
    }
    # without Git: path, -name / -iname, -type f|d, -maxdepth N
    $roots = @(); $name = $null; $ci = $false; $type = $null; $depth = $null
    for ($i = 0; $i -lt $argv.Count; $i++) {
        switch -CaseSensitive ([string]$argv[$i]) {
            '-name' { $i++; $name = [string]$argv[$i] }
            '-iname' { $i++; $name = [string]$argv[$i]; $ci = $true }
            '-type' { $i++; $type = [string]$argv[$i] }
            '-maxdepth' { $i++; $depth = [int]$argv[$i] }
            default { $roots += [string]$argv[$i] }
        }
    }
    if (-not $roots) { $roots = @('.') }
    foreach ($r in $roots) {
        $p = @{ Path = $r; Recurse = $true; Force = $true; ErrorAction = 'SilentlyContinue' }
        if ($null -ne $depth) { $p.Depth = [Math]::Max(0, $depth - 1) }
        if ($type -eq 'f') { $p.File = $true } elseif ($type -eq 'd') { $p.Directory = $true }
        $base = (Resolve-Path $r).ProviderPath.TrimEnd('\')
        Get-ChildItem @p | Where-Object { -not $name -or ($(if ($ci) { $_.Name -like $name } else { $_.Name -clike $name })) } |
            ForEach-Object { ($r.TrimEnd('\', '/') + $_.FullName.Substring($base.Length)).Replace('\', '/') }
    }
}

function global:sort {
    $gnu = __vx_gnu sort
    $files = $args.Count -gt 0 -and @($args | Where-Object { $_ -is [string] -and $_ -notmatch '^-' -and -not (Test-Path -LiteralPath $_) }).Count -eq 0
    if ($gnu -and ((__vx_linuxy Sort-Object $args) -or ($files -and -not $MyInvocation.ExpectingInput))) {
        if ($MyInvocation.ExpectingInput) { __vx_pipe $input $gnu @() $args } else { & $gnu @args }
        return
    }
    if ($MyInvocation.ExpectingInput) { $input | Sort-Object @args } else { Sort-Object @args }
}

function global:diff {
    $gnu = __vx_gnu diff
    $paths = @($args | Where-Object { $_ -is [string] -and $_ -notmatch '^-' })
    if ($gnu -and ((__vx_linuxy Compare-Object $args) -or ($paths.Count -eq 2 -and (Test-Path -LiteralPath $paths[0]) -and (Test-Path -LiteralPath $paths[1])))) { & $gnu --color=auto @args; return }
    Compare-Object @args
}

function global:tee {
    $gnu = __vx_gnu tee
    if ($gnu -and (__vx_linuxy Tee-Object $args)) { __vx_pipe $input $gnu @() $args; return }
    $input | Tee-Object @args
}

# where.exe for names; Where-Object in a pipeline (`... | where Name -eq x`)
function global:where {
    if ($MyInvocation.ExpectingInput -or ($args.Count -and $args[0] -is [scriptblock])) { $input | Where-Object @args; return }
    & "$env:SystemRoot\System32\where.exe" @args
}

function global:which {
    $all = $false; $names = @(); $status = 0
    foreach ($a in $args) { if ($a -eq '-a') { $all = $true } else { $names += [string]$a } }
    foreach ($n in $names) {
        $found = @(Get-Command $n -All:$all -ErrorAction SilentlyContinue)
        if (-not $all) { $found = @($found | Select-Object -First 1) }
        $g = __vx_gnu $n
        if (-not $found.Count -and -not $g) { [Console]::Error.WriteLine("which: no $n in PATH"); $status = 1; continue }
        foreach ($c in $found) {
            switch ($c.CommandType) {
                'Application' { $c.Source }
                'ExternalScript' { $c.Source }
                'Alias' { "${n}: aliased to $($c.Definition)" }
                'Function' { if ($c.ScriptBlock.File -like '*vibex-commands.ps1' -or $c.Definition -like '*__vx*' -or $c.Definition -like '*$exe*') { "${n}: Linux-style command (Vibex)" } else { "${n}: PowerShell function" } }
                'Cmdlet' { "${n}: PowerShell cmdlet ($($c.Source))" }
                default { "${n}: $($c.CommandType)" }
            }
        }
        if ($g -and ($all -or -not $found.Count)) { $g }
    }
    $global:LASTEXITCODE = $status
}

# cd (home), cd - (back), and the usual
$global:__VibexOldPwd = $null
function global:cd {
    $here = (Get-Location).Path
    if ($args.Count -eq 0) { Set-Location ~ }
    elseif ($args.Count -eq 1 -and $args[0] -eq '-') {
        if (-not $global:__VibexOldPwd) { Write-Error 'cd: OLDPWD not set'; return }
        Set-Location $global:__VibexOldPwd; (Get-Location).Path
    }
    else { Set-Location @args }
    if ((Get-Location).Path -ne $here) { $global:__VibexOldPwd = $here }
}
function global:pwd { $executionContext.SessionState.Path.CurrentLocation.Path }

function global:open { foreach ($t in $args) { if ([string]$t -match '^[a-z][a-z0-9+.-]*://') { Start-Process ([string]$t) } else { Invoke-Item -Path $t } }; if (-not $args) { Invoke-Item . } }
Set-Item function:global:xdg-open -Value (Get-Item function:open).ScriptBlock

# du (GNU's from Git, else a PowerShell one)
if (-not (__vx_gnu du)) {
    function global:du {
        $f = __vx_flags $args
        $h = $f.short.Contains('h')
        $paths = if ($f.rest.Count) { $f.rest } else { @('.') }
        foreach ($p in $paths) {
            $sum = (Get-ChildItem -Path $p -Recurse -File -Force -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum
            "{0}`t{1}" -f $(if ($h) { __vx_human $sum } else { [Math]::Ceiling($sum / 1KB) }), $p
        }
    }
}

# ------------------------------------------------------------ processes

function global:__vx_human([double]$b) {
    foreach ($u in '', 'K', 'M', 'G', 'T') { if ([Math]::Abs($b) -lt 1024 -or $u -eq 'T') { if ($u -eq '') { return "{0:0}" -f $b }; return "{0:0.#}$u" -f $b }; $b /= 1024 }
}

# ps aux / ps -ef: one line per process (text, so | grep works); ps alone
# is still Get-Process
function global:ps {
    $style = $args.Count -gt 0 -and [string]$args[0] -match '^-?(aux|axu|ax|au|ef|e|A|elf|eF)$'
    if (-not $style -and -not (__vx_linuxy Get-Process $args)) { Get-Process @args; return }
    $cpu = @{}; $mem = @{}
    foreach ($p in Get-Process) { $cpu[$p.Id] = $p.CPU; $mem[$p.Id] = $p.WorkingSet64 }
    '{0,7} {1,7} {2,9} {3,8}  {4}' -f 'PID', 'PPID', 'TIME(s)', 'MEM', 'COMMAND'
    Get-CimInstance Win32_Process | Sort-Object ProcessId | ForEach-Object {
        $line = if ($_.CommandLine) { $_.CommandLine } else { $_.Name }
        '{0,7} {1,7} {2,9} {3,8}  {4}' -f $_.ProcessId, $_.ParentProcessId, ('{0:0.0}' -f $cpu[[int]$_.ProcessId]), (__vx_human $mem[[int]$_.ProcessId]), $line
    }
}

# kill [-9] PID... ; kill -Name x and friends stay Stop-Process
function global:kill {
    $ids = @(); $other = $false
    foreach ($a in $args) {
        $s = [string]$a
        if ($s -eq '-l' -or $s -eq '-L') { 'HUP INT QUIT KILL TERM STOP CONT  (Windows only ends a process: every signal does the same)'; return }
        if ($s -match '^-(\d+|[A-Z]+|SIG[A-Z]+)$') { continue }
        if ($s -match '^\d+$') { $ids += [int]$s } else { $other = $true }
    }
    if ($other) { Stop-Process @args; return }
    if (-not $ids) { Write-Error 'usage: kill [-9] PID...'; return }
    Stop-Process -Id $ids -Force -ErrorAction Continue
}

# processes by name: a regex on the name (pgrep, pkill), the exact name
# (killall), or on the whole command line with -f
function global:__vx_procs([object[]]$argv, [bool]$exact = $false) {
    $f = __vx_flags $argv
    $full = $f.short.Contains('f')
    $ci = $f.short.Contains('i')
    $hits = foreach ($pat in $f.rest) {
        $pat = [string]$pat -replace '\.exe$', ''
        if ($full) { Get-CimInstance Win32_Process | Where-Object { $_.CommandLine -match $pat } | ForEach-Object { Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue } }
        elseif ($exact) { Get-Process | Where-Object { $_.ProcessName -eq $pat } }
        elseif ($ci) { Get-Process | Where-Object { $_.ProcessName -match $pat } }
        else { Get-Process | Where-Object { $_.ProcessName -cmatch $pat } }
    }
    @($hits | Where-Object { $_ -and $_.Id -ne $PID } | Sort-Object Id -Unique)
}
function global:pgrep { $f = __vx_flags $args; foreach ($p in (__vx_procs $args)) { if ($f.short.Contains('l')) { "$($p.Id) $($p.ProcessName)" } else { $p.Id } } }
function global:pkill { $p = __vx_procs $args; if (-not $p.Count) { $global:LASTEXITCODE = 1; return }; $p | Stop-Process -Force -ErrorAction Continue }
function global:killall { $p = __vx_procs $args $true; if (-not $p.Count) { [Console]::Error.WriteLine("killall: no process found"); $global:LASTEXITCODE = 1; return }; $p | Stop-Process -Force -ErrorAction Continue }

# top: the busiest processes, every 2 s; q quits
function global:top {
    $cores = [Environment]::ProcessorCount
    $prev = @{}; $t0 = Get-Date
    try {
        [Console]::CursorVisible = $false
        Clear-Host
        while ($true) {
            $now = Get-Date
            $dt = [Math]::Max(0.2, ($now - $t0).TotalSeconds); $t0 = $now
            $rows = foreach ($p in Get-Process) {
                $c = $p.CPU
                $pc = if ($null -ne $c -and $prev.ContainsKey($p.Id)) { ($c - $prev[$p.Id]) / $dt / $cores * 100 } else { 0 }
                if ($null -ne $c) { $prev[$p.Id] = $c }
                [pscustomobject]@{ Id = $p.Id; Name = $p.ProcessName; Cpu = $pc; Mem = $p.WorkingSet64 }
            }
            $os = Get-CimInstance Win32_OperatingSystem
            $up = $now - $os.LastBootUpTime
            $used = ($os.TotalVisibleMemorySize - $os.FreePhysicalMemory) * 1KB
            $h = [Console]::WindowHeight
            $lines = @(
                "top - {0:HH:mm:ss} up {1}d {2:hh\:mm},  {3} tasks,  {4} cores    (q quits)" -f $now, [int]$up.TotalDays, $up, @($rows).Count, $cores
                "Mem: {0} used of {1}" -f (__vx_human $used), (__vx_human ($os.TotalVisibleMemorySize * 1KB))
                ''
                '{0,7}  {1,6}  {2,8}  {3}' -f 'PID', '%CPU', 'MEM', 'NAME'
            )
            $lines += $rows | Sort-Object Cpu, Mem -Descending | Select-Object -First ([Math]::Max(5, $h - 6)) |
                ForEach-Object { '{0,7}  {1,6:0.0}  {2,8}  {3}' -f $_.Id, $_.Cpu, (__vx_human $_.Mem), $_.Name }
            $w = [Math]::Max(20, [Console]::WindowWidth - 1)
            [Console]::SetCursorPosition(0, 0)
            [Console]::Write((($lines | ForEach-Object { $s = [string]$_; if ($s.Length -gt $w) { $s.Substring(0, $w) } else { $s.PadRight($w) } }) -join "`n"))
            $until = (Get-Date).AddSeconds(2)
            while ((Get-Date) -lt $until) {
                if ([Console]::KeyAvailable) { $k = [Console]::ReadKey($true); if ($k.KeyChar -eq 'q' -or $k.Key -eq 'Escape') { return } }
                Start-Sleep -Milliseconds 100
            }
        }
    } finally { [Console]::CursorVisible = $true; Write-Host '' }
}
Set-Item function:global:htop -Value (Get-Item function:top).ScriptBlock

# ------------------------------------------------------------ system

function global:df {
    $f = __vx_flags $args
    $h = $f.short -match '[hH]'
    $fmt = '{0,-14} {1,9} {2,9} {3,9} {4,5}  {5}'
    $fmt -f 'Filesystem', $(if ($h) { 'Size' } else { '1K-blocks' }), 'Used', 'Avail', 'Use%', 'Mounted on'
    Get-CimInstance Win32_LogicalDisk | Where-Object { $_.Size } | ForEach-Object {
        $size = [double]$_.Size; $free = [double]$_.FreeSpace; $used = $size - $free
        $n = { param($v) if ($h) { __vx_human $v } else { [Math]::Floor($v / 1KB) } }
        $fs = if ($_.VolumeName) { "$($_.DeviceID) $($_.VolumeName)" } else { "$($_.DeviceID) $($_.FileSystem)" }
        $fmt -f $fs, (& $n $size), (& $n $used), (& $n $free), ('{0:0}%' -f ($used / $size * 100)), "$($_.DeviceID)\"
    }
}

function global:free {
    $f = __vx_flags $args
    $os = Get-CimInstance Win32_OperatingSystem
    $tot = [double]$os.TotalVisibleMemorySize * 1KB; $fr = [double]$os.FreePhysicalMemory * 1KB
    $stot = [double]($os.TotalVirtualMemorySize - $os.TotalVisibleMemorySize) * 1KB
    $sfree = [Math]::Max([double]0, [double]($os.FreeVirtualMemory - $os.FreePhysicalMemory) * 1KB)
    $n = { param($v) if ($f.short.Contains('h')) { __vx_human $v } elseif ($f.short.Contains('g')) { [Math]::Floor($v / 1GB) } elseif ($f.short.Contains('m')) { [Math]::Floor($v / 1MB) } else { [Math]::Floor($v / 1KB) } }
    '{0,-6} {1,12} {2,12} {3,12}' -f '', 'total', 'used', 'free'
    '{0,-6} {1,12} {2,12} {3,12}' -f 'Mem:', (& $n $tot), (& $n ($tot - $fr)), (& $n $fr)
    '{0,-6} {1,12} {2,12} {3,12}' -f 'Swap:', (& $n $stot), (& $n ([Math]::Max([double]0, $stot - $sfree))), (& $n $sfree)
}

function global:uptime {
    $boot = (Get-CimInstance Win32_OperatingSystem).LastBootUpTime
    $up = (Get-Date) - $boot
    if ($args -contains '-p') { 'up {0} days, {1} hours, {2} minutes' -f [int][Math]::Floor($up.TotalDays), $up.Hours, $up.Minutes; return }
    if ($args -contains '-s') { '{0:yyyy-MM-dd HH:mm:ss}' -f $boot; return }
    ' {0:HH:mm:ss} up {1} days, {2:hh\:mm},  {3} user' -f (Get-Date), [int][Math]::Floor($up.TotalDays), $up, 1
}

function global:nproc { [Environment]::ProcessorCount }

function global:uname {
    $f = __vx_flags $args
    $v = [Environment]::OSVersion.Version
    $arch = switch ($env:PROCESSOR_ARCHITECTURE) { 'AMD64' { 'x86_64' } 'ARM64' { 'aarch64' } 'x86' { 'i686' } default { $env:PROCESSOR_ARCHITECTURE } }
    $parts = [ordered]@{ s = 'Windows_NT'; n = $env:COMPUTERNAME; r = "$($v.Major).$($v.Minor).$($v.Build)"; v = (Get-CimInstance Win32_OperatingSystem).Caption; m = $arch; o = 'Windows' }
    if (-not $f.short) { return 'Windows_NT' }
    if ($f.short.Contains('a')) { return ($parts.Values -join ' ') }
    ($parts.Keys | Where-Object { $f.short.Contains($_) } | ForEach-Object { $parts[$_] }) -join ' '
}

function global:ifconfig { if ($args -contains '-a') { ipconfig /all } else { ipconfig @args } }

# ip a / ip r / ip link
function global:ip {
    $what = if ($args.Count) { [string]$args[0] } else { 'addr' }
    switch -Regex ($what) {
        '^(a|addr|address)$' {
            $i = 0
            foreach ($ad in Get-NetAdapter -IncludeHidden:$false -ErrorAction SilentlyContinue | Sort-Object ifIndex) {
                $i++
                $state = if ($ad.Status -eq 'Up') { 'UP' } else { 'DOWN' }
                "$($ad.ifIndex): $($ad.Name): <$state> mtu $($ad.MtuSize) state $state"
                "    link/ether $($ad.MacAddress.Replace('-', ':').ToLower())"
                foreach ($a in Get-NetIPAddress -InterfaceIndex $ad.ifIndex -ErrorAction SilentlyContinue) {
                    $fam = if ($a.AddressFamily -eq 'IPv4') { 'inet' } else { 'inet6' }
                    "    $fam $($a.IPAddress)/$($a.PrefixLength)"
                }
            }
        }
        '^(r|route)$' {
            Get-NetRoute -AddressFamily IPv4 -ErrorAction SilentlyContinue | Sort-Object RouteMetric | ForEach-Object {
                $dst = if ($_.DestinationPrefix -eq '0.0.0.0/0') { 'default' } else { $_.DestinationPrefix }
                $via = if ($_.NextHop -and $_.NextHop -ne '0.0.0.0') { " via $($_.NextHop)" } else { '' }
                "$dst$via dev $($_.InterfaceAlias) metric $($_.RouteMetric)"
            }
        }
        '^(l|link)$' { Get-NetAdapter | ForEach-Object { "$($_.ifIndex): $($_.Name): <$($_.Status.ToString().ToUpper())> link/ether $($_.MacAddress.Replace('-', ':').ToLower())" } }
        default { Write-Error "ip: try 'ip a', 'ip r' or 'ip link'" }
    }
}

# Sockets with their programs: lsof -i [:PORT], ss -tlnp, netstat -tulpn
function global:__vx_sockets([bool]$tcp, [bool]$udp, [bool]$listen, [bool]$all, $port) {
    $names = @{}; foreach ($p in Get-Process) { $names[$p.Id] = $p.ProcessName }
    $fmt = '{0,-5} {1,-11} {2,-28} {3,-28} {4}'
    $fmt -f 'Proto', 'State', 'Local', 'Peer', 'Process'
    if ($tcp) {
        foreach ($c in Get-NetTCPConnection -ErrorAction SilentlyContinue | Sort-Object LocalPort) {
            if ($listen -and $c.State -ne 'Listen') { continue }
            if (-not $listen -and -not $all -and $c.State -eq 'Listen') { continue }
            if ($port -and $c.LocalPort -ne $port -and $c.RemotePort -ne $port) { continue }
            $fmt -f 'tcp', $c.State, "$($c.LocalAddress):$($c.LocalPort)", "$($c.RemoteAddress):$($c.RemotePort)", "$($names[[int]$c.OwningProcess]) ($($c.OwningProcess))"
        }
    }
    if ($udp) {
        foreach ($c in Get-NetUDPEndpoint -ErrorAction SilentlyContinue | Sort-Object LocalPort) {
            if ($port -and $c.LocalPort -ne $port) { continue }
            $fmt -f 'udp', 'UNCONN', "$($c.LocalAddress):$($c.LocalPort)", '*:*', "$($names[[int]$c.OwningProcess]) ($($c.OwningProcess))"
        }
    }
}
function global:lsof {
    $f = __vx_flags $args @('i')
    if (-not ($f.short.Contains('i'))) { Write-Error 'lsof: Vibex does network sockets: lsof -i [:PORT], lsof -i tcp:3000'; return }
    $spec = [string]$f.value['i']
    $port = if ($spec -match ':(\d+)$') { [int]$Matches[1] } else { $null }
    $udp = $spec -notmatch '^tcp'
    $tcp = $spec -notmatch '^udp'
    __vx_sockets $tcp $udp $false $true $port
}
function global:ss {
    $f = __vx_flags $args
    $t = $f.short.Contains('t'); $u = $f.short.Contains('u')
    if (-not $t -and -not $u) { $t = $true; $u = $true }
    __vx_sockets $t $u ($f.short.Contains('l')) ($f.short.Contains('a')) $null
}
function global:netstat {
    # Linux flags (-tlnp, -tulpn, -plant) -> ss; Windows ones (-ano, -b, -r) -> netstat.exe
    $linux = @($args | Where-Object { [string]$_ -match '^-[a-zA-Z]*[lu][a-zA-Z]*$' }).Count -gt 0
    if ($linux) { ss @args } else { & "$env:SystemRoot\System32\NETSTAT.EXE" @args }
}

# ------------------------------------------------------------ environment

function global:export {
    if (-not $args.Count) { Get-ChildItem env: | Sort-Object Name | ForEach-Object { "declare -x $($_.Name)=`"$($_.Value)`"" }; return }
    foreach ($a in $args) {
        $s = [string]$a
        if ($s -notmatch '^([A-Za-z_][A-Za-z0-9_]*)=(.*)$') { if ($s -notmatch '^[A-Za-z_]\w*$') { Write-Error "export: '$s': not a valid identifier" }; continue }
        $name, $val = $Matches[1], $Matches[2]
        # export PATH=$PATH:/x (PowerShell has no $PATH: it arrives empty)
        if ($name -eq 'PATH') {
            $cur = [string]$env:PATH
            if ($cur -and $val.StartsWith($cur + ':')) { $val = $cur.TrimEnd(';') + ';' + $val.Substring($cur.Length + 1) }
            elseif ($cur -and $val.EndsWith(':' + $cur)) { $val = $val.Substring(0, $val.Length - $cur.Length - 1) + ';' + $cur }
            elseif ($val.StartsWith(':')) { $val = $cur.TrimEnd(';') + ';' + $val.Substring(1) }
            elseif ($val.EndsWith(':') -and $val -notmatch ';') { $val = $val.TrimEnd(':') + ';' + $cur }
        }
        Set-Item -Path "env:$name" -Value $val
    }
}
function global:unset { foreach ($n in $args) { Remove-Item -Path "env:$n" -ErrorAction SilentlyContinue; Remove-Variable -Name $n -Scope Global -ErrorAction SilentlyContinue } }
function global:printenv { if ($args.Count) { foreach ($n in $args) { [Environment]::GetEnvironmentVariable([string]$n) } } else { Get-ChildItem env: | Sort-Object Name | ForEach-Object { "$($_.Name)=$($_.Value)" } } }

# env (list) / env NAME=value ... COMMAND ARGS
function global:env {
    $set = @{}; $i = 0
    while ($i -lt $args.Count -and [string]$args[$i] -match '^([A-Za-z_]\w*)=(.*)$') { $set[$Matches[1]] = $Matches[2]; $i++ }
    if ($i -ge $args.Count) {
        if ($set.Count) { foreach ($k in $set.Keys) { Set-Item "env:$k" $set[$k] } } else { printenv }
        return
    }
    $old = @{}; foreach ($k in $set.Keys) { $old[$k] = [Environment]::GetEnvironmentVariable($k); Set-Item "env:$k" $set[$k] }
    try { $cmd = $args[$i]; $rest = @($args | Select-Object -Skip ($i + 1)); & $cmd @rest }
    finally { foreach ($k in $old.Keys) { if ($null -eq $old[$k]) { Remove-Item "env:$k" -ErrorAction SilentlyContinue } else { Set-Item "env:$k" $old[$k] } } }
}

# set (cmd: list / NAME=value) or Set-Variable
function global:set {
    if (-not $args.Count) { printenv; return }
    if ($args.Count -eq 1 -and [string]$args[0] -match '^([A-Za-z_][\w()]*)=(.*)$') { Set-Item "env:$($Matches[1])" $Matches[2]; return }
    Set-Variable @args -Scope Global
}

# alias ll='ls -la' (bash) / alias (list)
function global:alias {
    if (-not $args.Count) { Get-Alias | Sort-Object Name | ForEach-Object { "alias $($_.Name)='$($_.Definition)'" }; return }
    foreach ($a in $args) {
        $s = [string]$a
        if ($s -match '^([\w.-]+)=(.*)$') {
            $n, $v = $Matches[1], $Matches[2].Trim("'", '"')
            if (Test-Path "Alias:$n") { Remove-Item "Alias:$n" -Force }
            Set-Item "function:global:$n" -Value ([scriptblock]::Create("Invoke-Expression ('$($v.Replace("'", "''"))' + ' ' + (`$args -join ' '))"))
        } else {
            $c = Get-Command $s -ErrorAction SilentlyContinue | Select-Object -First 1
            if ($c) { "alias $s='$($c.Definition.Trim())'" } else { Write-Error "alias: ${s}: not found" }
        }
    }
}
function global:unalias { foreach ($n in $args) { Remove-Item "Alias:$n", "function:$n" -Force -ErrorAction SilentlyContinue } }

# sudo COMMAND: Windows' own sudo when it's turned on, else an elevated
# PowerShell window running it (UAC asks either way)
function global:sudo {
    $shell = (Get-Process -Id $PID).Path
    if (-not $args.Count) { Start-Process $shell -Verb RunAs; return }
    $line = ($args | ForEach-Object { $s = [string]$_; if ($s -match '[\s''"]') { "'" + $s.Replace("'", "''") + "'" } else { $s } }) -join ' '
    $mode = (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Sudo' -ErrorAction SilentlyContinue).Enabled
    $sudoExe = "$env:SystemRoot\System32\sudo.exe"
    if ($mode -and (Test-Path $sudoExe)) {
        $c = Get-Command ([string]$args[0]) -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($c -and $c.CommandType -eq 'Application') { & $sudoExe @args } else { & $sudoExe $shell -NoLogo -Command $line }
        return
    }
    Start-Process $shell -Verb RunAs -ArgumentList '-NoLogo', '-NoExit', '-Command', $line
}

# wget URL (curl.exe underneath when there's no wget.exe)
function global:wget {
    $real = Get-Command wget.exe -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($real) { & $real.Source @args; return }
    $f = __vx_flags $args @('O')
    $c = @('-L', '--fail')
    if ($f.short.Contains('q')) { $c += '-s' }
    if ($f.short.Contains('c')) { $c += @('-C', '-') }
    if ($f.value.ContainsKey('O')) { if ($f.value['O'] -eq '-') { } else { $c += @('-o', $f.value['O']) } } else { $c += '-O' }
    & "$env:SystemRoot\System32\curl.exe" @c @($f.rest)
}

# man NAME: PowerShell's help, or a GNU tool's --help
function global:man {
    if (-not $args.Count) { Get-Help; return }
    $n = [string]$args[0]
    $c = Get-Command $n -ErrorAction SilentlyContinue | Select-Object -First 1
    $g = __vx_gnu $n
    if ($g -and (-not $c -or $c.CommandType -eq 'Function')) { & $g --help 2>&1 | Out-Host -Paging; return }
    Get-Help $n -Full | Out-Host -Paging
}

if (-not (__vx_gnu chmod)) { function global:chmod { } }   # NTFS has no x bit

# ------------------------------------------------------------ anything else
# A command PowerShell can't find, that Git's usr\bin has (nano, vim, file,
# stat, basename, realpath, tar's friends ...), runs from there.
$global:__VibexPrevNotFound = $ExecutionContext.InvokeCommand.CommandNotFoundAction
$ExecutionContext.InvokeCommand.CommandNotFoundAction = {
    param([string]$name, [System.Management.Automation.CommandLookupEventArgs]$e)
    $n = $name -replace '\.exe$', ''
    if ($global:__VibexGnu -and $n -notlike 'get-*' -and $n -match '^[\w.+-]+$') {
        $exe = Join-Path $global:__VibexGnu "$n.exe"
        if (Test-Path -LiteralPath $exe) {
            $e.Command = $ExecutionContext.InvokeCommand.GetCommand($exe, 'Application')
            $e.StopSearch = $true
            return
        }
    }
    if ($global:__VibexPrevNotFound) { & $global:__VibexPrevNotFound $name $e }
}

# ------------------------------------------------------------ the typed line
# Linux one-liners pasted into PowerShell 5.1: a && b, a || b, 2>/dev/null.
# (PowerShell 7 has && and || itself; /dev/null is rewritten in both.)
$global:__VibexRewrite = {
    param([string]$line)
    if (-not $line -or ($line.IndexOf('&&') -lt 0 -and $line.IndexOf('||') -lt 0 -and $line.IndexOf('/dev/null') -lt 0 -and $line -cnotmatch '\$PATH\b')) { return $line }
    $parts = New-Object 'System.Collections.Generic.List[string]'
    $ops = New-Object 'System.Collections.Generic.List[string]'
    $sb = New-Object System.Text.StringBuilder
    $q = $null; $depth = 0; $i = 0
    while ($i -lt $line.Length) {
        $c = [string]$line[$i]
        if ($q) {
            [void]$sb.Append($c)
            if ($c -eq '`' -and $q -eq '"' -and $i + 1 -lt $line.Length) { $i++; [void]$sb.Append($line[$i]) }
            elseif ($c -eq $q) { $q = $null }
            $i++; continue
        }
        if ($c -eq "'") { $q = $c; [void]$sb.Append($c); $i++; continue }
        if ($c -eq '"') {
            # $PATH inside double quotes too
            [void]$sb.Append($c); $i++
            while ($i -lt $line.Length -and [string]$line[$i] -ne '"') {
                if ([string]$line[$i] -eq '`' -and $i + 1 -lt $line.Length) { [void]$sb.Append($line[$i]).Append($line[$i + 1]); $i += 2; continue }
                if ($line.Length - $i -ge 5 -and $line.Substring($i, 5) -ceq '$PATH' -and ($line.Length -eq $i + 5 -or $line[$i + 5] -notmatch '\w')) { [void]$sb.Append('${env:PATH}'); $i += 5; continue }
                [void]$sb.Append($line[$i]); $i++
            }
            if ($i -lt $line.Length) { [void]$sb.Append('"'); $i++ }
            continue
        }
        if ($c -eq '`' -and $i + 1 -lt $line.Length) { [void]$sb.Append($c).Append($line[$i + 1]); $i += 2; continue }
        # bash's $PATH (PowerShell has none: export PATH=$PATH:/x)
        if ($c -eq '$' -and $line.Length - $i -ge 5 -and $line.Substring($i, 5) -ceq '$PATH' -and ($line.Length -eq $i + 5 -or $line[$i + 5] -notmatch '\w')) { [void]$sb.Append('${env:PATH}'); $i += 5; continue }
        if ('({['.Contains($c)) { $depth++ } elseif (')}]'.Contains($c)) { $depth-- }
        $two = if ($i + 1 -lt $line.Length) { $line.Substring($i, 2) } else { '' }
        if ($depth -eq 0 -and ($two -eq '&&' -or $two -eq '||')) { $parts.Add($sb.ToString().Trim()); $ops.Add($two); [void]$sb.Clear(); $i += 2; continue }
        if ($line.Length - $i -ge 9 -and $line.Substring($i, 9) -eq '/dev/null' -and $sb.ToString().TrimEnd().EndsWith('>')) {
            $s = $sb.ToString().TrimEnd()
            if ($s.EndsWith('&>')) { $s = $s.Substring(0, $s.Length - 2) + '*>' }
            [void]$sb.Clear(); [void]$sb.Append($s).Append('$null'); $i += 9; continue
        }
        [void]$sb.Append($c); $i++
    }
    $parts.Add($sb.ToString().Trim())
    if ($ops.Count -eq 0 -or $PSVersionTable.PSVersion.Major -ge 7) {
        $out = $parts[0]
        for ($k = 0; $k -lt $ops.Count; $k++) { $out += " $($ops[$k]) $($parts[$k + 1])" }
        return $out
    }
    # bash's meaning, left to right: a && b || c runs c when a or b failed
    $out = "$($parts[0]); `$global:__vxok = `$?"
    for ($k = 0; $k -lt $ops.Count; $k++) {
        $test = if ($ops[$k] -eq '&&') { '$global:__vxok' } else { '-not $global:__vxok' }
        $out += "; if ($test) { $($parts[$k + 1]); `$global:__vxok = `$? }"
    }
    $out
}
