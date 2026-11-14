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
# still Get-ChildItem, `ls | sort Length`, `ls *.tmp | rm`,
# `Get-Process x | kill`, `Get-Process | where Name -eq x` and
# `kill -Name notepad` do what they always did (objects, pipeline input,
# streaming). Only flags that look like Linux ones (`-la`, `-rf`,
# `--color`) switch to the Linux behaviour.
#
# Loaded by vibex.ps1 inside Vibex. Turn off: Settings -> Terminal ->
# "Linux and Windows commands", or config.linux_commands = false.
#
# Load time matters (every pane runs this): no New-Object / Test-Path /
# Join-Path at load (their first use costs ~70-100 ms each in Windows
# PowerShell), .NET calls instead, and the generated functions are made
# from one script block.

if ($env:VIBEX_COMMANDS -eq '0' -or $ExecutionContext.SessionState.PSVariable.GetValue('__VibexCommands')) { return }
$global:__VibexCommands = $true

# Vibex is a UTF-8 terminal: say so both ways, so piping into and out of
# the GNU tools keeps accents, emoji and box drawing intact. (Windows
# PowerShell also starts every pipe into a program with a BOM unless the
# console's input side is UTF-8 without one.)
$global:__VibexUtf8 = [System.Text.UTF8Encoding]::new($false)
try { if ([Console]::OutputEncoding.CodePage -ne 65001) { [Console]::OutputEncoding = $global:__VibexUtf8 } } catch { }
try { [Console]::InputEncoding = $global:__VibexUtf8 } catch { }
$global:OutputEncoding = $global:__VibexUtf8

$global:__VibexGnu = $null
foreach ($__vxd in @($env:VIBEX_GNU, "$env:ProgramFiles\Git\usr\bin", "${env:ProgramFiles(x86)}\Git\usr\bin", "$env:LOCALAPPDATA\Programs\Git\usr\bin")) {
    if ($__vxd -and [IO.File]::Exists([IO.Path]::Combine($__vxd, 'grep.exe'))) { $global:__VibexGnu = $__vxd; break }
}
$global:__VibexGnuCache = @{}
$global:__VibexParamCache = @{}

function global:__vx_gnu([string]$name) {
    if (-not $global:__VibexGnu) { return $null }
    $c = $global:__VibexGnuCache
    if (-not $c.ContainsKey($name)) {
        $p = [IO.Path]::Combine($global:__VibexGnu, "$name.exe")
        $c[$name] = if ([IO.File]::Exists($p)) { $p } else { $null }
    }
    $c[$name]
}

