# Build the release Vibex + its single-file installer into dist\.
#
#   powershell -ExecutionPolicy Bypass -File tools\package.ps1
#       [-SignThumbprint <cert SHA1>]      Authenticode-sign vibex.exe and the setup
#       [-FeedUrl https://.../Vibex-Setup-<v>.exe]
#                                          also write dist\update.txt, the signed
#                                          update feed (key: %USERPROFILE%\.vibex)
#
#       [-TargetDir <dir>]                 cargo target dir (relative = to where
#                                          you run this from)
#
# Output: dist\Vibex-Setup-<version>.exe  (run it to install / update)
param([string]$TargetDir = "", [string]$SignThumbprint = "", [string]$FeedUrl = "", [string]$Notes = "")

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
# -TargetDir is relative to the caller's folder, and has to be absolute for
# cargo and build.rs (which run in other folders)
if ($TargetDir -and -not [IO.Path]::IsPathRooted($TargetDir)) {
    $TargetDir = Join-Path (Get-Location).ProviderPath $TargetDir
}
if ($TargetDir) { $TargetDir = [IO.Path]::GetFullPath($TargetDir) }
Set-Location $root

if ($TargetDir) { $env:CARGO_TARGET_DIR = $TargetDir } else { Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue }
$target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root 'target' }
Write-Output "target dir: $target"

# Keep this machine's paths (C:\Users\<name>\.cargo\registry\... in panic
# messages) out of the shipped binaries. CARGO_ENCODED_RUSTFLAGS (0x1F
# separated) because the paths may hold spaces; later prefixes win.
$sep = [string][char]0x1f
$rustflags = @()
if ($env:CARGO_ENCODED_RUSTFLAGS) { $rustflags += $env:CARGO_ENCODED_RUSTFLAGS -split $sep }
elseif ($env:RUSTFLAGS) { $rustflags += $env:RUSTFLAGS -split '\s+' | Where-Object { $_ } }
$cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
$rustflags += "--remap-path-prefix=$($env:USERPROFILE)=~", "--remap-path-prefix=$cargoHome=cargo", "--remap-path-prefix=$root=vibex"
$env:CARGO_ENCODED_RUSTFLAGS = $rustflags -join $sep

function Assert-File($path, $what) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "$what not found at $path" }
}

cargo build --release -p vibex
if ($LASTEXITCODE) { throw "building vibex failed" }
$env:VIBEX_PAYLOAD = Join-Path $target 'release\vibex.exe'
Assert-File $env:VIBEX_PAYLOAD 'the release vibex.exe'
if ($SignThumbprint) {
    & (Join-Path $PSScriptRoot 'sign.ps1') -Thumbprint $SignThumbprint -Files $env:VIBEX_PAYLOAD
}

# OpenConsole.exe (MIT, microsoft/terminal) lets sixel / kitty images through
# ConPTY. Taken from vendor\OpenConsole.exe, else from Windows Terminal.
$oc = Join-Path $root 'vendor\OpenConsole.exe'
if (-not (Test-Path $oc)) {
    $wt = Get-AppxPackage -Name Microsoft.WindowsTerminal -ErrorAction SilentlyContinue | Sort-Object Version -Descending | Select-Object -First 1
    if ($wt) { $oc = Join-Path $wt.InstallLocation 'OpenConsole.exe' }
}
if (Test-Path $oc) {
    $env:VIBEX_OPENCONSOLE_PAYLOAD = $oc
    Write-Output "bundling $oc"
} else {
    Remove-Item Env:VIBEX_OPENCONSOLE_PAYLOAD -ErrorAction SilentlyContinue
    Write-Warning "no OpenConsole.exe found: inline images won't work in the installed Vibex"
}
# nano + Neovim (tools\bundle.lock): fetched and checked once, then zipped
# into the setup
if (-not (Test-Path (Join-Path $root 'vendor\tools\versions.txt'))) {
    & (Join-Path $PSScriptRoot 'fetch-bundle.ps1')
}
$toolsZip = Join-Path $target 'vibex-tools.zip'
if (Test-Path $toolsZip) { Remove-Item $toolsZip }
& (Join-Path $env:SystemRoot 'System32\tar.exe') -a -cf $toolsZip -C (Join-Path $root 'vendor\tools') .
if ($LASTEXITCODE) { throw "zipping the editors failed" }
$env:VIBEX_TOOLS_PAYLOAD = $toolsZip
Write-Output "bundling nano + Neovim ($([math]::Round((Get-Item $toolsZip).Length / 1MB, 1)) MB)"
# vibex.com: the console `vibex` command (shells wait for CLI output)
cargo build --release -p vibex-launcher
if ($LASTEXITCODE) { throw "building vibex-launcher failed" }
$env:VIBEX_LAUNCHER_PAYLOAD = Join-Path $target 'release\vibex-launcher.exe'
Assert-File $env:VIBEX_LAUNCHER_PAYLOAD 'vibex-launcher.exe'
Assert-File $env:VIBEX_TOOLS_PAYLOAD 'the editors zip'
cargo build --release -p vibex-setup
if ($LASTEXITCODE) { throw "building vibex-setup failed" }

$version = (Get-Item $env:VIBEX_PAYLOAD).VersionInfo.ProductVersion
if ($version -notmatch '^\d+\.\d+\.\d+([-+][0-9A-Za-z.+-]+)?$') { throw "vibex.exe has no usable version (ProductVersion '$version')" }
New-Item -ItemType Directory -Force (Join-Path $root 'dist') | Out-Null
$out = Join-Path $root "dist\Vibex-Setup-$version.exe"
Copy-Item (Join-Path $target 'release\vibex-setup.exe') $out -Force
if ($SignThumbprint) {
    & (Join-Path $PSScriptRoot 'sign.ps1') -Thumbprint $SignThumbprint -Files $out
}
$size = [math]::Round((Get-Item $out).Length / 1MB, 1)
Write-Output "built $out ($size MB)"

if ($FeedUrl) {
    cargo build --release -p vibex-release
    if ($LASTEXITCODE) { throw "building vibex-release failed" }
    $feed = Join-Path $root 'dist\update.txt'
    # Windows PowerShell drops an empty argument, so only pass --notes with text
    $feedArgs = @('feed', '--version', $version, '--url', $FeedUrl, '--installer', $out, '--out', $feed)
    if ($Notes) { $feedArgs += @('--notes', $Notes) }
    & (Join-Path $target 'release\vibex-release.exe') @feedArgs
    if ($LASTEXITCODE) { throw "signing the update feed failed" }
    Write-Output "publish $out at $FeedUrl, and $feed where config.update_feed points"
}
