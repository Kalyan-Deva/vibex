# Download, check and unpack the editors Vibex ships (tools\bundle.lock)
# into vendor\tools:
#
#   vendor\tools\nvim\bin\nvim.exe ...          Neovim (official build)
#   vendor\tools\nano\usr\bin\nano.exe + DLLs   GNU nano (MSYS2), its runtime,
#   vendor\tools\nano\usr\share\nano ...        syntax files, terminfo, magic
#   vendor\tools\LICENSES\...                   every licence
#   vendor\tools\versions.txt                   "nvim 0.12.6" / "nano 9.3-1"
#
# Downloads are cached in vendor\cache and must match the SHA-256 in the
# lock file, or nothing is unpacked.
#
#   powershell -ExecutionPolicy Bypass -File tools\fetch-bundle.ps1
#
# Vibex runs the copy inside the bundle (tools\update.ps1) to update the
# installed editors: -Lock a new lock file, -Out a fresh folder, -Cache temp.
param([switch]$Force, [string]$Lock = '', [string]$Out = '', [string]$Cache = '')

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$lockFile = if ($Lock) { $Lock } else { Join-Path $PSScriptRoot 'bundle.lock' }
$cache = if ($Cache) { $Cache } else { Join-Path $root 'vendor\cache' }
$out = if ($Out) { $Out } else { Join-Path $root 'vendor\tools' }
$tar = Join-Path $env:SystemRoot 'System32\tar.exe'   # bsdtar: zip and .zst
New-Item -ItemType Directory -Force $cache | Out-Null
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$items = Get-Content $lockFile | Where-Object { $_ -and $_ -notmatch '^\s*#' } | ForEach-Object {
    $f = $_ -split '\s+'
    [pscustomobject]@{ Name = $f[0]; Version = $f[1]; Sha = $f[2].ToLower(); Url = $f[3]; File = Join-Path $cache (Split-Path $f[3] -Leaf) }
}

foreach ($i in $items) {
    $ok = (Test-Path $i.File) -and ((Get-FileHash $i.File -Algorithm SHA256).Hash.ToLower() -eq $i.Sha)
    if (-not $ok -or $Force) {
        Write-Output "downloading $($i.Name) $($i.Version)"
        $tmp = "$($i.File).part"
        Invoke-WebRequest -Uri $i.Url -OutFile $tmp -UseBasicParsing -UserAgent 'vibex-build'
        $got = (Get-FileHash $tmp -Algorithm SHA256).Hash.ToLower()
        if ($got -ne $i.Sha) { Remove-Item $tmp; throw "$($i.Name): SHA-256 is $got, the lock file says $($i.Sha)" }
        Move-Item $tmp $i.File -Force
    }
}
Write-Output "all $($items.Count) downloads match bundle.lock"

if (Test-Path $out) { Remove-Item -Recurse -Force $out }
New-Item -ItemType Directory -Force $out, (Join-Path $out 'LICENSES') | Out-Null

# Neovim: the zip holds nvim-win64\{bin,lib,share}
$nv = $items | Where-Object Name -eq 'nvim'
$stage = Join-Path $cache 'stage-nvim'
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory $stage | Out-Null
& $tar -xf $nv.File -C $stage
if ($LASTEXITCODE) { throw 'unpacking Neovim failed' }
$top = Get-ChildItem $stage -Directory | Select-Object -First 1
Move-Item $top.FullName (Join-Path $out 'nvim')
$nvLicense = Get-ChildItem (Join-Path $out 'nvim') -Recurse -File -Include 'LICENSE*', 'license*' -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $nvLicense) {
    # the Windows zip has no licence file of its own: Apache-2.0 + Vim licence
    Invoke-WebRequest -Uri "https://raw.githubusercontent.com/neovim/neovim/v$($nv.Version)/LICENSE.txt" -OutFile (Join-Path $out 'LICENSES\neovim.txt') -UseBasicParsing -UserAgent 'vibex-build'
} else { Copy-Item $nvLicense.FullName (Join-Path $out 'LICENSES\neovim.txt') }

