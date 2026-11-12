# Write winget manifests for a published installer.
#
#   powershell -ExecutionPolicy Bypass -File tools\winget.ps1 -Url https://.../Vibex-Setup-0.1.0.exe
#
# Output: dist\winget\manifests\k\KalyanGopalam\Vibex\<version>\*.yaml, ready
# for `winget validate` and a PR to microsoft/winget-pkgs. The installer URL
# must be public (winget downloads it), so publish the release first.
param(
    [Parameter(Mandatory = $true)][string]$Url,
    [string]$Installer = ''
)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
if (-not $Installer) {
    $Installer = Get-ChildItem (Join-Path $root 'dist') -Filter 'Vibex-Setup-*.exe' | Sort-Object LastWriteTime -Descending | Select-Object -First 1 -ExpandProperty FullName
}
if (-not $Installer -or -not (Test-Path $Installer)) { throw "no installer: run tools\package.ps1 first" }
if ($Url -notmatch '^https://') { throw "the installer URL must be https" }

$version = (Get-Item $Installer).VersionInfo.ProductVersion
$sha = (Get-FileHash $Installer -Algorithm SHA256).Hash
$id = 'KalyanGopalam.Vibex'
$dir = Join-Path $root "dist\winget\manifests\k\KalyanGopalam\Vibex\$version"
New-Item -ItemType Directory -Force $dir | Out-Null

@"
# yaml-language-server: `$schema=https://aka.ms/winget-manifest.version.1.6.0.schema.json
PackageIdentifier: $id
PackageVersion: $version
DefaultLocale: en-US
ManifestType: version
ManifestVersion: 1.6.0
"@ | Set-Content -Encoding utf8 (Join-Path $dir "$id.yaml")

@"
# yaml-language-server: `$schema=https://aka.ms/winget-manifest.installer.1.6.0.schema.json
PackageIdentifier: $id
PackageVersion: $version
InstallerType: exe
Scope: user
InstallModes:
  - interactive
  - silent
InstallerSwitches:
  Silent: --quiet
  SilentWithProgress: --quiet
UpgradeBehavior: install
Commands:
  - vibex
Installers:
  - Architecture: x64
    InstallerUrl: $Url
    InstallerSha256: $sha
ManifestType: installer
ManifestVersion: 1.6.0
"@ | Set-Content -Encoding utf8 (Join-Path $dir "$id.installer.yaml")

@"
# yaml-language-server: `$schema=https://aka.ms/winget-manifest.defaultLocale.1.6.0.schema.json
PackageIdentifier: $id
PackageVersion: $version
PackageLocale: en-US
Publisher: Kalyan Gopalam
PackageName: Vibex Terminal
License: Proprietary
ShortDescription: A GPU-accelerated terminal for Windows with Lua configuration, splits, tabs and inline images.
Tags:
  - terminal
  - console
  - shell
ManifestType: defaultLocale
ManifestVersion: 1.6.0
"@ | Set-Content -Encoding utf8 (Join-Path $dir "$id.locale.en-US.yaml")

Write-Output "wrote $dir"
if (Get-Command winget -ErrorAction SilentlyContinue) {
    winget validate --manifest $dir
}