# A cmdlet's parameter names and aliases (cached: Get-Command is slow)
function global:__vx_params([string]$cmdlet) {
    $c = $global:__VibexParamCache
    if (-not $c.ContainsKey($cmdlet)) {
        $cmd = Get-Command $cmdlet -ErrorAction SilentlyContinue | Select-Object -First 1
        $c[$cmdlet] = @(if ($cmd) { foreach ($p in $cmd.Parameters.Values) { $p.Name; $p.Aliases } })
    }
    $c[$cmdlet]
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
            if ($null -eq $names) { $names = __vx_params $cmdlet }
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
    # PowerShell marks a bare -x it passes in $args; a quoted '-x', or one
    # after `--` (which PowerShell keeps for itself), isn't marked: that's
    # a file name, as `rm -rf -- -dash` means
    $marked = @($argv | Where-Object { $_ -is [string] -and $null -ne $_.PSObject.Members['<CommandParameterName>'] }).Count -gt 0
    for ($i = 0; $i -lt $argv.Count; $i++) {
        $a = $argv[$i]
        if ($a -is [int] -and $a -lt 0) { $a = [string]$a }
        if ($done -or $a -isnot [string]) { $r.rest += $a; continue }
        if ($marked -and $a.StartsWith('-') -and $null -eq $argv[$i].PSObject.Members['<CommandParameterName>'] -and $argv[$i] -isnot [int]) { $r.rest += $a; continue }
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

# Wrappers run the PowerShell command as a steppable pipeline, made right
# in the wrapper (`$vxa = $args; $sp = { & 'Cmdlet' @vxa }...`), so it keeps
# the cmdlet's pipeline input and streaming (`ls *.tmp | rm`, `... | where`)
# and parameter names like -Force survive. Variables the cmdlet sets
# (tee -Variable x, -OutVariable, -ErrorVariable ...) land in the wrapper's
# scope; __vx_export_vars hands them to the caller, as the alias would.
function global:__vx_export_vars([object[]]$argv) {
    for ($i = 0; $i -lt $argv.Count - 1; $i++) {
        if ($argv[$i] -is [string] -and $argv[$i] -match '^-(v(a(r(i(a(b(l(e)?)?)?)?)?)?)?|OutVariable|ov|ErrorVariable|ev|WarningVariable|wv|InformationVariable|iv)$') {
            $n = ([string]$argv[$i + 1]).TrimStart('+')
            $v = Get-Variable -Name $n -Scope 1 -ErrorAction SilentlyContinue
            if ($v) { Set-Variable -Name $n -Value $v.Value -Scope 2 }
        }
    }
}

# Where a command's output goes, from the typed line: Top = a top-level
# statement whose output reaches the host (not `$x = ...`, `(...)`,
# `foreach ($f in ...)`, a redirection or a later pipeline stage); Last =
# the last statement on the line (its status is the line's).
function global:__vx_ctx($inv) {
    $r = @{ Top = $false; Last = $false; Host = $false }
    if ($inv.CommandOrigin -ne 'Runspace' -or $inv.PipelinePosition -ne $inv.PipelineLength -or -not $inv.Line) { return $r }
    $ast = $null
    try { $ast = [System.Management.Automation.Language.Parser]::ParseInput($inv.Line, [ref]$null, [ref]$null) } catch { return $r }
    $off = $inv.OffsetInLine - 1
    $cmd = $ast.Find({ param($x) $x -is [System.Management.Automation.Language.CommandAst] -and $x.Extent.StartOffset -eq $off }, $true)
    if (-not $cmd) { return $r }
    $pipe = $cmd.Parent
    if ($pipe -isnot [System.Management.Automation.Language.PipelineAst]) { return $r }
    $nb = $pipe.Parent
    if ($nb -isnot [System.Management.Automation.Language.NamedBlockAst] -or $nb.Parent.Parent) { return $r }
    $r.Top = $true
    $r.Last = [object]::ReferenceEquals($nb.Statements[$nb.Statements.Count - 1], $pipe)
    $r.Host = -not @($cmd.Redirections | Where-Object { $_.FromStream -eq 'Output' -or $_.FromStream -eq 'All' }).Count
    $r
}

# An emulated command's exit status: $LASTEXITCODE, and (when it is the
# last thing on the typed line) the red mark at the next prompt. A script
# function can't make $? false, so the prompt reads $global:__VibexFail.
function global:__vx_status([int]$code, $inv) {
    $global:LASTEXITCODE = $code
    if ($code -ne 0 -and (__vx_ctx $inv).Last) { $global:__VibexFail = $code }
}

# Arguments for Git's MSYS programs. Their runtime re-parses the command
# line Windows PowerShell builds: it globs unquoted * ? [ ], expands
# {a,b} and takes ' as a quote. File names should still glob (`ls *.txt`),
# but a grep/sed/awk pattern, find's expression, tr's sets and xargs'
# command must reach the program as typed: those go in double quotes
# (PowerShell 5.1 passes the quotes through when there's no space in the
# argument; with a space it quotes it itself). A ' anywhere is quoted too.
function global:__vx_mq($s, [bool]$literal = $true) {
    if ($s -isnot [string] -or $s -match '[\s"]') { return $s }
    if (($literal -and $s -match "[*?\[\]{}']") -or $s.Contains("'")) { return '"' + ($s -replace '(\\+)$', '$1$1') + '"' }
    $s
}
$global:__VibexMsysSpec = @{
    #          short options with a value, of those: protect, script given; long: valued, protect, script given
    grep = @('efmABCdD', 'e', 'ef', 'regexp|file|max-count|after-context|before-context|context|include|exclude|exclude-dir|exclude-from|label|binary-files|devices|directories', 'regexp|include|exclude|exclude-dir', 'regexp|file')
    sed  = @('efl', 'e', 'ef', 'expression|file|line-length', 'expression', 'expression|file')
    awk  = @('fvF', 'vF', 'f', 'file|assign|field-separator', 'assign|field-separator', 'file')
}
function global:__vx_msys([string]$tool, [object[]]$argv) {
    $a = @(foreach ($x in $argv) { if ($x -is [int] -or $x -is [long] -or $x -is [double]) { [string]$x } else { $x } })
    if ($tool -eq 'tr' -or $tool -eq 'xargs') { return , @(foreach ($x in $a) { __vx_mq $x }) }
    if ($tool -eq 'find') {
        $expr = $false
        return , @(foreach ($x in $a) { if (-not $expr -and $x -is [string] -and $x -match '^[-(!),]') { $expr = $true }; __vx_mq $x $expr })
    }
    $key = if ($tool -match '^(e|f)?grep$') { 'grep' } elseif ($tool -eq 'gawk') { 'awk' } else { $tool }
    $spec = $global:__VibexMsysSpec[$key]
    if (-not $spec) { return , @(foreach ($x in $a) { __vx_mq $x $false }) }
    $vs, $ps, $gs, $vl, $pl, $gl = $spec
    $out = New-Object System.Collections.Generic.List[object]
    $first = -1; $firstVal = $null; $given = $false; $dd = $false; $i = 0
    while ($i -lt $a.Count) {
        $x = $a[$i]; $i++
        if ($dd -or $x -isnot [string] -or $x -eq '-' -or -not $x.StartsWith('-')) { if ($first -lt 0) { $first = $out.Count; $firstVal = $x }; $out.Add((__vx_mq $x $false)); continue }
        if ($x -eq '--') { $dd = $true; $out.Add($x); continue }
        if ($x.StartsWith('--')) {
            $name = ($x.Substring(2) -split '=', 2)[0]
            if ($name -match "^($gl)$") { $given = $true }
            $out.Add((__vx_mq $x $false))
            if (-not $x.Contains('=') -and $name -match "^($vl)$" -and $i -lt $a.Count) { $out.Add((__vx_mq $a[$i] ($name -match "^($pl)$"))); $i++ }
            continue
        }
        $out.Add((__vx_mq $x $false))
        for ($j = 1; $j -lt $x.Length; $j++) {
            $c = [string]$x[$j]
            if ($vs.Contains($c)) {
                if ($gs.Contains($c)) { $given = $true }
                if ($j -eq $x.Length - 1 -and $i -lt $a.Count) { $out.Add((__vx_mq $a[$i] $ps.Contains($c))); $i++ }
                break
            }
        }
    }
    # the pattern / script: the first operand, unless -e / -f gave one
    if (-not $given -and $first -ge 0) { $out[$first] = __vx_mq $firstVal $true }
    , $out.ToArray()
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
    $out = [System.Text.StringBuilder]::new()
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
    $words = @($exe) + @($opts) + @($argv | ForEach-Object { [string]$_ })
    $cmdline = __vx_cmdesc (($words | ForEach-Object { __vx_q $_ }) -join ' ')
    $pre = if ($withPath) { "@set `"PATH=$($global:__VibexGnu);%PATH%`"`r`n" } else { '' }
    $bat = [IO.Path]::Combine([IO.Path]::GetTempPath(), ("vibex-pipe-{0}.cmd" -f [guid]::NewGuid().ToString('N').Substring(0, 12)))
    [IO.File]::WriteAllText($bat, "$pre@`"$tr`" -d `"\r`" | $cmdline`r`n@exit /b %errorlevel%`r`n", $global:__VibexUtf8)
    try { $in | Out-String -Stream | & $env:ComSpec /d /c $bat }
    finally { [IO.File]::Delete($bat) }
}

# Aliases PowerShell puts in the way (functions can't beat aliases).
# (The provider call, not Get-Alias / Remove-Item: their first use costs
# ~80 ms at load.)
foreach ($__vxa in 'ls', 'dir', 'cat', 'rm', 'del', 'erase', 'rd', 'cp', 'copy', 'mv', 'move', 'sort', 'diff', 'tee', 'kill', 'ps', 'curl', 'wget', 'where', 'cd', 'pwd', 'man', 'set') {
    try { if ($ExecutionContext.SessionState.InvokeProvider.Item.Exists("Alias:\$__vxa")) { $ExecutionContext.SessionState.InvokeProvider.Item.Remove([string[]]@("Alias:\$__vxa"), $false, $true, $true) } } catch { }
}
foreach ($__vxv in '__vxa', '__vxd', '__vxv') { $ExecutionContext.SessionState.PSVariable.Remove($__vxv) }

# A GNU program the wrapper ran failed as the last thing on the typed line:
# red mark at the next prompt (a function returns with $? still true)
function global:__vx_native_done($inv) {
    if ($global:LASTEXITCODE -and (__vx_ctx $inv).Last) { $global:__VibexFail = $global:LASTEXITCODE }
}

# ------------------------------------------------------------ GNU text tools
# Pipe-aware: PowerShell objects are formatted as text first, so
# `Get-Process | grep chrome` and `ls | wc -l` work like on Linux.
function global:__vx_text([string]$name, [string]$tool, [object[]]$o, [object[]]$argv, $inv, $in) {
    $exe = __vx_gnu $tool
    if ($inv.ExpectingInput) { __vx_pipe $in $exe $o $argv ($name -eq 'xargs'); return }
    $m = __vx_msys $name $argv
    if ($name -eq 'xargs') { $old = $env:PATH; $env:PATH = $global:__VibexGnu + ';' + $old; try { & $exe @o @m } finally { $env:PATH = $old } }
    else { & $exe @o @m }
    __vx_native_done $inv
}
if (__vx_gnu grep) {
    function global:grep { __vx_text grep grep @('--color=auto') $args $MyInvocation $input }
    function global:egrep { __vx_text egrep grep @('-E', '--color=auto') $args $MyInvocation $input }
    function global:fgrep { __vx_text fgrep grep @('-F', '--color=auto') $args $MyInvocation $input }
    if (__vx_gnu sed) { function global:sed { __vx_text sed sed @() $args $MyInvocation $input } }
    if (__vx_gnu awk) { function global:awk { __vx_text awk awk @() $args $MyInvocation $input } }
    if (__vx_gnu gawk) { function global:gawk { __vx_text gawk gawk @() $args $MyInvocation $input } }
    if (__vx_gnu wc) { function global:wc { __vx_text wc wc @() $args $MyInvocation $input } }
    if (__vx_gnu uniq) { function global:uniq { __vx_text uniq uniq @() $args $MyInvocation $input } }
    if (__vx_gnu cut) { function global:cut { __vx_text cut cut @() $args $MyInvocation $input } }
    if (__vx_gnu tr) { function global:tr { __vx_text tr tr @() $args $MyInvocation $input } }
    if (__vx_gnu xargs) { function global:xargs { __vx_text xargs xargs @() $args $MyInvocation $input } }
    if (__vx_gnu head) { function global:head { __vx_text head head @() $args $MyInvocation $input } }
    if (__vx_gnu tail) { function global:tail { __vx_text tail tail @() $args $MyInvocation $input } }
    if (__vx_gnu less) { function global:less { __vx_text less less @('-R') $args $MyInvocation $input } }
    if (__vx_gnu nl) { function global:nl { __vx_text nl nl @() $args $MyInvocation $input } }
    if (__vx_gnu tac) { function global:tac { __vx_text tac tac @() $args $MyInvocation $input } }
    if (__vx_gnu rev) { function global:rev { __vx_text rev rev @() $args $MyInvocation $input } }
    if (__vx_gnu column) { function global:column { __vx_text column column @() $args $MyInvocation $input } }
    if (__vx_gnu base64) { function global:base64 { __vx_text base64 base64 @() $args $MyInvocation $input } }
    if (__vx_gnu md5sum) { function global:md5sum { __vx_text md5sum md5sum @() $args $MyInvocation $input } }
    if (__vx_gnu sha1sum) { function global:sha1sum { __vx_text sha1sum sha1sum @() $args $MyInvocation $input } }
    if (__vx_gnu sha256sum) { function global:sha256sum { __vx_text sha256sum sha256sum @() $args $MyInvocation $input } }
    if (__vx_gnu vim) { function global:vi { __vx_text vi vim @() $args $MyInvocation $input } }
}

# Without Git: PowerShell versions of the everyday ones.
if (-not (__vx_gnu grep)) {
    function global:grep {
        $f = __vx_flags $args @('e', 'm')
        $pat = if ($f.value.ContainsKey('e')) { $f.value['e'] } else { $f.rest[0]; $f.rest = @($f.rest | Select-Object -Skip 1) }
        if ($null -eq $pat) { Write-Error 'usage: grep [-inrvlcwF] PATTERN [FILE...]'; __vx_status 2 $MyInvocation; return }
        $p = @{ Pattern = $pat; CaseSensitive = -not $f.short.Contains('i'); NotMatch = $f.short.Contains('v') }
        if ($f.short.Contains('F')) { $p.SimpleMatch = $true }
        if ($f.short.Contains('w') -and -not $p.SimpleMatch) { $p.Pattern = "\b(?:$pat)\b" }
        $hits = @(if ($f.rest.Count -gt 0) {
                $files = if ($f.short -cmatch '[rR]') { Get-ChildItem -Path $f.rest -Recurse -File -ErrorAction SilentlyContinue } else { Get-Item -Path $f.rest -ErrorAction SilentlyContinue }
                $files | Select-String @p
            } else { $input | Out-String -Stream | Select-String @p })
        $many = $f.rest.Count -gt 1 -or $f.short -cmatch '[rR]'
        if ($f.short.Contains('l')) { $hits | Select-Object -ExpandProperty Path -Unique }
        elseif ($f.short.Contains('c')) { $hits.Count }
        else {
            foreach ($h in $hits) {
                $pre = ''
                if ($many -and $h.Path -ne 'InputStream') { $pre += (Resolve-Path -Relative $h.Path) + ':' }
                if ($f.short.Contains('n')) { $pre += "$($h.LineNumber):" }
                "$pre$($h.Line)"
            }
        }
        __vx_status ([int](-not $hits.Count)) $MyInvocation
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
    begin {
        $sp = $null; $mode = 'ps'
        $gnu = __vx_gnu ls
        # `ls -r` alone is PowerShell's -Recurse (what PowerShell hands type it for)
        $onlyR = (@($args | Where-Object { $_ -is [string] -and $_ -match '^-' }) -join ' ') -ceq '-r'
        $linuxy = -not $onlyR -and -not $MyInvocation.ExpectingInput -and (__vx_linuxy Get-ChildItem $args)
        if ($linuxy) { $mode = if ($gnu) { 'gnu' } else { 'emulate' } }
        # the Dragon (Kali) style: plain `ls` and `ls folder` typed at the
        # prompt are Linux's too, unless a PowerShell parameter is given or
        # the output goes somewhere (`$x = ls`, `ls | ...`, `(ls).Count`)
        elseif ($gnu -and $env:VIBEX_PROMPT -eq 'dragon' -and -not $MyInvocation.ExpectingInput -and -not $onlyR -and
            -not @($args | Where-Object { $_ -isnot [string] -or $_ -match '^-[A-Za-z]{2,}' }).Count -and (__vx_ctx $MyInvocation).Host) { $mode = 'gnu' }
        if ($mode -eq 'ps') { $vxa = $args; $sp = { & 'Microsoft.PowerShell.Management\Get-ChildItem' @vxa }.GetSteppablePipeline($MyInvocation.CommandOrigin); $sp.Begin($MyInvocation.ExpectingInput) }
    }
    process { if ($sp -and $MyInvocation.ExpectingInput) { $sp.Process($_) } }
    end {
        if ($sp) { $sp.End(); __vx_export_vars $args; return }
        if ($mode -eq 'gnu') { $m = __vx_msys 'ls' $args; & $gnu --color=auto @m; __vx_native_done $MyInvocation; return }
        # no Git: -a/-A show hidden, -R recurses, -d lists folders themselves
        $f = __vx_flags $args
        $p = @{}
        if ($f.short -cmatch '[aA]') { $p.Force = $true }
        if ($f.short.Contains('R') -or $f.long -contains 'recursive') { $p.Recurse = $true }
        if ($f.rest.Count) { $p.Path = $f.rest }
        if ($f.short.Contains('d')) { Get-Item @p } else { Get-ChildItem @p }
    }
}

# Kali sorts names the way its en_US.UTF-8 locale does (case doesn't come
# first: bugbounty before CTF)
if ($env:VIBEX_PROMPT -eq 'dragon' -and -not $env:LC_ALL -and -not $env:LC_COLLATE) { $env:LC_COLLATE = 'en_US.UTF-8' }

# Kali's ls colours: dircolors' database plus its fix for 777 folders
if ($env:VIBEX_PROMPT -eq 'dragon' -and -not $env:LS_COLORS) {
    $__vxdc = __vx_gnu dircolors
    if ($__vxdc) {
        # (running it sets $LASTEXITCODE: put it back as it was)
        $__vxlec = $ExecutionContext.SessionState.PSVariable.Get('LASTEXITCODE')
        $__vxdb = [regex]::Match(((& $__vxdc -b) -join "`n"), "LS_COLORS='([^']*)'")
        if ($__vxdb.Success) { $env:LS_COLORS = $__vxdb.Groups[1].Value.TrimEnd(':') + ':ow=30;44:' }
        if ($__vxlec) { $global:LASTEXITCODE = $__vxlec.Value } else { $ExecutionContext.SessionState.PSVariable.Remove('LASTEXITCODE') }
    }
    foreach ($__vxv in '__vxdc', '__vxdb', '__vxlec', '__vxv') { $ExecutionContext.SessionState.PSVariable.Remove($__vxv) }
}
function global:ll { $gnu = __vx_gnu ls; if ($gnu) { $m = __vx_msys 'ls' $args; & $gnu -lh --color=auto @m } else { Get-ChildItem -Force @args } }
function global:la { $gnu = __vx_gnu ls; if ($gnu) { $m = __vx_msys 'ls' $args; & $gnu -lAh --color=auto @m } else { Get-ChildItem -Force @args } }

# cmd's switches (/s, /q ...) present?
function global:__vx_cmdswitch([object[]]$argv) { @($argv | Where-Object { $_ -is [string] -and $_ -match '^/[A-Za-z?-]' }).Count -gt 0 }

# dir /s /b (cmd's) or dir (PowerShell's)
function global:dir {
    begin { $sp = $null; if (-not (__vx_cmdswitch $args)) { $vxa = $args; $sp = { & 'Microsoft.PowerShell.Management\Get-ChildItem' @vxa }.GetSteppablePipeline($MyInvocation.CommandOrigin); $sp.Begin($MyInvocation.ExpectingInput) } }
    process { if ($sp -and $MyInvocation.ExpectingInput) { $sp.Process($_) } }
    end { if ($sp) { $sp.End(); __vx_export_vars $args } else { & $env:ComSpec /d /c dir @args } }
}

function global:cat {
    begin {
        $sp = $null; $buf = $null
        $gnu = __vx_gnu cat
        if ($gnu -and (__vx_linuxy Get-Content $args)) { $buf = [System.Collections.Generic.List[object]]::new() }
        else { $vxa = $args; $sp = { & 'Microsoft.PowerShell.Management\Get-Content' @vxa }.GetSteppablePipeline($MyInvocation.CommandOrigin); $sp.Begin($MyInvocation.ExpectingInput) }
    }
    process { if ($MyInvocation.ExpectingInput) { if ($sp) { $sp.Process($_) } else { $buf.Add($_) } } }
    end {
        if ($sp) { $sp.End(); __vx_export_vars $args; return }
        if ($MyInvocation.ExpectingInput) { __vx_pipe $buf $gnu @() $args } else { $m = __vx_msys 'cat' $args; & $gnu @m; __vx_native_done $MyInvocation }
    }
}

# Why rm must not touch this (GNU's --preserve-root, plus drive roots,
# PowerShell drive roots and your home folder, which Windows has no
# single `/` for), or $null. '.' and '..' are always refused.
function global:__vx_rm_refusal([string]$arg, $item, [bool]$noPreserve = $false) {
    if (($arg -replace '[\\/]+$', '') -match '(^|[\\/])\.{1,2}$') { return "rm: refusing to remove '.' or '..' directory: skipping '$arg'" }
    if ($noPreserve -or -not $item -or -not $item.PSIsContainer) { return $null }
    $root = $false
    if ($item -is [IO.DirectoryInfo]) {
        $full = $item.FullName.TrimEnd('\')
        if ($null -eq $item.Parent) { $root = $true }
        elseif ($item.PSDrive -and $item.PSDrive.Root -and $full -eq $item.PSDrive.Root.TrimEnd('\')) { $root = $true }
        else {
            $homes = @($env:USERPROFILE, $HOME, (Get-PSProvider FileSystem).Home) | Where-Object { $_ } | ForEach-Object { ([string]$_).TrimEnd('\') }
            if ($homes -contains $full) { return "rm: refusing to remove your home folder '$arg'" }
        }
    } elseif (-not $item.PSParentPath) { $root = $true }
    if ($root) { return "rm: it is dangerous to operate recursively on '$arg' (a drive root)`nrm: use --no-preserve-root to override this failsafe" }
    $null
}

# A path argument: the literal file if there is one (brackets in names),
# else a wildcard
function global:__vx_items([string]$p) {
    if (Test-Path -LiteralPath $p) { @(Get-Item -LiteralPath $p -Force -ErrorAction SilentlyContinue) }
    else { @(Get-Item -Path $p -Force -ErrorAction SilentlyContinue) }
}

function global:rm {
    begin { $sp = $null; if (-not (__vx_linuxy Remove-Item $args)) { $vxa = $args; $sp = { & 'Microsoft.PowerShell.Management\Remove-Item' @vxa }.GetSteppablePipeline($MyInvocation.CommandOrigin); $sp.Begin($MyInvocation.ExpectingInput) } }
    process { if ($sp -and $MyInvocation.ExpectingInput) { $sp.Process($_) } }
    end {
        if ($sp) { $sp.End(); __vx_export_vars $args; return }
        $f = __vx_flags $args
        $recurse = $f.short -cmatch '[rR]' -or $f.long -contains 'recursive'
        $force = $f.short.Contains('f') -or $f.long -contains 'force'
        $verbose = $f.short.Contains('v') -or $f.long -contains 'verbose'
        $ask = $f.short.Contains('i')
        $noPreserve = $f.long -contains 'no-preserve-root'
        $status = 0
        if (-not $f.rest.Count -and -not $force) { Write-Error "rm: missing operand"; $status = 1 }
        foreach ($p in $f.rest) {
            $p = [string]$p
            $why = __vx_rm_refusal $p $null
            if ($why) { Write-Error $why; $status = 1; continue }
            # rm -rf /*, C:\*, ~\*: a wildcard right at a drive root or home
            if ($recurse -and -not $noPreserve -and -not (Test-Path -LiteralPath $p) -and [Management.Automation.WildcardPattern]::ContainsWildcardCharacters($p)) {
                $parent = Split-Path $p -Parent
                if (-not $parent) { $parent = '.' }
                $pi = Get-Item -Path $parent -Force -ErrorAction SilentlyContinue
                if ($pi -and (__vx_rm_refusal $parent $pi)) { Write-Error "rm: refusing to expand '$p' at a drive root or your home folder (--no-preserve-root overrides)"; $status = 1; continue }
            }
            $items = @(__vx_items $p)
            if (-not $items.Count) {
                if (-not $force) { Write-Error "rm: cannot remove '$p': No such file or directory"; $status = 1 }
                continue
            }
            foreach ($it in $items) {
                if ($it.PSIsContainer -and -not $recurse) {
                    if ($f.short.Contains('d') -and -not @(Get-ChildItem -LiteralPath $it.PSPath -Force -ErrorAction SilentlyContinue).Count) { }
                    else { Write-Error "rm: cannot remove '$p': Is a directory"; $status = 1; continue }
                }
                if ($recurse) { $why = __vx_rm_refusal $p $it $noPreserve; if ($why) { Write-Error $why; $status = 1; continue } }
                if ($ask -and (Read-Host "rm: remove '$($it.Name)'? [y/N]") -notmatch '^[yY]') { continue }
                $ev = $null
                Microsoft.PowerShell.Management\Remove-Item -LiteralPath $it.PSPath -Recurse:$recurse -Force -ErrorAction SilentlyContinue -ErrorVariable ev
                if ($ev) { foreach ($e in $ev) { Write-Error "rm: cannot remove '$p': $($e.Exception.Message)" }; $status = 1 }
                elseif ($verbose) { "removed '$($it.Name)'" }
            }
        }
        __vx_status $status $MyInvocation
    }
}

# cmd's del /s /q and rd /s /q on a drive root or your home folder
function global:__vx_cmd_refusal([object[]]$argv) {
    if (-not @($argv | Where-Object { $_ -is [string] -and $_ -match '^/[sS]$' }).Count) { return $null }
    foreach ($a in $argv) {
        $s = [string]$a
        if ($s -match '^/') { continue }
        $target = $s
        if ([Management.Automation.WildcardPattern]::ContainsWildcardCharacters($s)) { $target = Split-Path $s -Parent; if (-not $target) { $target = '.' } }
        $it = Get-Item -LiteralPath $target -Force -ErrorAction SilentlyContinue
        if (-not $it) { $it = Get-Item -Path $target -Force -ErrorAction SilentlyContinue | Select-Object -First 1 }
        $why = __vx_rm_refusal $target $it
        if ($why) { return $why }
    }
    $null
}
function global:__vx_remove_like([string]$name, [object[]]$argv, $inv) {
    $why = __vx_cmd_refusal $argv
    if ($why) { Write-Error ($why -replace '^rm:', "${name}:"); __vx_status 1 $inv; return }
    & $env:ComSpec /d /c $name @argv
}
# cmd's switches (del /q, rd /s /q) go to cmd; anything else is Remove-Item
function global:del {
    begin { $sp = $null; if (-not (__vx_cmdswitch $args)) { $vxa = $args; $sp = { & 'Microsoft.PowerShell.Management\Remove-Item' @vxa }.GetSteppablePipeline($MyInvocation.CommandOrigin); $sp.Begin($MyInvocation.ExpectingInput) } }
    process { if ($sp -and $MyInvocation.ExpectingInput) { $sp.Process($_) } }
    end { if ($sp) { $sp.End(); __vx_export_vars $args } else { __vx_remove_like 'del' $args $MyInvocation } }
}
function global:erase {
    begin { $sp = $null; if (-not (__vx_cmdswitch $args)) { $vxa = $args; $sp = { & 'Microsoft.PowerShell.Management\Remove-Item' @vxa }.GetSteppablePipeline($MyInvocation.CommandOrigin); $sp.Begin($MyInvocation.ExpectingInput) } }
    process { if ($sp -and $MyInvocation.ExpectingInput) { $sp.Process($_) } }
    end { if ($sp) { $sp.End(); __vx_export_vars $args } else { __vx_remove_like 'erase' $args $MyInvocation } }
}
function global:rd {
    begin { $sp = $null; if (-not (__vx_cmdswitch $args)) { $vxa = $args; $sp = { & 'Microsoft.PowerShell.Management\Remove-Item' @vxa }.GetSteppablePipeline($MyInvocation.CommandOrigin); $sp.Begin($MyInvocation.ExpectingInput) } }
    process { if ($sp -and $MyInvocation.ExpectingInput) { $sp.Process($_) } }
    end { if ($sp) { $sp.End(); __vx_export_vars $args } else { __vx_remove_like 'rd' $args $MyInvocation } }
}

function global:cp {
    begin { $sp = $null; if (-not (__vx_linuxy Copy-Item $args)) { $vxa = $args; $sp = { & 'Microsoft.PowerShell.Management\Copy-Item' @vxa }.GetSteppablePipeline($MyInvocation.CommandOrigin); $sp.Begin($MyInvocation.ExpectingInput) } }
    process { if ($sp -and $MyInvocation.ExpectingInput) { $sp.Process($_) } }
    end {
        if ($sp) { $sp.End(); __vx_export_vars $args; return }
        $f = __vx_flags $args
        if ($f.rest.Count -lt 2) { Write-Error 'usage: cp [-rfvn] SOURCE... DEST'; __vx_status 1 $MyInvocation; return }
        $dest = [string]$f.rest[-1]; $status = 0
        $recurse = $f.short -cmatch '[rRa]' -or $f.long -contains 'recursive'
        foreach ($s in $f.rest[0..($f.rest.Count - 2)]) {
            $items = @(__vx_items ([string]$s))
            if (-not $items.Count) { Write-Error "cp: cannot stat '$s': No such file or directory"; $status = 1; continue }
            foreach ($it in $items) {
                if ($it.PSIsContainer -and -not $recurse) { Write-Error "cp: -r not specified; omitting directory '$s'"; $status = 1; continue }
                $target = if (Test-Path -LiteralPath $dest -PathType Container) { [IO.Path]::Combine($dest, $it.Name) } else { $dest }
                if ($f.short.Contains('n') -and (Test-Path -LiteralPath $target)) { continue }
                $ev = $null
                Copy-Item -LiteralPath $it.FullName -Destination $dest -Recurse:$recurse -Force:($f.short.Contains('f')) -ErrorAction SilentlyContinue -ErrorVariable ev
                if ($ev) { foreach ($e in $ev) { Write-Error "cp: $($e.Exception.Message)" }; $status = 1 }
                elseif ($f.short.Contains('v')) { "'$s' -> '$target'" }
            }
        }
        __vx_status $status $MyInvocation
    }
}
function global:copy {
    begin { $sp = $null; if (-not (__vx_cmdswitch $args)) { $vxa = $args; $sp = { & 'Microsoft.PowerShell.Management\Copy-Item' @vxa }.GetSteppablePipeline($MyInvocation.CommandOrigin); $sp.Begin($MyInvocation.ExpectingInput) } }
    process { if ($sp -and $MyInvocation.ExpectingInput) { $sp.Process($_) } }
    end { if ($sp) { $sp.End(); __vx_export_vars $args } else { & $env:ComSpec /d /c copy @args } }
}

function global:mv {
    begin { $sp = $null; if (-not (__vx_linuxy Move-Item $args)) { $vxa = $args; $sp = { & 'Microsoft.PowerShell.Management\Move-Item' @vxa }.GetSteppablePipeline($MyInvocation.CommandOrigin); $sp.Begin($MyInvocation.ExpectingInput) } }
    process { if ($sp -and $MyInvocation.ExpectingInput) { $sp.Process($_) } }
    end {
        if ($sp) { $sp.End(); __vx_export_vars $args; return }
        $f = __vx_flags $args
        if ($f.rest.Count -lt 2) { Write-Error 'usage: mv [-fvni] SOURCE... DEST'; __vx_status 1 $MyInvocation; return }
        $dest = [string]$f.rest[-1]; $status = 0
        foreach ($s in $f.rest[0..($f.rest.Count - 2)]) {
            $items = @(__vx_items ([string]$s))
            if (-not $items.Count) { Write-Error "mv: cannot stat '$s': No such file or directory"; $status = 1; continue }
            foreach ($it in $items) {
                $target = if (Test-Path -LiteralPath $dest -PathType Container) { [IO.Path]::Combine($dest, $it.Name) } else { $dest }
                if (Test-Path -LiteralPath $target) {
                    if ($f.short.Contains('n')) { continue }
                    if ($f.short.Contains('i') -and (Read-Host "mv: overwrite '$target'? [y/N]") -notmatch '^[yY]') { continue }
                }
                # GNU mv replaces an existing file unless -n / -i says otherwise
                $ev = $null
                Move-Item -LiteralPath $it.FullName -Destination $dest -Force -ErrorAction SilentlyContinue -ErrorVariable ev
                if ($ev) { foreach ($e in $ev) { Write-Error "mv: $($e.Exception.Message)" }; $status = 1 }
                elseif ($f.short.Contains('v')) { "renamed '$s' -> '$target'" }
            }
        }
        __vx_status $status $MyInvocation
    }
}
function global:move {
    begin { $sp = $null; if (-not (__vx_cmdswitch $args)) { $vxa = $args; $sp = { & 'Microsoft.PowerShell.Management\Move-Item' @vxa }.GetSteppablePipeline($MyInvocation.CommandOrigin); $sp.Begin($MyInvocation.ExpectingInput) } }
    process { if ($sp -and $MyInvocation.ExpectingInput) { $sp.Process($_) } }
    end { if ($sp) { $sp.End(); __vx_export_vars $args } else { & $env:ComSpec /d /c move @args } }
}

function global:mkdir {
    # mkdir a b c (several plain names) makes each, like Linux
    if ($args.Count -gt 1 -and -not @($args | Where-Object { $_ -isnot [string] -or $_ -match '^-' }).Count) {
        foreach ($p in $args) { New-Item -ItemType Directory -Path $p }
        return
    }
    if (-not (__vx_linuxy New-Item $args)) { New-Item -ItemType Directory @args; return }
    $f = __vx_flags $args @('m')
    $parents = $f.short.Contains('p') -or $f.long -contains 'parents'
    $status = 0
    foreach ($p in $f.rest) {
        if (Test-Path -LiteralPath $p) {
            if (-not $parents) { Write-Error "mkdir: cannot create directory '$p': File exists"; $status = 1 }
            continue
        }
        $ev = $null
        New-Item -ItemType Directory -Path $p -Force:$parents -ErrorAction SilentlyContinue -ErrorVariable ev | Out-Null
        if ($ev) { Write-Error "mkdir: cannot create directory '$p': $($ev[0].Exception.Message)"; $status = 1 }
        elseif ($f.short.Contains('v')) { "mkdir: created directory '$p'" }
    }
    __vx_status $status $MyInvocation
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
        try { if ($MyInvocation.ExpectingInput) { __vx_pipe $input $gnu @() $argv $true } else { $m = __vx_msys 'find' $argv; & $gnu @m; __vx_native_done $MyInvocation } }
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
        $p = @{ LiteralPath = $r; Recurse = $true; Force = $true; ErrorAction = 'SilentlyContinue' }
        if ($null -ne $depth) { $p.Depth = [Math]::Max(0, $depth - 1) }
        if ($type -eq 'f') { $p.File = $true } elseif ($type -eq 'd') { $p.Directory = $true }
        $base = (Resolve-Path -LiteralPath $r).ProviderPath.TrimEnd('\')
        Get-ChildItem @p | Where-Object { -not $name -or ($(if ($ci) { $_.Name -like $name } else { $_.Name -clike $name })) } |
            ForEach-Object { ($r.TrimEnd('\', '/') + $_.FullName.Substring($base.Length)).Replace('\', '/') }
    }
}

function global:sort {
    $gnu = __vx_gnu sort
    $files = $args.Count -gt 0 -and @($args | Where-Object { $_ -is [string] -and $_ -notmatch '^-' -and -not (Test-Path -LiteralPath $_) }).Count -eq 0
    if ($gnu -and ((__vx_linuxy Sort-Object $args) -or ($files -and -not $MyInvocation.ExpectingInput))) {
        if ($MyInvocation.ExpectingInput) { __vx_pipe $input $gnu @() $args } else { $m = __vx_msys 'sort' $args; & $gnu @m; __vx_native_done $MyInvocation }
        return
    }
    if ($MyInvocation.ExpectingInput) { $input | Sort-Object @args } else { Sort-Object @args }
}

function global:diff {
    $gnu = __vx_gnu diff
    $paths = @($args | Where-Object { $_ -is [string] -and $_ -notmatch '^-' })
    if ($gnu -and ((__vx_linuxy Compare-Object $args) -or ($paths.Count -eq 2 -and (Test-Path -LiteralPath $paths[0]) -and (Test-Path -LiteralPath $paths[1])))) { $m = __vx_msys 'diff' $args; & $gnu --color=auto @m; __vx_native_done $MyInvocation; return }
    Compare-Object @args
}

function global:tee {
    begin {
        $sp = $null; $buf = $null; $gnu = __vx_gnu tee
        if ($gnu -and (__vx_linuxy Tee-Object $args)) { $buf = [System.Collections.Generic.List[object]]::new() }
        else { $vxa = $args; $sp = { & 'Microsoft.PowerShell.Utility\Tee-Object' @vxa }.GetSteppablePipeline($MyInvocation.CommandOrigin); $sp.Begin($MyInvocation.ExpectingInput) }
    }
    process { if ($MyInvocation.ExpectingInput) { if ($sp) { $sp.Process($_) } else { $buf.Add($_) } } }
    end { if ($sp) { $sp.End(); __vx_export_vars $args } else { __vx_pipe $buf $gnu @() $args } }
}

# where.exe for names; Where-Object in a pipeline (`... | where Name -eq x`)
function global:where {
    begin {
        $sp = $null
        if ($MyInvocation.ExpectingInput -or ($args.Count -and $args[0] -is [scriptblock])) { $vxa = $args; $sp = { & 'Microsoft.PowerShell.Core\Where-Object' @vxa }.GetSteppablePipeline($MyInvocation.CommandOrigin); $sp.Begin($MyInvocation.ExpectingInput) }
    }
    process { if ($sp -and $MyInvocation.ExpectingInput) { $sp.Process($_) } }
    end { if ($sp) { $sp.End(); __vx_export_vars $args } else { & "$env:SystemRoot\System32\where.exe" @args } }
}

function global:which {
    $all = $false; $names = @(); $status = 0
    foreach ($a in $args) { if ($a -eq '-a') { $all = $true } else { $names += [string]$a } }
    foreach ($n in $names) {
        $found = @(Get-Command $n -All:$all -ErrorAction SilentlyContinue)
        if (-not $all) { $found = @($found | Select-Object -First 1) }
        $g = __vx_gnu $n
        if (-not $found.Count -and -not $g) { Write-Error "which: no $n in PATH"; $status = 1; continue }
        foreach ($c in $found) {
            switch ($c.CommandType) {
                'Application' { $c.Source }
                'ExternalScript' { $c.Source }
                'Alias' { "${n}: aliased to $($c.Definition)" }
                'Function' { if ($c.ScriptBlock.File -like '*vibex-commands.ps1' -or $c.Definition -clike '*__vx_*') { "${n}: Linux-style command (Vibex)" } else { "${n}: PowerShell function" } }
                'Cmdlet' { "${n}: PowerShell cmdlet ($($c.Source))" }
                default { "${n}: $($c.CommandType)" }
            }
        }
        if ($g -and ($all -or -not $found.Count)) { $g }
    }
    __vx_status $status $MyInvocation
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
# pwd prints the path like Linux's, and still has PowerShell's PathInfo
# properties: (pwd).Path, (pwd).ProviderPath, (pwd).Drive keep working
function global:pwd {
    $l = $executionContext.SessionState.Path.CurrentLocation
    $s = [psobject]$l.Path
    foreach ($p in 'Path', 'ProviderPath', 'Drive', 'Provider') { $s.PSObject.Properties.Add([psnoteproperty]::new($p, $l.$p)) }
    $s
}

function global:open { foreach ($t in $args) { if ([string]$t -match '^[a-z][a-z0-9+.-]*://') { Start-Process ([string]$t) } else { Invoke-Item -Path $t } }; if (-not $args) { Invoke-Item . } }
function global:xdg-open { open @args }

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
    begin {
        $sp = $null
        $style = $args.Count -gt 0 -and [string]$args[0] -match '^-?(aux|axu|ax|au|ef|e|A|elf|eF)$'
        if (-not $style -and -not (__vx_linuxy Get-Process $args)) { $vxa = $args; $sp = { & 'Microsoft.PowerShell.Management\Get-Process' @vxa }.GetSteppablePipeline($MyInvocation.CommandOrigin); $sp.Begin($MyInvocation.ExpectingInput) }
    }
    process { if ($sp -and $MyInvocation.ExpectingInput) { $sp.Process($_) } }
    end {
        if ($sp) { $sp.End(); __vx_export_vars $args; return }
        $cpu = @{}; $mem = @{}
        foreach ($p in Get-Process) { $cpu[$p.Id] = $p.CPU; $mem[$p.Id] = $p.WorkingSet64 }
        '{0,7} {1,7} {2,9} {3,8}  {4}' -f 'PID', 'PPID', 'TIME(s)', 'MEM', 'COMMAND'
        Get-CimInstance Win32_Process | Sort-Object ProcessId | ForEach-Object {
            $line = if ($_.CommandLine) { $_.CommandLine } else { $_.Name }
            '{0,7} {1,7} {2,9} {3,8}  {4}' -f $_.ProcessId, $_.ParentProcessId, ('{0:0.0}' -f $cpu[[int]$_.ProcessId]), (__vx_human $mem[[int]$_.ProcessId]), $line
        }
    }
}

# kill [-9|-KILL|-s SIG] PID... ; kill -Name x, kill -Id 5 -WhatIf and
# `Get-Process x | kill` stay Stop-Process. Signals are upper case (or
# numbers), so PowerShell's -WhatIf / -Confirm / -Id are never mistaken
# for one.
function global:kill {
    begin {
        $sp = $null; $ids = @(); $other = $false; $list = $false
        $psNames = __vx_params Stop-Process
        for ($i = 0; $i -lt $args.Count; $i++) {
            $s = [string]$args[$i]
            if ($s -ceq '-l' -or $s -ceq '-L') { $list = $true; continue }
            if ($s -ceq '-s' -or $s -ceq '-n') { $i++; continue }
            if ($s -cmatch '^-(\d+|[A-Z][A-Z0-9]*)$' -and $psNames -notcontains $s.Substring(1)) { continue }
            if ($s -match '^\d+$') { $ids += [int]$s } else { $other = $true }
        }
        if ($other) { $vxa = $args; $sp = { & 'Microsoft.PowerShell.Management\Stop-Process' @vxa }.GetSteppablePipeline($MyInvocation.CommandOrigin); $sp.Begin($MyInvocation.ExpectingInput) }
        elseif ($MyInvocation.ExpectingInput -and -not $ids) { $sp = { Microsoft.PowerShell.Management\Stop-Process -Force }.GetSteppablePipeline($MyInvocation.CommandOrigin); $sp.Begin($MyInvocation.ExpectingInput) }
    }
    process { if ($sp -and $MyInvocation.ExpectingInput) { $sp.Process($_) } }
    end {
        if ($sp) { $sp.End(); __vx_export_vars $args; return }
        if ($list) { 'HUP INT QUIT KILL TERM STOP CONT  (Windows only ends a process: every signal does the same)'; return }
        if (-not $ids) { Write-Error 'kill: usage: kill [-9] PID...'; __vx_status 2 $MyInvocation; return }
        $status = 0
        foreach ($id in $ids) {
            $p = Get-Process -Id $id -ErrorAction SilentlyContinue
            if (-not $p) { Write-Error "kill: ($id) - No such process"; $status = 1; continue }
            $ev = $null
            Stop-Process -InputObject $p -Force -ErrorAction SilentlyContinue -ErrorVariable ev
            if ($ev) { Write-Error "kill: ($id) - $($ev[0].Exception.Message)"; $status = 1 }
        }
        __vx_status $status $MyInvocation
    }
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
function global:pgrep { $f = __vx_flags $args; $p = @(__vx_procs $args); foreach ($x in $p) { if ($f.short.Contains('l')) { "$($x.Id) $($x.ProcessName)" } else { $x.Id } }; __vx_status ([int](-not $p.Count)) $MyInvocation }
function global:pkill { $p = @(__vx_procs $args); if (-not $p.Count) { __vx_status 1 $MyInvocation; return }; $p | Stop-Process -Force -ErrorAction Continue }
function global:killall { $p = @(__vx_procs $args $true); if (-not $p.Count) { Write-Error "killall: no process found"; __vx_status 1 $MyInvocation; return }; $p | Stop-Process -Force -ErrorAction Continue }

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
function global:htop { top @args }

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
    if (-not $f.short) { return 'Windows_NT' }
    $parts = [ordered]@{ s = 'Windows_NT'; n = $env:COMPUTERNAME; r = "$($v.Major).$($v.Minor).$($v.Build)"; v = (Get-CimInstance Win32_OperatingSystem).Caption; m = $arch; o = 'Windows' }
    if ($f.short.Contains('a')) { return ($parts.Values -join ' ') }
    ($parts.Keys | Where-Object { $f.short.Contains($_) } | ForEach-Object { $parts[$_] }) -join ' '
}

function global:ifconfig { if ($args -contains '-a') { ipconfig /all } else { ipconfig @args } }

# ip a / ip r / ip link
function global:ip {
    $what = if ($args.Count) { [string]$args[0] } else { 'addr' }
    switch -Regex ($what) {
        '^(a|addr|address)$' {
            foreach ($ad in Get-NetAdapter -IncludeHidden:$false -ErrorAction SilentlyContinue | Sort-Object ifIndex) {
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
    # -i:3000 and -iTCP:3000 (attached) as well as -i :3000
    $spec = $null; $hasI = $false
    for ($i = 0; $i -lt $args.Count; $i++) {
        $s = [string]$args[$i]
        if ($s -cmatch '^-[a-zA-Z]*i$') { $hasI = $true; if ($i + 1 -lt $args.Count -and [string]$args[$i + 1] -notmatch '^-') { $i++; $spec = [string]$args[$i] } }
        # -i:3000 reaches here as '-i:' and 3000 (PowerShell's -param:value)
        elseif ($s -cmatch '^-[a-zA-Z]*i(.*):$' -and $i + 1 -lt $args.Count) { $hasI = $true; $spec = $Matches[1] + ':' + [string]$args[$i + 1]; $i++ }
        elseif ($s -cmatch '^-[a-zA-Z]*i(.+)$') { $hasI = $true; $spec = $Matches[1] }
    }
    if (-not $hasI) { Write-Error 'lsof: Vibex does network sockets: lsof -i [:PORT], lsof -i tcp:3000'; return }
    $spec = [string]$spec
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
    if (-not $args.Count -or ($args.Count -eq 1 -and $args[0] -ceq '-p')) { Get-ChildItem env: | Sort-Object Name | ForEach-Object { "declare -x $($_.Name)=`"$($_.Value)`"" }; return }
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

# set (cmd: list / NAME=value) or Set-Variable, in the caller's scope like
# the real `set` alias (a script's `set x 1` stays in the script)
function global:set {
    if (-not $args.Count) { printenv; return }
    if ($args.Count -eq 1 -and [string]$args[0] -match '^([A-Za-z_][\w()]*)=(.*)$') { Set-Item "env:$($Matches[1])" $Matches[2]; return }
    if (@($args | Where-Object { $_ -is [string] -and $_ -match '^-s(c(o(p(e)?)?)?)?:?$' }).Count) { Set-Variable @args }
    else { Set-Variable @args -Scope 1 }
}

# alias ll='ls -la' (bash) / alias (list). The arguments follow the text,
# as with bash: `ll 'my dir'` is `ls -la 'my dir'`, quoting kept.
function global:alias {
    if (-not $args.Count) { Get-Alias | Sort-Object Name | ForEach-Object { "alias $($_.Name)='$($_.Definition)'" }; return }
    foreach ($a in $args) {
        $s = [string]$a
        if ($s -match '^([\w.-]+)=(.*)$') {
            $n, $v = $Matches[1], $Matches[2].Trim("'", '"')
            if (Get-Alias -Name $n -ErrorAction SilentlyContinue) { Remove-Item "Alias:$n" -Force }
            $sb = try { [scriptblock]::Create("$v @args") } catch { [scriptblock]::Create("Invoke-Expression ('$($v.Replace("'", "''"))' + ' ' + (`$args -join ' '))") }
            Set-Item "function:global:$n" -Value $sb
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
    if ($f.value.ContainsKey('O')) { if ($f.value['O'] -ne '-') { $c += @('-o', $f.value['O']) } } else { $c += '-O' }
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
        $exe = [IO.Path]::Combine($global:__VibexGnu, "$n.exe")
        if ([IO.File]::Exists($exe)) {
            $e.Command = $ExecutionContext.InvokeCommand.GetCommand($exe, 'Application')
            $e.StopSearch = $true
            return
        }
    }
    if ($global:__VibexPrevNotFound) { & $global:__VibexPrevNotFound $name $e }
}

# ------------------------------------------------------------ the typed line
# Linux one-liners pasted into PowerShell 5.1: a && b, a || b, 2>/dev/null,
# &>/dev/null, $PATH. Uses PowerShell's own tokenizer, so && inside quotes,
# here-strings, comments and script blocks is left alone, and a line with
# none of these comes back exactly as typed. (PowerShell 7 has && and ||
# itself; /dev/null and $PATH are rewritten in both.)
$global:__VibexRewrite = {
    param([string]$line)
    if (-not $line -or ($line.IndexOf('&&') -lt 0 -and $line.IndexOf('||') -lt 0 -and $line.IndexOf('/dev/null') -lt 0 -and $line.IndexOf('$PATH', [StringComparison]::Ordinal) -lt 0)) { return $line }
    Set-StrictMode -Off
    $tokens = $null; $errs = $null
    [void][System.Management.Automation.Language.Parser]::ParseInput($line, [ref]$tokens, [ref]$errs)
    $K = [System.Management.Automation.Language.TokenKind]
    # edits: start, end, replacement; a replacement of $null splits the line there
    $edits = New-Object 'System.Collections.Generic.List[object]'
    $pathVar = -not (Get-Variable -Name PATH -ErrorAction SilentlyContinue)
    $split = $PSVersionTable.PSVersion.Major -lt 7
    $depth = 0
    $flat = New-Object 'System.Collections.Generic.List[object]'
    foreach ($t in $tokens) {
        $flat.Add($t)
        # variables inside "strings" and bare arguments (PATH=$PATH:/x)
        if ($t -is [System.Management.Automation.Language.StringExpandableToken] -and $t.NestedTokens) { foreach ($n in $t.NestedTokens) { if ($n.Kind -eq $K::Variable) { $flat.Add($n) } } }
    }
    for ($i = 0; $i -lt $tokens.Count; $i++) {
        $t = $tokens[$i]
        if ($t.Kind -in $K::LParen, $K::LCurly, $K::AtParen, $K::AtCurly, $K::DollarParen, $K::LBracket) { $depth++; continue }
        if ($t.Kind -in $K::RParen, $K::RCurly, $K::RBracket) { $depth--; continue }
        if ($split -and $depth -eq 0 -and ($t.Kind -eq $K::AndAnd -or $t.Kind -eq $K::OrOr)) { $edits.Add(@($t.Extent.StartOffset, $t.Extent.EndOffset, $null, $t.Text)); continue }
        if ($t.Kind -eq $K::Redirection -and $i + 1 -lt $tokens.Count -and $tokens[$i + 1].Text -eq '/dev/null' -and $tokens[$i + 1].Kind -eq $K::Generic) {
            $n = $tokens[$i + 1]
            $start = $t.Extent.StartOffset; $op = $t.Text
            # &>/dev/null: bash's "both streams" is PowerShell's *>
            if ($i -gt 0 -and $tokens[$i - 1].Kind -eq $K::Ampersand -and $tokens[$i - 1].Extent.EndOffset -eq $start -and $op -eq '>') { $start = $tokens[$i - 1].Extent.StartOffset; $op = '*>' }
            $edits.Add(@($start, $t.Extent.EndOffset, $op, $null))
            $edits.Add(@($n.Extent.StartOffset, $n.Extent.EndOffset, '$null', $null))
            $i++
        }
    }
    # bash's $PATH (PowerShell has none: export PATH=$PATH:/x), unless a
    # PowerShell variable of that name exists or it's being assigned to
    if ($pathVar) {
        for ($i = 0; $i -lt $flat.Count; $i++) {
            $t = $flat[$i]
            # ($PATH:/x tokenizes as the drive-qualified variable $PATH:)
            if ($t.Kind -ne $K::Variable -or $t.Text -cnotmatch '^\$PATH:?$') { continue }
            $next = $null
            foreach ($u in $tokens) { if ($u.Extent.StartOffset -ge $t.Extent.EndOffset -and $u.Kind -ne $K::NewLine) { $next = $u; break } }
            if ($next -and ($next.TokenFlags -band [System.Management.Automation.Language.TokenFlags]::AssignmentOperator)) { continue }
            $edits.Add(@($t.Extent.StartOffset, ($t.Extent.StartOffset + 5), '${env:PATH}', $null))
        }
    }
    if (-not $edits.Count) { return $line }
    $sorted = @($edits | Sort-Object { $_[0] })
    $parts = New-Object 'System.Collections.Generic.List[string]'
    $ops = New-Object 'System.Collections.Generic.List[string]'
    $sb = [System.Text.StringBuilder]::new(); $pos = 0
    foreach ($e in $sorted) {
        [void]$sb.Append($line.Substring($pos, $e[0] - $pos))
        if ($null -eq $e[2]) { $parts.Add($sb.ToString().Trim()); $ops.Add($e[3]); [void]$sb.Clear() }
        else { [void]$sb.Append($e[2]) }
        $pos = $e[1]
    }
    [void]$sb.Append($line.Substring($pos))
    if ($ops.Count -eq 0) { return $sb.ToString() }
    $parts.Add($sb.ToString().Trim())
    # bash's meaning, left to right: a && b || c runs c when a or b failed
    $out = "$($parts[0]); `$global:__vxok = `$?"
    for ($k = 0; $k -lt $ops.Count; $k++) {
        $test = if ($ops[$k] -eq '&&') { '$global:__vxok' } else { '-not $global:__vxok' }
        $out += "; if ($test) { $($parts[$k + 1]); `$global:__vxok = `$? }"
    }
    $out
}