# nano + runtime: every MSYS2 package into one tree, then keep what nano uses
$stage = Join-Path $cache 'stage-msys'
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory $stage | Out-Null
foreach ($i in $items | Where-Object Name -ne 'nvim') {
    & $tar -xf $i.File -C $stage 2>$null
    if ($LASTEXITCODE) { throw "unpacking $($i.Name) failed" }
}
$nano = Join-Path $out 'nano'
$bin = Join-Path $nano 'usr\bin'
New-Item -ItemType Directory -Force $bin, (Join-Path $nano 'etc'), (Join-Path $nano 'usr\share\misc'), (Join-Path $nano 'tmp') | Out-Null
Copy-Item (Join-Path $stage 'usr\bin\nano.exe') $bin
# only the DLLs nano loads (not the C++ / OpenMP runtimes that come along)
foreach ($dll in 'msys-2.0', 'msys-ncursesw6', 'msys-intl-8', 'msys-iconv-2', 'msys-charset-1', 'msys-magic-1', 'msys-gcc_s-seh-1', 'msys-z', 'msys-bz2-1', 'msys-lzma-5', 'msys-zstd-1') {
    Copy-Item (Join-Path $stage "usr\bin\$dll.dll") $bin
}
Copy-Item -Recurse (Join-Path $stage 'usr\share\nano') (Join-Path $nano 'usr\share\nano')
Copy-Item (Join-Path $stage 'usr\share\misc\magic.mgc') (Join-Path $nano 'usr\share\misc')
# HOME is the Windows profile folder (nano keeps its history there)
[IO.File]::WriteAllText((Join-Path $nano 'etc\nsswitch.conf'), "db_home: windows`n")
# nanorc: colours for every language nano knows, mouse, line numbers off
$rc = Get-Content (Join-Path $stage 'etc\nanorc') -Raw
$rc += "`n## Vibex`ninclude `"/usr/share/nano/*.nanorc`"`ninclude `"/usr/share/nano/extra/*.nanorc`"`nset mouse`nset indicator`n"
[IO.File]::WriteAllText((Join-Path $nano 'etc\nanorc'), $rc.Replace("`r`n", "`n"))
# terminfo for the terminals nano will meet (ncurses looks in /usr/share/terminfo and /usr/lib/terminfo)
foreach ($t in 'cygwin', 'xterm', 'xterm-256color', 'ms-terminal', 'vt100', 'dumb') {
    foreach ($base in 'usr\share\terminfo', 'usr\lib\terminfo') {
        $src = Get-ChildItem (Join-Path $stage $base) -Recurse -File -Filter $t -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($src) {
            $rel = $src.FullName.Substring($stage.Length + 1)
            $dst = Join-Path $nano ($rel -replace '^usr\\lib\\terminfo', 'usr\share\terminfo')
            New-Item -ItemType Directory -Force (Split-Path $dst) | Out-Null
            Copy-Item $src.FullName $dst -Force
            break
        }
    }
}
# licences of everything shipped from MSYS2
foreach ($d in Get-ChildItem (Join-Path $stage 'usr\share\licenses') -Directory -ErrorAction SilentlyContinue) {
    Copy-Item -Recurse $d.FullName (Join-Path $out "LICENSES\msys2-$($d.Name)")
}
$src = ($items | Where-Object Name -ne 'nvim' | ForEach-Object { "  $($_.Name) $($_.Version): $($_.Url)" }) -join "`n"
[IO.File]::WriteAllText((Join-Path $out 'LICENSES\SOURCES.txt'), @"
Neovim $($nv.Version): https://github.com/neovim/neovim/tree/v$($nv.Version) (Apache-2.0 and the Vim licence)

GNU nano and its runtime come from the MSYS2 project (https://www.msys2.org).
GNU nano is free software under the GNU GPL version 3 or later. The source
for each package shipped here is in the MSYS2 source repository
(https://github.com/msys2/MSYS2-packages) and at
https://mirror.msys2.org/msys/sources/ ; packages:
$src
"@.Replace("`r`n", "`n"))

# what's installed, and how Vibex updates it later
Copy-Item $lockFile (Join-Path $out 'bundle.lock')
Copy-Item $PSCommandPath (Join-Path $out 'update.ps1')
[IO.File]::WriteAllText((Join-Path $out 'versions.txt'), "nvim $($nv.Version)`nnano $(($items | Where-Object Name -eq 'nano').Version)`n")
Remove-Item -Recurse -Force (Join-Path $cache 'stage-msys'), (Join-Path $cache 'stage-nvim')
$mb = [math]::Round(((Get-ChildItem $out -Recurse -File | Measure-Object Length -Sum).Sum) / 1MB, 1)
Write-Output "unpacked into $out ($mb MB)"
